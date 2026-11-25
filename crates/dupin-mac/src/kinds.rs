//! Dupin's kinds with what macOS supplies them: the services Send to… shares files with, and
//! which directories Finder opens as folders.

use std::path::PathBuf;

use dupin_core::Item;
use dupin_core::filter::{Sorter, SourceFilters, Stages};
use dupin_core::kind::Kinds;

pub fn native() -> Kinds {
    Kinds::native(crate::sharing::targets().into())
}

/// A file or folder row for `path`, as a native file source shows one.
pub fn file(kinds: &Kinds, path: PathBuf, title: String, matched: Vec<usize>) -> Item {
    let folder = path.is_dir() && !crate::folder::is_package(&path);
    kinds.file(path, title, matched, folder)
}

/// What a native source that matches candidates itself is composed with: the filter stages
/// that find and order them and the kinds that give its rows their actions.
#[derive(Clone)]
pub struct Rows {
    pub filters: SourceFilters,
    pub kinds: Kinds,
}

impl Rows {
    /// The fuzzy matcher, relevance order and Dupin's own kinds, for a source run outside the
    /// app.
    pub fn native() -> Self {
        Self {
            filters: SourceFilters::new(Stages::native(dupin_core::filter::fuzzy(), Sorter::Relevance)),
            kinds: native(),
        }
    }
}
