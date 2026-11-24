//! File and folder results.

use std::path::PathBuf;

use dupin_core::Item;

/// A file or folder result that opens it.
pub fn item(path: PathBuf, title: String, matched: Vec<usize>) -> Item {
    Item::path(path, title, matched)
}
