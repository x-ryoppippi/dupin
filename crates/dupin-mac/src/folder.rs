//! A folder's entries, for browsing it within the launcher.

use std::cmp::Reverse;
use std::fs::DirEntry;
use std::path::{Path, PathBuf};

use dupin_core::{Item, Provider, ProviderError, Request};
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::NSString;

use crate::kinds::Rows;

/// Lists `Request::folder`, folders first and then by name. The query filters the entries
/// with the source's matcher, best matches first unless it sorts by title; hidden entries
/// appear only when it starts with a dot.
pub struct FolderProvider {
    rows: Rows,
}

impl FolderProvider {
    pub fn new(rows: Rows) -> Self {
        Self { rows }
    }
}

struct Entry {
    path: PathBuf,
    name: String,
    folder: bool,
    score: Option<i32>,
    matched: Vec<usize>,
}

impl Provider for FolderProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        let Some(folder) = &request.folder else {
            return Ok(());
        };
        let filter = request.query.trim();
        let stages = self.rows.filters.current();
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(folder).map_err(|_| ProviderError)? {
            if request.is_cancelled() {
                return Ok(());
            }
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !filter.starts_with('.') {
                continue;
            }
            entries.push(Entry {
                folder: is_folder(&entry),
                path: entry.path(),
                name,
                score: None,
                matched: Vec::new(),
            });
        }
        if !filter.is_empty() {
            let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
            let found = stages.batch_matches(filter, &names).unwrap_or_else(|| {
                names
                    .iter()
                    .map(|name| stages.matcher.score(filter, name))
                    .collect()
            });
            entries = entries
                .into_iter()
                .zip(found)
                .filter_map(|(entry, found)| {
                    let found = found?;
                    Some(Entry {
                        score: Some(found.score),
                        matched: found.positions,
                        ..entry
                    })
                })
                .collect();
        }
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        match stages.batch_order(&names) {
            Some(order) => {
                let mut slots: Vec<_> = entries.into_iter().map(Some).collect();
                entries = order
                    .into_iter()
                    .filter_map(|index| slots[index].take())
                    .collect();
            }
            None => {
                if stages.sorter == dupin_core::filter::Sorter::Title {
                    entries.iter_mut().for_each(|entry| entry.score = None);
                }
                entries.sort_by_cached_key(|entry| {
                    (Reverse(entry.score), !entry.folder, entry.name.to_lowercase())
                });
            }
        }
        if stages.ordered() {
            entries.iter_mut().for_each(|entry| entry.score = None);
        }
        emit(
            entries
                .into_iter()
                .take(request.limit)
                .map(|entry| item(&self.rows.kinds, entry))
                .collect(),
        );
        Ok(())
    }
}

fn item(kinds: &dupin_core::kind::Kinds, entry: Entry) -> Item {
    let mut item = kinds.file(entry.path, entry.name, entry.matched, entry.folder);
    if let dupin_core::View::Text { subtitle, .. } = &mut item.view {
        // Every entry shares the browsed folder, which the chip already names.
        subtitle.clear();
    }
    item.score = entry.score;
    item
}

/// A directory that isn't a package such as an app bundle, which Finder shows as a file.
fn is_folder(entry: &DirEntry) -> bool {
    let path = entry.path();
    let directory = match entry.file_type() {
        Ok(kind) if kind.is_symlink() => path.is_dir(),
        Ok(kind) => kind.is_dir(),
        Err(_) => false,
    };
    directory && !is_package(&path)
}

/// A folder that browsing lists, following symlinks: a directory that isn't a package.
pub fn is_browsable(path: &Path) -> bool {
    path.is_dir() && !is_package(path)
}

pub(crate) fn is_package(path: &Path) -> bool {
    autoreleasepool(|_| {
        NSWorkspace::sharedWorkspace().isFilePackageAtPath(&NSString::from_str(&path.to_string_lossy()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(test: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dupin-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["beta", "Alpha", ".git", "Tool.app/Contents"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in ["alpha.txt", "zeta.md", ".hidden"] {
            std::fs::write(root.join(file), "").unwrap();
        }
        root
    }

    fn list(folder: &Path, query: &str) -> Vec<Item> {
        let mut request = Request::detached(query, 100);
        request.folder = Some(folder.to_path_buf());
        let mut items = Vec::new();
        FolderProvider::new(Rows::native())
            .search(&request, &mut |batch| items.extend(batch))
            .unwrap();
        items
    }

    fn titles(items: &[Item]) -> Vec<&str> {
        items.iter().map(Item::display_title).collect()
    }

    #[test]
    fn lists_folders_first_then_by_name_without_hidden_entries() {
        let root = fixture("listing");
        let items = list(&root, "");
        assert_eq!(
            titles(&items),
            ["Alpha", "beta", "alpha.txt", "Tool.app", "zeta.md"]
        );
        let browsable = |title: &str| {
            items
                .iter()
                .find(|item| item.display_title() == title)
                .unwrap()
                .action("browse")
                .is_some()
        };
        assert!(browsable("beta"));
        assert!(!browsable("Tool.app"), "a package opens rather than browses");
        assert!(!browsable("alpha.txt"));
        assert!(is_browsable(&root.join("beta")));
        assert!(
            !is_browsable(&root.join("Tool.app")),
            "a typed path skips packages too"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_query_filters_fuzzily_and_a_leading_dot_shows_hidden_entries() {
        let root = fixture("filter");
        assert_eq!(titles(&list(&root, "alp")), ["Alpha", "alpha.txt"]);
        assert_eq!(titles(&list(&root, ".h")), [".hidden"]);
        assert!(list(&root, "zz").is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_folder_fails_and_no_folder_lists_nothing() {
        let mut request = Request::detached("", 10);
        request.folder = Some("/nonexistent/dupin".into());
        let provider = FolderProvider::new(Rows::native());
        assert!(provider.search(&request, &mut |_| {}).is_err());
        request.folder = None;
        assert!(provider.search(&request, &mut |_| unreachable!()).is_ok());
    }
}
