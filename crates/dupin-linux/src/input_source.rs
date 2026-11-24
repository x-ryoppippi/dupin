//! The keyboard input source the panel types with. Linux has no system-wide input source API
//! (input methods such as fcitx and IBus each have their own), so no source is ever selected.

/// Which input source the panel types with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Ascii,
    Id(String),
}

/// A source to put back when the panel hides. None exists on Linux.
pub enum Previous {}

impl Previous {
    pub fn restore(self) {
        match self {}
    }
}

/// Always `None`: there is no source to switch to.
pub fn select(_: &Target) -> Option<Previous> {
    None
}
