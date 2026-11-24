//! The focused app's selected text, which Dupin doesn't read on Linux yet.

pub enum Selection {
    Text(String),
    Empty,
    PermissionRequired,
}

pub fn read(_on_untrusted: impl FnOnce()) -> Selection {
    Selection::Empty
}
