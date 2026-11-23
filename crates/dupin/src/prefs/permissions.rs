//! Settings → Permissions: each macOS permission a Dupin feature can use, what uses it, and
//! whether macOS grants it, with a way to its System Settings pane. The page only reads status;
//! a feature asks for access itself, when it is used (#12).

use dupin_mac::privacy::{Access, Permission};
use rune::{El, Icon, row};
use rune_kit::{Badge, BadgeVariant, Button, Kit};

use super::{Prefs, card, setting};
use crate::icons;

/// A permission and the features that use it. A feature that needs a permission names itself
/// here, so the page explains why Dupin may ask.
pub(super) struct Entry {
    pub permission: Permission,
    pub used_by: &'static [&'static str],
}

pub(super) const ENTRIES: &[Entry] = &[
    Entry {
        permission: Permission::Accessibility,
        used_by: &[
            "focused window display",
            "selected-text search",
            "snippet pasting",
        ],
    },
    Entry {
        permission: Permission::Calendars,
        used_by: &["calendar search"],
    },
];

fn used_entries() -> impl Iterator<Item = &'static Entry> {
    ENTRIES.iter().filter(|entry| !entry.used_by.is_empty())
}

pub(super) fn available() -> bool {
    used_entries().next().is_some()
}

/// Status reads are cheap and never prompt, so the page rereads them whenever Settings gains
/// focus, such as on returning from System Settings.
pub(super) fn read() -> Vec<Access> {
    used_entries().map(|entry| entry.permission.status()).collect()
}

fn glyph(permission: Permission) -> &'static Icon {
    match permission {
        Permission::Accessibility => &icons::ACCESSIBILITY,
        Permission::Calendars => &icons::CALENDAR,
    }
}

fn status_title(access: Access) -> &'static str {
    match access {
        Access::Allowed => "Allowed",
        Access::NotAllowed => "Not allowed",
        Access::NotDetermined => "Not asked yet",
        Access::Denied => "Denied",
        Access::Restricted => "Restricted",
        Access::WriteOnly => "Add only",
        Access::Unknown => "Unknown",
    }
}

fn used_by(features: &[&str]) -> Option<String> {
    match features {
        [] => None,
        [one] => Some(format!("Used by {one}")),
        [rest @ .., last] => Some(format!("Used by {} and {last}", rest.join(", "))),
    }
}

fn entry_row(k: &Kit, index: usize, entry: &Entry, access: Access) -> El {
    let permission = entry.permission;
    let badge = Badge::new(status_title(access))
        .variant(if access == Access::Allowed {
            BadgeVariant::Accent
        } else {
            BadgeVariant::Neutral
        })
        .build(k)
        .key(format!("permission-{index}-status"));
    let open = Button::new("Open System Settings")
        .on_press(move |_: &mut Prefs| {
            if !permission.open_settings() {
                crate::log::warn(&format!("couldn't open the {} settings", permission.name()));
            }
        })
        .build(k)
        .key(format!("permission-{index}-open"));
    setting(
        k,
        glyph(permission),
        permission.name(),
        used_by(entry.used_by).as_deref(),
        row().gap(12.0).center_y().child(badge).child(open),
    )
}

pub(super) fn page(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let rows = used_entries()
        .zip(&s.permissions)
        .enumerate()
        .map(|(i, (entry, access))| entry_row(k, i, entry, *access))
        .collect();
    vec![
        card(k, hair, rows),
        super::note(
            k,
            "Dupin asks for a permission only when you use a feature that needs it. File search reads \
             the Spotlight index and needs none.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_read_as_a_list() {
        assert_eq!(used_by(&[]), None);
        assert_eq!(
            used_by(&["snippet paste"]).as_deref(),
            Some("Used by snippet paste")
        );
        assert_eq!(
            used_by(&["snippet paste", "selected text", "the active display"]).as_deref(),
            Some("Used by snippet paste, selected text and the active display")
        );
    }
}
