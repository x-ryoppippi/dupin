//! Moving, copying, renaming and compressing files from the panel, which Dupin doesn't do on
//! Linux yet.

use std::io;
use std::path::{Path, PathBuf};

const UNSUPPORTED: &str = "File operations aren't supported on Linux yet.";

#[derive(Clone, Default)]
pub struct FileTransfers(());

impl FileTransfers {
    /// Nothing is staged on Linux, so nothing is left to clean up.
    pub fn recover_abandoned_copies(&self) -> io::Result<()> {
        Ok(())
    }

    pub fn move_to(&self, _path: &Path, _folder: &Path) -> Result<PathBuf, String> {
        Err(UNSUPPORTED.into())
    }

    pub fn copy_to(&self, _path: &Path, _folder: &Path) -> Result<PathBuf, String> {
        Err(UNSUPPORTED.into())
    }

    pub fn duplicate(&self, _path: &Path) -> Result<PathBuf, String> {
        Err(UNSUPPORTED.into())
    }
}

pub fn rename(_path: &Path, _name: &str) -> Result<PathBuf, String> {
    Err(UNSUPPORTED.into())
}

pub fn compress(_path: &Path) -> Result<PathBuf, String> {
    Err(UNSUPPORTED.into())
}

/// Whether a rename keeps an extension to protect: every entry but a folder has one. Linux has
/// no packages, folders that read as files.
pub fn has_extension(path: &Path) -> bool {
    !path.is_dir()
}
