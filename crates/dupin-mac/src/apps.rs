//! Installed apps: `.app` bundles in the standard folders, fuzzy-matched by every name the
//! system knows them by.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use dupin_core::fuzzy;
use dupin_core::{Item, ItemKind, Provider, Request};

/// Rescan when the index is older than this, so a newly installed app turns up without a
/// file-system watcher. A rescan of the standard folders takes a few milliseconds once each
/// bundle's names are cached.
const STALE_AFTER: Duration = Duration::from_secs(10);
/// How deep to look for bundles below a root: `/Applications/Utilities/X.app` is depth 2.
const MAX_DEPTH: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    /// The name Finder shows: localized, without `.app`.
    pub name: String,
    /// Other names that should find the app: the file name when it differs from `name`, and
    /// Spotlight's alternate names and keywords (ChatGPT.app answers to "Codex", its former
    /// name; System Settings to "System Preferences").
    pub aliases: Vec<String>,
    pub path: PathBuf,
    /// The bundle behind a symlink (Safari lives in a Cryptex and /Applications links to
    /// it): Finder draws an alias badge on the link's icon but not on the bundle's.
    pub icon: PathBuf,
}

pub struct AppsProvider {
    roots: Vec<PathBuf>,
    index: Mutex<Option<(Instant, Vec<App>)>>,
    /// Each bundle's names, kept while its modification time stays the same: reading them
    /// from the system costs about half a millisecond a bundle, too much to repeat for every
    /// app on each rescan.
    names: Mutex<HashMap<PathBuf, (Option<SystemTime>, Names)>>,
}

type Names = (String, Vec<String>);

impl AppsProvider {
    /// `/Applications`, `/System/Applications` and `~/Applications`, with their subfolders
    /// (`Utilities` among them).
    pub fn standard() -> Self {
        let mut roots = vec![
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(Path::new(&home).join("Applications"));
        }
        Self::with_roots(roots)
    }

    pub fn with_roots(roots: Vec<PathBuf>) -> Self {
        AppsProvider {
            roots,
            index: Mutex::new(None),
            names: Mutex::new(HashMap::new()),
        }
    }

    /// Scan now if the index is missing or stale, so the first search after launch or after
    /// a while doesn't pay for it.
    pub fn warm(&self) {
        drop(self.apps());
    }

    fn apps(&self) -> Vec<App> {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        match &*index {
            Some((at, apps)) if at.elapsed() < STALE_AFTER => apps.clone(),
            _ => {
                let apps: Vec<App> = scan(&self.roots)
                    .into_iter()
                    .map(|app| {
                        let (display, extra) = self.names_of(&app.icon);
                        app.with_names(&display, extra)
                    })
                    .collect();
                *index = Some((Instant::now(), apps.clone()));
                apps
            }
        }
    }
}

impl AppsProvider {
    fn names_of(&self, bundle: &Path) -> Names {
        let modified = fs::metadata(bundle).and_then(|m| m.modified()).ok();
        let mut cache = self.names.lock().unwrap_or_else(|e| e.into_inner());
        match cache.get(bundle) {
            Some((at, names)) if modified.is_some() && *at == modified => names.clone(),
            _ => {
                let names = crate::bundle_names::lookup(bundle);
                cache.insert(bundle.to_owned(), (modified, names.clone()));
                names
            }
        }
    }
}

impl Provider for AppsProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) {
        emit(rank(&self.apps(), &request.query, request.limit));
    }
}

impl App {
    /// Take the displayed name and the extra names the system reports, keeping the file name
    /// as an alias when it differs. Names lose a trailing `.app`, and duplicates (ignoring
    /// case) and empty names are dropped.
    pub fn with_names(self, display: &str, extra: Vec<String>) -> App {
        let clean = |n: &str| n.trim().trim_end_matches(".app").trim().to_owned();
        let display = clean(display);
        let name = if display.is_empty() {
            self.name.clone()
        } else {
            display
        };
        let mut aliases: Vec<String> = Vec::new();
        for alias in std::iter::once(self.name).chain(extra).map(|n| clean(&n)) {
            let same = |n: &String| n.to_lowercase() == alias.to_lowercase();
            if !alias.is_empty() && !same(&name) && !aliases.iter().any(same) {
                aliases.push(alias);
            }
        }
        App {
            name,
            aliases,
            ..self
        }
    }
}

/// A match on an alias ranks below the same match on the displayed name.
const ALIAS_PENALTY: i32 = 8;

/// Best matches first; equal scores keep the shorter, then alphabetical, name. An app matched
/// through an alias shows the alias in its subtitle, since its title doesn't contain the query.
pub fn rank(apps: &[App], query: &str, limit: usize) -> Vec<Item> {
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
            let item = Item::path(
                ItemKind::Application,
                app.path.clone(),
                app.name.clone(),
                positions,
            );
            Item {
                icon: Some(app.icon.clone()),
                score: Some(score),
                subtitle: match alias {
                    Some(alias) => format!("{alias} · {}", item.subtitle),
                    None => item.subtitle.clone(),
                },
                ..item
            }
        })
        .collect()
}

