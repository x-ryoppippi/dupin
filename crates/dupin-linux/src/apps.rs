//! Installed apps: the desktop entries in the XDG data directories' `applications` folders,
//! fuzzy-matched by their localized name, their untranslated name, their generic name and
//! their keywords.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dupin_core::fuzzy;
use dupin_core::{Item, Provider, ProviderError, Request, View};

use crate::desktop_entry::{self, Locale};

pub type AppsWatcher = crate::watcher::PathWatcher;

/// How deep to look for entries below a root: vendors put theirs in subfolders
/// (`applications/kde4/foo.desktop`, whose ID is `kde4-foo.desktop`).
const MAX_DEPTH: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
struct App {
    /// The localized `Name`.
    name: String,
    /// Other names that should find the app: the untranslated name, the generic name and the
    /// keywords.
    aliases: Vec<String>,
    /// What the row shows under the name: the generic name, or the entry's folder.
    subtitle: Option<String>,
    /// The `.desktop` file: opening it launches the app, and Rune draws its `Icon`.
    path: PathBuf,
}

/// A scan: the revision and the roots' resolved paths it was made at, and the apps it found.
type Index = (u64, Vec<PathBuf>, Vec<App>);

pub struct AppsProvider {
    roots: Vec<PathBuf>,
    locale: Locale,
    desktops: Vec<String>,
    /// The scan, with the revision and the roots' resolved paths it was made at: a NixOS
    /// profile switch repoints a root's symlink without touching the folders a watch is on.
    index: Mutex<Option<Index>>,
    revision: AtomicU64,
}

impl AppsProvider {
    /// `$XDG_DATA_HOME/applications`, then each `$XDG_DATA_DIRS` entry's `applications`, with
    /// the specification's defaults (`~/.local/share`, `/usr/local/share:/usr/share`).
    pub fn standard() -> Self {
        let mut roots = Vec::new();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
            Some(data_home) => roots.push(PathBuf::from(data_home)),
            None => roots.extend(home.map(|home| home.join(".local/share"))),
        }
        match std::env::var_os("XDG_DATA_DIRS").filter(|v| !v.is_empty()) {
            Some(dirs) => roots.extend(std::env::split_paths(&dirs)),
            None => roots.extend(["/usr/local/share", "/usr/share"].map(PathBuf::from)),
        }
        let mut seen = HashSet::new();
        roots.retain(|root| root.is_absolute() && seen.insert(root.clone()));
        let roots = roots.into_iter().map(|root| root.join("applications")).collect();
        Self::with(roots, Locale::from_env(), desktop_entry::current_desktops())
    }

    fn with(roots: Vec<PathBuf>, locale: Locale, desktops: Vec<String>) -> Self {
        AppsProvider {
            roots,
            locale,
            desktops,
            index: Mutex::new(None),
            revision: AtomicU64::new(0),
        }
    }

    /// Watch the application folders and invalidate the cached list when an entry changes.
    /// Writes are debounced before the cached list is rebuilt.
    pub fn watch(
        self: &Arc<Self>,
        changed: impl Fn() + Send + Sync + 'static,
    ) -> Result<AppsWatcher, &'static str> {
        let roots = self.roots.clone();
        let provider = Arc::downgrade(self);
        AppsWatcher::start(roots, move || {
            if let Some(provider) = provider.upgrade() {
                provider.invalidate();
                changed();
            }
        })
    }

    fn invalidate(&self) {
        self.revision.fetch_add(1, Ordering::Release);
    }

    /// Scan before the first search so it doesn't wait for the disk.
    pub fn warm(&self) {
        drop(self.apps());
    }

    fn apps(&self) -> Vec<App> {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let revision = self.revision.load(Ordering::Acquire);
        let resolved = resolved(&self.roots);
        if let Some((cached_revision, cached_roots, apps)) = &*index
            && *cached_revision == revision
            && *cached_roots == resolved
        {
            return apps.clone();
        }
        let apps = scan(&self.roots, &self.locale, &self.desktops);
        *index = Some((revision, resolved, apps.clone()));
        apps
    }
}

impl Provider for AppsProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        emit(rank(&self.apps(), &request.query, request.limit));
        Ok(())
    }
}

