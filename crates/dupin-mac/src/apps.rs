//! Installed apps: `.app` bundles in the standard folders, fuzzy-matched by every name the
//! system knows them by.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use dupin_core::filter::Sorter;
use dupin_core::kind::Kinds;
use dupin_core::{Item, Provider, ProviderError, Request, View};

use crate::kinds::Rows;

pub type AppsWatcher = crate::watcher::PathWatcher;

/// How deep to look for bundles below a root: `/Applications/Utilities/X.app` is depth 2.
const MAX_DEPTH: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
struct App {
    /// The name Finder shows: localized, without `.app`.
    name: String,
    /// Other names that should find the app: the file name when it differs from `name`, and
    /// Spotlight's alternate names and keywords (ChatGPT.app answers to "Codex", its former
    /// name; System Settings to "System Preferences").
    aliases: Vec<String>,
    path: PathBuf,
    /// The bundle behind a symlink (Safari lives in a Cryptex and /Applications links to
    /// it): Finder draws an alias badge on the link's icon but not on the bundle's.
    icon: PathBuf,
}

pub struct AppsProvider {
    roots: Vec<PathBuf>,
    index: Mutex<Option<(u64, Vec<App>)>>,
    revision: AtomicU64,
    /// Each bundle's names, kept while its modification time stays the same: reading them
    /// from the system costs about half a millisecond a bundle, too much to repeat for every
    /// app on each rescan.
    names: Mutex<HashMap<PathBuf, (Option<SystemTime>, Names)>>,
    rows: Rows,
}

type Names = (String, Vec<String>);

impl AppsProvider {
    /// `/Applications`, `/System/Applications` and `~/Applications`, with their subfolders
    /// (`Utilities` among them).
    pub fn standard(rows: Rows) -> Self {
        let mut roots = vec![
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(Path::new(&home).join("Applications"));
        }
        Self::with_roots(roots, rows)
    }

    /// The bundles in `roots` and their subfolders only.
    pub fn with_roots(roots: Vec<PathBuf>, rows: Rows) -> Self {
        AppsProvider {
            rows,
            roots,
            index: Mutex::new(None),
            revision: AtomicU64::new(0),
            names: Mutex::new(HashMap::new()),
        }
    }

    /// These bundles only, each with the name Finder shows and the names Spotlight adds,
    /// without scanning or reading the system's: fixture apps in place of the Mac's.
    pub fn fixed(apps: impl IntoIterator<Item = (PathBuf, String, Vec<String>)>, rows: Rows) -> Self {
        let apps = apps
            .into_iter()
            .map(|(path, display, extra)| {
                let name = path
                    .file_stem()
                    .map_or_else(|| display.clone(), |stem| stem.to_string_lossy().into_owned());
                App {
                    name,
                    aliases: Vec::new(),
                    icon: path.clone(),
                    path,
                }
                .with_names(&display, extra)
            })
            .collect();
        let provider = Self::with_roots(Vec::new(), rows);
        *provider.index.lock().unwrap_or_else(|e| e.into_inner()) = Some((0, apps));
        provider
    }

