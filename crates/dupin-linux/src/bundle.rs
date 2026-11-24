//! The running app's own install: a bare executable, with no Info.plist and no update feed.

use std::path::PathBuf;

/// The executable's directory.
pub fn path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_default()
}

/// Info.plist values exist only in a macOS bundle.
pub fn info_string(_: &str) -> Option<String> {
    None
}
