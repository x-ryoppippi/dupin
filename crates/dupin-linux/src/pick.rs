//! Choosing a file or folder, which Dupin has no Linux dialog for yet.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
    Program,
}

/// Always `None`, as if the user cancelled.
pub fn choose(_kind: Kind, _message: &str) -> Option<PathBuf> {
    None
}
