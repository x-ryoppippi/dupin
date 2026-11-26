//! Where Dupin keeps what it learns about its user: `~/Library/Application Support/Dupin`, and
//! the data directory beside it. Usage history lives in the `usage` storage namespace, which
//! [`crate::sync`] loads and saves.

use std::path::PathBuf;

/// `~/Library/Application Support/Dupin`, where Dupin keeps what it learns about its user.
pub fn support_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support/Dupin"))
}

pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(|base| PathBuf::from(base).join("dupin"))
        .or_else(support_dir)
}

/// Usage history from before it moved into encrypted storage, read once and then removed.
pub const LEGACY_HISTORY: &str = "history.json";

pub fn legacy_history_path() -> Option<PathBuf> {
    support_dir().map(|dir| dir.join(LEGACY_HISTORY))
}

/// The usage history namespace's directory, whose presence means Dupin was used before.
pub fn usage_dir() -> Option<PathBuf> {
    crate::extensions::access::storage_path().map(|dir| dir.join("usage"))
}