/// Each root with its symlinks resolved, or as it is when it doesn't exist.
fn resolved(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .map(|root| fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
        .collect()
}

/// A match on an alias ranks below the same match on the displayed name.
const ALIAS_PENALTY: i32 = 8;

/// Best matches first; equal scores keep the shorter, then alphabetical, name. An app matched
/// through an alias shows the alias in its subtitle, since its title doesn't contain the query.
fn rank(apps: &[App], query: &str, limit: usize) -> Vec<Item> {
    let mut hits: Vec<(i32, Option<&str>, Vec<usize>, &App)> = apps
        .iter()
        .filter_map(|app| {
            let on_name = fuzzy::score(query, &app.name).map(|m| (m.score, None, m.positions));
            let on_alias = app
                .aliases
                .iter()
                .filter_map(|a| {
                    fuzzy::score(query, a).map(|m| (m.score - ALIAS_PENALTY, Some(a.as_str()), Vec::new()))
                })
                .max_by_key(|(score, ..)| *score);
            let (score, alias, positions) = match (on_name, on_alias) {
                (Some(n), Some(a)) => {
                    if a.0 > n.0 {
                        a
                    } else {
                        n
                    }
                }
                (n, a) => n.or(a)?,
            };
            Some((score, alias, positions, app))
        })
        .collect();
    hits.sort_by(|(a, _, _, x), (b, _, _, y)| b.cmp(a).then_with(|| x.name.cmp(&y.name)));
    hits.truncate(limit);
    hits.into_iter()
        .map(|(score, alias, positions, app)| {
            let mut item = Item::path(app.path.clone(), app.name.clone(), positions);
            if let View::Text { subtitle, .. } = &mut item.view {
                if let Some(own) = &app.subtitle {
                    subtitle.clone_from(own);
                }
                if let Some(alias) = alias {
                    *subtitle = format!("{alias} · {subtitle}");
                }
            }
            Item {
                score: Some(score),
                completion: None,
                ..item
            }
        })
        .collect()
}

/// Every application entry under `roots`, one per desktop-file ID: the first root wins, so a
/// user's own entry hides the system's entry of the same ID, and one that hides it (`Hidden`,
/// `NoDisplay`, another desktop's) hides it from the list too. Entries with the same name
/// after that keep the first.
fn scan(roots: &[PathBuf], locale: &Locale, desktops: &[String]) -> Vec<App> {
    let mut files = Vec::new();
    for root in roots {
        walk(root, root, 0, &mut files);
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    let mut apps = Vec::new();
    for (id, path) in files {
        if !ids.insert(id) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Some(entry) = desktop_entry::parse(&text, locale, desktops) else {
            continue;
        };
        if entry.exec.is_none() || !names.insert(entry.name.clone()) {
            continue;
        }
        let mut aliases: Vec<String> = Vec::new();
        for alias in entry
            .default_name
            .iter()
            .chain(&entry.generic_name)
            .chain(&entry.keywords)
        {
            let same = |n: &String| n.to_lowercase() == alias.to_lowercase();
            if !same(&entry.name) && !aliases.iter().any(same) {
                aliases.push(alias.clone());
            }
        }
        apps.push(App {
            name: entry.name,
            aliases,
            subtitle: entry.generic_name,
            path,
        });
    }
    apps
}

/// The `.desktop` files below `dir` with their desktop-file IDs (the path below `root`, `/`
/// as `-`), sorted by path within each folder.
fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "desktop") && path.is_file() {
            if let Ok(relative) = path.strip_prefix(root) {
                let id = relative.to_string_lossy().replace('/', "-");
                out.push((id, path));
            }
        } else if depth + 1 < MAX_DEPTH && path.is_dir() {
            walk(root, &path, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static NEXT_TREE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn tree() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dupin-linux-apps-{}-{}",
            std::process::id(),
            NEXT_TREE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(root: &Path, relative: &str, name: &str, extra: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec=true\n{extra}"),
        )
        .unwrap();
    }

    fn names(apps: &[App]) -> Vec<&str> {
        apps.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn the_first_root_wins_by_desktop_file_id() {
        let user = tree();
        let system = tree();
        write(&user, "firefox.desktop", "My Firefox", "");
        write(&system, "firefox.desktop", "Firefox", "");
        write(&system, "kde4/okular.desktop", "Okular", "");
        write(&user, "hidden.desktop", "Hidden", "NoDisplay=true\n");
        write(&system, "hidden.desktop", "Shown", "");
        let apps = scan(&[user.clone(), system.clone()], &Locale::default(), &[]);
        assert_eq!(names(&apps), ["My Firefox", "Okular"]);
        fs::remove_dir_all(user).unwrap();
        fs::remove_dir_all(system).unwrap();
    }

    #[test]
    fn keywords_find_the_app_and_show_in_the_subtitle() {
        let root = tree();
        write(
            &root,
            "org.gnome.Nautilus.desktop",
            "Files",
            "GenericName=File Manager\nKeywords=folder;explorer;\n",
        );
        let apps = scan(std::slice::from_ref(&root), &Locale::default(), &[]);
        let items = rank(&apps, "explorer", 10);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].display_title(), "Files");
        let View::Text { subtitle, .. } = &items[0].view else {
            panic!("expected text")
        };
        assert_eq!(subtitle, "explorer · File Manager");
        assert_eq!(
            items[0].action("open"),
            Some(&dupin_core::Action::Open(root.join("org.gnome.Nautilus.desktop")))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalidating_the_index_picks_up_new_entries() {
        let root = tree();
        let provider = AppsProvider::with(vec![root.clone()], Locale::default(), Vec::new());
        assert!(provider.apps().is_empty());
        write(&root, "new.desktop", "New App", "");
        provider.invalidate();
        assert_eq!(names(&provider.apps()), ["New App"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_repointed_root_is_scanned_again() {
        let base = tree();
        let (one, two) = (base.join("one"), base.join("two"));
        write(&one, "a.desktop", "Generation One", "");
        write(&two, "a.desktop", "Generation Two", "");
        let link = base.join("current");
        std::os::unix::fs::symlink(&one, &link).unwrap();
        let provider = AppsProvider::with(vec![link.clone()], Locale::default(), Vec::new());
        assert_eq!(names(&provider.apps()), ["Generation One"]);
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&two, &link).unwrap();
        assert_eq!(names(&provider.apps()), ["Generation Two"]);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn folder_watcher_invalidates_the_index_after_an_entry_is_added() {
        let root = tree();
        let missing = root.join("applications");
        let provider = Arc::new(AppsProvider::with(
            vec![missing.clone()],
            Locale::default(),
            Vec::new(),
        ));
        assert!(provider.apps().is_empty());
        let (changed, event) = std::sync::mpsc::channel();
        let watcher = provider
            .watch(move || changed.send(()).unwrap())
            .expect("start app folder watcher");
        write(&missing, "vendor/watched.desktop", "Watched App", "");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            event
                .recv_timeout(remaining)
                .expect("inotify should report an entry created in a watched root");
            if provider.apps().iter().any(|app| app.name == "Watched App") {
                break;
            }
        }
        drop(watcher);
        fs::remove_dir_all(root).unwrap();
    }
}