    /// Watch the application folders and invalidate the cached list when an app changes.
    /// Bundle writes are debounced before the cached list is rebuilt.
    pub fn watch(
        self: &Arc<Self>,
        changed: impl Fn() + Send + Sync + 'static,
    ) -> Result<AppsWatcher, &'static str> {
        let roots = self.roots.clone();
        let provider = Arc::downgrade(self);
        AppsWatcher::start(roots, app_subfolders, move || {
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

    pub(crate) fn catalog_items(&self) -> HashMap<PathBuf, Item> {
        self.apps()
            .into_iter()
            .map(|app| (app.path.clone(), app.item(&self.rows.kinds, Vec::new())))
            .collect()
    }

    fn apps(&self) -> Vec<App> {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let revision = self.revision.load(Ordering::Acquire);
        if let Some((cached_revision, apps)) = &*index
            && *cached_revision == revision
        {
            return apps.clone();
        }
        let apps: Vec<App> = scan(&self.roots)
            .into_iter()
            .map(|app| {
                let (display, extra) = self.names_of(&app.icon);
                app.with_names(&display, extra)
            })
            .collect();
        *index = Some((revision, apps.clone()));
        apps
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
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        emit(rank(&self.apps(), &request.query, request.limit, &self.rows));
        Ok(())
    }
}

impl App {
    fn item(&self, kinds: &Kinds, matched: Vec<usize>) -> Item {
        Item {
            icon: Some(dupin_core::Icon::File(self.icon.clone())),
            ..kinds.app(self.path.clone(), self.name.clone(), matched)
        }
    }

    /// Take the displayed name and the extra names the system reports, keeping the file name
    /// as an alias when it differs. Names lose a trailing `.app`, and duplicates (ignoring
    /// case) and empty names are dropped.
    fn with_names(self, display: &str, extra: Vec<String>) -> App {
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

/// Best matches first; equal scores keep the shorter, then alphabetical, name. Sorted by
/// title or by a package's sorter instead, the rows carry no score, so the usage history keeps
/// that order. An app matched through an alias shows the alias in its subtitle, since its
/// title doesn't contain the query. A package's matcher sees every name and alias in one batch.
fn rank(apps: &[App], query: &str, limit: usize, rows: &Rows) -> Vec<Item> {
    let stages = rows.filters.current();
    let matcher = &*stages.matcher;
    let batch = stages.batch_matcher.as_ref().and_then(|_| {
        let texts: Vec<&str> = apps
            .iter()
            .flat_map(|app| std::iter::once(app.name.as_str()).chain(app.aliases.iter().map(String::as_str)))
            .collect();
        stages.batch_matches(query, &texts)
    });
    let mut batched = batch.into_iter().flatten();
    let mut score = |text: &str| match batched.next() {
        Some(found) => found,
        None => matcher.score(query, text),
    };
    let mut hits: Vec<(i32, Option<&str>, Vec<usize>, &App)> = apps
        .iter()
        .filter_map(|app| {
            let on_name = score(&app.name).map(|m| (m.score, None, m.positions));
            let on_alias = app
                .aliases
                .iter()
                .filter_map(|a| score(a).map(|m| (m.score - ALIAS_PENALTY, Some(a.as_str()), Vec::new())))
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
    let order = stages.batch_sorter.as_ref().and_then(|_| {
        let titles: Vec<&str> = hits.iter().map(|(.., app)| app.name.as_str()).collect();
        stages.batch_order(&titles)
    });
    let by_title = match order {
        Some(order) => {
            let mut slots: Vec<_> = hits.into_iter().map(Some).collect();
            hits = order
                .into_iter()
                .filter_map(|index| slots[index].take())
                .collect();
            true
        }
        None if stages.sorter == Sorter::Title => {
            hits.sort_by_cached_key(|(.., app)| (app.name.to_lowercase(), app.name.clone()));
            true
        }
        None => {
            hits.sort_by(|(a, _, _, x), (b, _, _, y)| b.cmp(a).then_with(|| x.name.cmp(&y.name)));
            false
        }
    };
    hits.truncate(limit);
    hits.into_iter()
        .map(|(score, alias, positions, app)| {
            let mut item = app.item(&rows.kinds, positions);
            if let (Some(alias), View::Text { subtitle, .. }) = (alias, &mut item.view) {
                *subtitle = format!("{alias} · {subtitle}");
            }
            Item {
                score: (!by_title).then_some(score),
                completion: None,
                ..item
            }
        })
        .collect()
}

/// Every `.app` under `roots`, one entry per name: the first root wins, so an app in
/// `/Applications` hides a same-named copy further down the list.
fn scan(roots: &[PathBuf]) -> Vec<App> {
    let mut apps = Vec::new();
    for root in roots {
        walk(root, 0, &mut apps);
    }
    let mut seen = std::collections::HashSet::new();
    apps.retain(|a: &App| seen.insert(a.name.clone()));
    apps
}

fn app_subfolders(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter_map(|root| fs::read_dir(root).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| !is_bundle(path) && path.is_dir())
        .collect()
}

fn is_bundle(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "app")
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
        if is_bundle(&path) {
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
    fn text_subtitle(item: &dupin_core::Item) -> &str {
        let dupin_core::View::Text { subtitle, .. } = &item.view else {
            panic!("expected text")
        };
        subtitle
    }
    use super::*;

    fn rows() -> Rows {
        Rows::native()
    }

    fn rank(apps: &[App], query: &str, limit: usize) -> Vec<Item> {
        super::rank(apps, query, limit, &rows())
    }

    static NEXT_TREE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn tree(paths: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dupin-apps-{}-{}-{}",
            std::process::id(),
            paths.len(),
            NEXT_TREE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
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
        let mut subfolders = app_subfolders(std::slice::from_ref(&root));
        subfolders.sort();
        assert_eq!(subfolders, [root.join("NotAnApp"), root.join("Utilities")]);
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
    fn fixed_apps_are_found_by_every_name_without_a_scan() {
        let provider = AppsProvider::fixed(
            [(
                PathBuf::from("/System/Applications/System Settings.app"),
                "System Settings".into(),
                vec!["System Preferences".into()],
            )],
            rows(),
        );
        let mut found = Vec::new();
        provider
            .search(&Request::detached("prefs", 10), &mut |items| found.extend(items))
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "/System/Applications/System Settings.app");
        assert_eq!(
            text_subtitle(&found[0]),
            "System Preferences · /System/Applications"
        );
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
        let titles: Vec<_> = rank(&apps, "saf", 10)
            .into_iter()
            .map(|i| i.display_title().to_owned())
            .collect();
        assert_eq!(titles, ["Safari", "Safari Technology Preview", "Hush Safe"]);
        assert_eq!(rank(&apps, "saf", 1).len(), 1);
        for item in rank(&apps, "saf", 10) {
            assert!(item.action("open-with").is_none());
            assert!(item.action("trash").is_none());
        }
    }

    #[test]
    fn ranks_with_the_matcher_it_is_composed_with() {
        struct Prefix;
        impl dupin_core::filter::Matcher for Prefix {
            fn name(&self) -> &'static str {
                "test.prefix"
            }
            fn score(&self, query: &str, text: &str) -> Option<dupin_core::fuzzy::Match> {
                text.to_lowercase()
                    .starts_with(&query.to_lowercase())
                    .then(|| dupin_core::fuzzy::Match {
                        score: 0,
                        positions: (0..query.chars().count()).collect(),
                    })
            }
        }
        let apps = [
            app("Hush Safe", &[]),
            app("Safari", &[]),
            app("Notes", &["Safe Notes"]),
        ];
        let rows = Rows {
            filters: dupin_core::filter::SourceFilters::new(dupin_core::filter::Stages::native(
                Arc::new(Prefix),
                Sorter::Relevance,
            )),
            kinds: crate::kinds::native(),
        };
        let items = super::rank(&apps, "saf", 10, &rows);
        let titles: Vec<_> = items.iter().map(Item::display_title).collect();
        assert_eq!(titles, ["Safari", "Notes"]);
        assert_eq!(items[0].matched, [0, 1, 2]);
        assert_eq!(text_subtitle(&items[1]), "Safe Notes · /Applications");
        assert_eq!(
            rank(&apps, "saf", 10).len(),
            3,
            "the fuzzy matcher finds all three"
        );
    }

    #[test]
    fn sorted_by_title_matches_keep_alphabetical_order_without_scores() {
        let apps = [app("Safari", &[]), app("Hush Safe", &[]), app("safety", &[])];
        let rows = Rows::native();
        let relevance: Vec<_> = super::rank(&apps, "saf", 2, &rows)
            .iter()
            .map(|item| (item.display_title().to_owned(), item.score.is_some()))
            .collect();
        assert_eq!(relevance, [("Safari".into(), true), ("safety".into(), true)]);
        rows.filters.set(dupin_core::filter::Stages::native(
            dupin_core::filter::fuzzy(),
            Sorter::Title,
        ));
        let titled: Vec<_> = super::rank(&apps, "saf", 2, &rows)
            .iter()
            .map(|item| (item.display_title().to_owned(), item.score))
            .collect();
        assert_eq!(titled, [("Hush Safe".into(), None), ("Safari".into(), None)]);
    }

    #[test]
    fn a_batch_stage_matches_names_and_aliases_and_the_native_one_stands_in_when_it_cannot() {
        use dupin_core::filter::{BatchStage, Hit, Stages};

        /// Finds names ending in the query, and orders by length, longest first; `None` when
        /// told to fail.
        struct Suffix(bool);
        impl BatchStage for Suffix {
            fn name(&self) -> &str {
                "test/suffix"
            }
            fn run(&self, query: &str, titles: &[&str]) -> Option<Vec<Hit>> {
                if !self.0 {
                    return None;
                }
                let mut hits: Vec<_> = titles
                    .iter()
                    .enumerate()
                    .filter(|(_, title)| query.is_empty() || title.to_lowercase().ends_with(query))
                    .map(|(index, title)| Hit {
                        index,
                        score: Some(title.len() as i32),
                        positions: Vec::new(),
                    })
                    .collect();
                if query.is_empty() {
                    hits.sort_by_key(|hit| std::cmp::Reverse(titles[hit.index].len()));
                }
                Some(hits)
            }
        }
        let apps = [
            app("Safari", &[]),
            app("Notes", &["Memo Notes"]),
            app("Calendar", &["iCal Notes"]),
        ];
        let rows = Rows::native();
        let compose = |works: bool| {
            let mut stages = Stages::native(dupin_core::filter::fuzzy(), Sorter::Relevance);
            stages.batch_matcher = Some(Arc::new(Suffix(works)));
            stages.batch_sorter = Some(Arc::new(Suffix(works)));
            rows.filters.set(stages);
        };
        compose(true);
        let found = super::rank(&apps, "notes", 10, &rows);
        let titles: Vec<_> = found.iter().map(Item::display_title).collect();
        assert_eq!(
            titles,
            ["Calendar", "Notes"],
            "longest first, an alias finding Calendar"
        );
        assert_eq!(text_subtitle(&found[0]), "iCal Notes · /Applications");
        assert!(found.iter().all(|item| item.score.is_none()));
        compose(false);
        let fallen_back = super::rank(&apps, "saf", 10, &rows);
        assert_eq!(fallen_back.len(), 1);
        assert_eq!(fallen_back[0].display_title(), "Safari");
        assert!(fallen_back[0].score.is_some(), "the native stages rank as ever");
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
        assert_eq!(
            item.action("open"),
            Some(&dupin_core::Action::Open(app.path.clone()))
        );
        assert_eq!(item.icon, Some(dupin_core::Icon::File(app.icon)));
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
        assert_eq!(items[0].display_title(), "ChatGPT");
        assert!(items[0].matched.is_empty(), "nothing in the title matched");
        assert_eq!(text_subtitle(&items[0]), "Codex · /Applications");
    }

    #[test]
    fn the_displayed_name_beats_an_equal_alias_match() {
        let apps = [app("Preview Tool", &[]), app("Something", &["Preview Tool"])];
        let titles: Vec<_> = rank(&apps, "prev", 10)
            .into_iter()
            .map(|i| i.display_title().to_owned())
            .collect();
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

    #[test]
    fn invalidating_the_index_picks_up_new_bundles() {
        let root = tree(&[]);
        let provider = AppsProvider::with_roots(vec![root.clone()], rows());
        assert!(provider.apps().is_empty());

        fs::create_dir_all(root.join("New App.app")).unwrap();
        provider.invalidate();

        assert_eq!(provider.apps()[0].name, "New App");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_watcher_invalidates_the_index_after_a_bundle_is_added() {
        let root = tree(&[]);
        let missing = root.join("Applications");
        let provider = Arc::new(AppsProvider::with_roots(vec![missing.clone()], rows()));
        assert!(provider.apps().is_empty());
        let (changed, event) = std::sync::mpsc::channel();
        let watcher = provider
            .watch(move || changed.send(()).unwrap())
            .expect("start app folder watcher");

        fs::create_dir_all(missing.join("Watched App.app")).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            event
                .recv_timeout(remaining)
                .expect("the watcher should invalidate the index after a bundle is added");
            if provider.apps().iter().any(|app| app.name == "Watched App") {
                break;
            }
        }
        drop(watcher);
        fs::remove_dir_all(root).unwrap();
    }
}
