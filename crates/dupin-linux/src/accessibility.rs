//! macOS Accessibility, which Linux has no counterpart to grant.

/// Linux has no settings pane to open for it.
pub const SETTINGS_URL: &str = "";

pub fn is_trusted() -> bool {
    false
}

/// Linux has no prompt to ask, so this never requests one.
pub fn request_once() -> bool {
    false
}