/// Every `.app` under `roots`, one entry per name: the first root wins, so an app in
/// `/Applications` hides a same-named copy further down the list.
pub fn scan(roots: &[PathBuf]) -> Vec<App> {
    let mut apps = Vec::new();
    for root in roots {
        walk(root, 0, &mut apps);
    }
    let mut seen = std::collections::HashSet::new();
    apps.retain(|a: &App| seen.insert(a.name.clone()));
    apps
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<App>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        // Bundles are directories; follow symlinks too (Xcode-select style links into /Applications).
        let is_dir = kind.is_dir() || (kind.is_symlink() && path.is_dir());
        if !is_dir {
            continue;
        }
        if path.extension().is_some_and(|e| e == "app") {
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                let icon = if kind.is_symlink() {
                    fs::canonicalize(&path).unwrap_or_else(|_| path.clone())
                } else {
                    path.clone()
                };
                out.push(App {
                    name: name.to_owned(),
                    aliases: Vec::new(),
                    path,
                    icon,
                });
            }
        } else if depth + 1 < MAX_DEPTH {
            walk(&path, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(paths: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dupin-apps-{}-{}", std::process::id(), paths.len()));
        let _ = fs::remove_dir_all(&root);
        for p in paths {
            fs::create_dir_all(root.join(p)).unwrap();
        }
        root
    }

    #[test]
    fn finds_bundles_one_folder_deep_but_not_inside_bundles() {
        let root = tree(&[
            "Safari.app/Contents/MacOS",
            "Utilities/Terminal.app",
            "Utilities/Deeper/Hidden.app",
            "Xcode.app/Contents/Applications/Instruments.app",
            "NotAnApp",
        ]);
        let names: Vec<_> = scan(std::slice::from_ref(&root))
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, ["Safari", "Terminal", "Xcode"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_linked_bundle_takes_its_icon_from_the_target() {
        let root = tree(&["Real/Linked.app/Contents"]);
        let apps_dir = root.join("Apps");
        fs::create_dir_all(&apps_dir).unwrap();
        std::os::unix::fs::symlink(root.join("Real/Linked.app"), apps_dir.join("Linked.app")).unwrap();
        let apps = scan(std::slice::from_ref(&apps_dir));
        assert_eq!(apps[0].path, apps_dir.join("Linked.app"));
        assert_eq!(
            apps[0].icon,
            fs::canonicalize(root.join("Real/Linked.app")).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ranks_by_fuzzy_score() {
        let apps: Vec<App> = ["Safari Technology Preview", "Safari", "Hush Safe", "Notes"]
            .iter()
            .map(|n| {
                let path = PathBuf::from(format!("/Applications/{n}.app"));
                App {
                    name: (*n).into(),
                    aliases: Vec::new(),
                    icon: path.clone(),
                    path,
                }
            })
            .collect();
        let titles: Vec<_> = rank(&apps, "saf", 10).into_iter().map(|i| i.title).collect();
        assert_eq!(titles, ["Safari", "Safari Technology Preview", "Hush Safe"]);
        assert_eq!(rank(&apps, "saf", 1).len(), 1);
    }

    #[test]
    fn results_open_the_link_but_show_the_bundles_icon() {
        let app = App {
            name: "Safari".into(),
            aliases: Vec::new(),
            path: PathBuf::from("/Applications/Safari.app"),
            icon: PathBuf::from("/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app"),
        };
        let item = rank(std::slice::from_ref(&app), "saf", 1).remove(0);
        assert_eq!(item.primary, dupin_core::Action::Open(app.path.clone()));
        assert_eq!(item.icon, Some(app.icon));
    }

    fn app(name: &str, aliases: &[&str]) -> App {
        let path = PathBuf::from(format!("/Applications/{name}.app"));
        App {
            name: name.into(),
            aliases: aliases.iter().map(|a| (*a).into()).collect(),
            icon: path.clone(),
            path,
        }
    }

    #[test]
    fn an_alias_finds_the_app_and_shows_in_the_subtitle() {
        let apps = [app("ChatGPT", &["Codex"]), app("Calculator", &[])];
        let items = rank(&apps, "codex", 10);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "ChatGPT");
        assert!(items[0].matched.is_empty(), "nothing in the title matched");
        assert_eq!(items[0].subtitle, "Codex · /Applications");
    }

    #[test]
    fn the_displayed_name_beats_an_equal_alias_match() {
        let apps = [app("Preview Tool", &[]), app("Something", &["Preview Tool"])];
        let titles: Vec<_> = rank(&apps, "prev", 10).into_iter().map(|i| i.title).collect();
        assert_eq!(titles, ["Preview Tool", "Something"]);
    }

    #[test]
    fn names_are_cleaned_and_deduplicated() {
        let scanned = App {
            name: "System Settings".into(),
            ..app("x", &[])
        };
        let extra = [
            "Settings",
            "System Settings.app",
            "System Preferences",
            "",
            "settings",
        ]
        .map(String::from)
        .to_vec();
        let named = scanned.with_names("システム設定.app", extra);
        assert_eq!(named.name, "システム設定");
        assert_eq!(
            named.aliases,
            ["System Settings", "Settings", "System Preferences"]
        );
    }
}
