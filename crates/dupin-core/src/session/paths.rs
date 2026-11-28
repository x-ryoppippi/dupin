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
        let walk = match &self.search.location {
            SearchLocation::Folder(folder) => walk(Some(folder), text, None),
            SearchLocation::Command if self.search.command == *self.commands.default() => {
                walk(None, text, home().as_deref())
            }
            _ => None,
        }?;
        let browser = self.commands.browser()?.clone();
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
/// relative to it. Each name followed by a slash is entered while it names a folder; the first
/// that doesn't, and everything after it, stays as the filter.
fn walk<'a>(from: Option<&Path>, text: &'a str, home: Option<&Path>) -> Option<Walk<'a>> {
    let mut steps = Vec::new();
    let (mut folder, mut rest) = match from {
        Some(folder) => (folder.to_path_buf(), text),
        None => {
            let (left, folder, rest) = if let Some(rest) = text.strip_prefix("~/") {
                ("~", home?.to_path_buf(), rest)
            } else {
                ("", PathBuf::from("/"), text.strip_prefix('/')?)
            };
            if !folder.is_dir() {
                return None;
            }
            steps.push((left.to_owned(), folder.clone()));
            (folder, rest)
        }
    };
    while let Some((name, after)) = rest.split_once('/') {
        let next = folder.join(name);
        if name.is_empty() || name == "." || name == ".." || !next.is_dir() {
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
        std::fs::create_dir(root.path().join(".config")).unwrap();
        std::fs::write(root.path().join("notes.txt"), "").unwrap();
        root
    }

    fn steps<'a>(walk: &'a Walk<'_>) -> Vec<(&'a str, &'a Path)> {
        walk.steps
            .iter()
            .map(|(left, folder)| (left.as_str(), folder.as_path()))
            .collect()
    }

    #[test]
    fn a_home_path_enters_each_folder_it_names_and_filters_the_last() {
        let home = tree();
        let home = home.path();
        let walk = walk(None, "~/Dev/dupin/REA", Some(home)).unwrap();
        assert_eq!(
            steps(&walk),
            [
                ("~", home),
                ("Dev", &*home.join("Dev")),
                ("dupin", &*home.join("Dev/dupin"))
            ]
        );
        assert_eq!(walk.filter, "REA");
        assert_eq!(walk_filter(None, "~/", home), Some(""));
        assert_eq!(walk_filter(None, "~/.config/", home), Some(""));
    }

    #[test]
    fn an_absolute_path_starts_at_the_root() {
        let root = tree();
        let text = format!("{}/Dev/x", root.path().display());
        let walk = walk(None, &text, None).unwrap();
        assert_eq!(walk.steps[0], (String::new(), PathBuf::from("/")));
        assert_eq!(walk.steps.last().unwrap().1, root.path().join("Dev"));
        assert_eq!(walk.filter, "x");
        assert_eq!(walk_filter(None, "/", root.path()), Some(""));
    }

    #[test]
    fn a_name_that_is_not_a_folder_stays_in_the_filter() {
        let home = tree();
        let home = home.path();
        assert_eq!(walk_filter(None, "~/Dve/dupin/", home), Some("Dve/dupin/"));
        assert_eq!(walk_filter(None, "~/notes.txt/", home), Some("notes.txt/"));
        assert_eq!(walk_filter(None, "~/Dev//", home), Some("/"));
        assert_eq!(walk_filter(None, "~/../", home), Some("../"));
    }

    #[test]
    fn the_main_search_walks_only_a_path_from_home_or_the_root() {
        let home = tree();
        let home = home.path();
        for text in ["~", "Dev/", "~Dev/", " ~/Dev/"] {
            assert_eq!(walk(None, text, Some(home)), None, "{text:?}");
        }
        assert_eq!(walk(None, "~/Dev/", None), None, "no home to start from");
    }

    #[test]
    fn within_a_folder_a_path_is_relative_to_it() {
        let root = tree();
        let walk = walk(Some(root.path()), "Dev/dupin/a", None).unwrap();
        assert_eq!(
            steps(&walk),
            [
                ("Dev", &*root.path().join("Dev")),
                ("dupin", &*root.path().join("Dev/dupin"))
            ]
        );
        assert_eq!(walk.filter, "a");
        assert_eq!(super::walk(Some(root.path()), "Dev", None), None);
        assert_eq!(super::walk(Some(root.path()), "Dve/", None), None);
    }

    fn walk_filter<'a>(from: Option<&Path>, text: &'a str, home: &Path) -> Option<&'a str> {
        walk(from, text, Some(home)).map(|walk| walk.filter)
    }
}
