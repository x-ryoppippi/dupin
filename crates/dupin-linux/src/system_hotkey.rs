//! The desktop's own shortcuts. Dupin doesn't read them on Linux yet, so no chord is reported
//! as taken.

use rune::Shortcut;

pub fn owner(_shortcut: &Shortcut) -> Option<&'static str> {
    None
}
