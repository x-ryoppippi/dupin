//! A query typed as a folder path: `~/` or `/` in the main search opens that folder, and a
//! slash typed after a subfolder's name while browsing enters it.

use std::path::{Path, PathBuf};

use super::{Effect, SearchLocation, Session};

/// The folders a typed path walks into, each with the text it leaves behind in the view
/// before it, and what follows the last folder.
#[derive(Debug, PartialEq, Eq)]
struct Walk<'a> {
    steps: Vec<(String, PathBuf)>,
    filter: &'a str,
}

impl Session {
    /// Walks the folders `text` names, if it names any, leaving the rest as the filter.
    pub(super) fn walk_path(&mut self, text: &str) -> Option<Vec<Effect>> {
        if self.page().is_some() || self.destination.is_some() {
            return None;
        }
        let browser = self.commands.browser()?.clone();
        let browsable = self.commands.browsable()?;
        let walk = match &self.search.location {
            SearchLocation::Folder(folder) => walk(Some(folder), text, browsable),
            SearchLocation::Command if self.search.command == *self.commands.default() => {
                walk(None, text, browsable)
            }
            _ => None,
        }?;
        for (left, folder) in walk.steps {
            self.search.query = left;
            self.push_visit(browser.clone(), SearchLocation::Folder(folder), None);
        }
        self.search.query = walk.filter.to_owned();
        let mut effects = vec![Effect::ReplaceQuery(self.search.query.clone())];
        effects.extend(self.replace_search());
        Some(effects)
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// From the main search (`from` is None) a path starts with `~/` or `/`; within a folder it is
/// relative to it. Each name followed by a slash is entered while `browsable` takes it for a
/// folder; the first that isn't, and everything after it, stays as the filter.
fn walk<'a>(from: Option<&Path>, text: &'a str, browsable: fn(&Path) -> bool) -> Option<Walk<'a>> {
    let mut steps = Vec::new();
    let (mut folder, mut rest) = match from {
        Some(folder) => (folder.to_path_buf(), text),
        None => {
            let (left, folder, rest) = if let Some(rest) = text.strip_prefix("~/") {
                ("~", home()?, rest)
            } else {
                ("", PathBuf::from("/"), text.strip_prefix('/')?)
            };
            if !browsable(&folder) {
                return None;
            }
            steps.push((left.to_owned(), folder.clone()));
            (folder, rest)
        }
    };
    while let Some((name, after)) = rest.split_once('/') {
        if name.is_empty() {
            rest = after;
            continue;
        }
        let next = folder.join(name);
        if name == "." || name == ".." || !browsable(&next) {
            break;
        }
        steps.push((name.to_owned(), next.clone()));
        folder = next;
        rest = after;
    }
    (!steps.is_empty()).then_some(Walk { steps, filter: rest })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Dev/dupin")).unwrap();
        std::fs::create_dir_all(root.path().join("Tool.app/Contents")).unwrap();
        std::fs::create_dir(root.path().join(".config")).unwrap();
        std::fs::write(root.path().join("notes.txt"), "").unwrap();
        root
    }

    /// A directory, except one named like an app bundle, as the app's predicate treats packages.
    fn browsable(path: &Path) -> bool {
        path.is_dir() && path.extension().is_none_or(|extension| extension != "app")
    }

    fn filter<'a>(from: &Path, text: &'a str) -> Option<&'a str> {
        walk(Some(from), text, browsable).map(|walk| walk.filter)
    }

    #[test]
    fn a_path_enters_each_folder_it_names_and_filters_by_the_rest() {
        let root = tree();
        let root = root.path();
        let walk = walk(Some(root), "Dev/dupin/REA", browsable).unwrap();
        let steps: Vec<_> = walk
            .steps
            .iter()
            .map(|(left, folder)| (left.as_str(), folder.clone()))
            .collect();
        assert_eq!(
            steps,
            [("Dev", root.join("Dev")), ("dupin", root.join("Dev/dupin"))]
        );
        assert_eq!(walk.filter, "REA");
        assert_eq!(filter(root, ".config/"), Some(""));
        assert_eq!(filter(root, "Dev//dupin/"), Some(""), "a doubled slash stays put");
    }

    #[test]
    fn an_absolute_path_starts_at_the_root() {
        let root = tree();
        let text = format!("{}/Dev/x", root.path().display());
        let walk = walk(None, &text, browsable).unwrap();
        assert_eq!(walk.steps[0], (String::new(), PathBuf::from("/")));
        assert_eq!(walk.steps.last().unwrap().1, root.path().join("Dev"));
        assert_eq!(walk.filter, "x");
    }

    #[test]
    fn a_name_that_is_not_a_browsable_folder_stays_in_the_filter() {
        let root = tree();
        let root = root.path();
        assert_eq!(filter(root, "Dve/dupin/"), None);
        assert_eq!(filter(root, "Dev/Dve/x"), Some("Dve/x"));
        assert_eq!(filter(root, "notes.txt/"), None);
        assert_eq!(
            filter(root, "Tool.app/"),
            None,
            "a package opens rather than browses"
        );
        assert_eq!(filter(root, "../"), None);
    }

    #[test]
    fn the_main_search_walks_only_a_path_from_home_or_the_root() {
        for text in ["~", "Dev/", "~Dev/", " ~/Dev/"] {
            assert_eq!(walk(None, text, browsable), None, "{text:?}");
        }
    }
}
