//! Settings → Permissions: each macOS permission a Dupin feature can use, what uses it, and
//! whether macOS grants it, with controls to request or manage access.

use dupin_mac::privacy::{Access, CalendarAccess, Permission, PermissionStatus};
use rune::{El, Icon, row};
use rune_kit::{Badge, BadgeVariant, Button, Kit};

use super::Prefs;
use super::controls::{card, note, setting};
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
            "window management",
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
pub(super) fn read(calendar: &CalendarAccess) -> Vec<PermissionStatus> {
    used_entries()
        .map(|entry| {
            if rune::tooling::is_headless() {
                return PermissionStatus::Access(Access::NotDetermined);
            }
            match entry.permission {
                Permission::Accessibility => PermissionStatus::Access(entry.permission.status()),
                Permission::Calendars => calendar.status(),
            }
        })
        .collect()
}

fn glyph(permission: Permission) -> &'static Icon {
    match permission {
        Permission::Accessibility => &icons::ACCESSIBILITY,
        Permission::Calendars => &icons::CALENDAR,
    }
}

fn status_title(status: PermissionStatus) -> &'static str {
    let access = match status {
        PermissionStatus::Access(access) => access,
        PermissionStatus::Requesting => return "Waiting for macOS",
        PermissionStatus::RequestIncomplete => return "Request incomplete",
        PermissionStatus::Unavailable => return "Unavailable",
    };
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

fn entry_row(s: &Prefs, k: &Kit, index: usize, entry: &Entry, status: PermissionStatus) -> El {
    let permission = entry.permission;
    let badge = Badge::new(status_title(status))
        .variant(if status == PermissionStatus::Access(Access::Allowed) {
            BadgeVariant::Accent
        } else {
            BadgeVariant::Neutral
        })
        .build(k)
        .key(format!("permission-{index}-status"));
    let action = match status {
        PermissionStatus::Access(Access::NotDetermined) | PermissionStatus::RequestIncomplete => {
            Button::new("Request Access")
                .on_press(move |s: &mut Prefs| {
                    if !rune::tooling::is_headless() {
                        match permission {
                            Permission::Accessibility => {
                                if permission.status() == Access::NotDetermined {
                                    dupin_mac::accessibility::request_once();
                                }
                            }
                            Permission::Calendars => s.calendar_access.request(),
                        }
                        s.refresh_permissions();
                    }
                })
                .build(k)
                .key(format!("permission-{index}-request"))
        }
        PermissionStatus::Requesting | PermissionStatus::Unavailable => Button::new("Request Access")
            .disabled(true)
            .build(k)
            .key(format!("permission-{index}-request")),
        PermissionStatus::Access(_) => Button::new("Open System Settings")
            .on_press(move |_: &mut Prefs| {
                if !permission.open_settings() {
                    crate::log::warn(&format!("couldn't open the {} settings", permission.name()));
                }
            })
            .build(k)
            .key(format!("permission-{index}-open")),
    };
    let mut detail = used_by(entry.used_by).unwrap_or_default();
    if permission == Permission::Calendars && !s.config.calendar.enabled {
        detail = "Turn on Calendar search in Search to use it.".into();
    }
    if permission == Permission::Accessibility
        && s.config.display != dupin_core::config::DisplaySelection::Active
        && s.config.selection_hotkey.is_empty()
        && !s.config.snippet_paste
        && !s.config.window_management
    {
        detail.push_str(". Turn on a feature that uses Accessibility in General or Search.");
    }
    if status == PermissionStatus::Unavailable {
        detail.push_str(" Open the packaged Dupin.app to grant access.");
    }
    setting(
        k,
        glyph(permission),
        permission.name(),
        Some(&detail),
        row().gap(12.0).center_y().child(badge).child(action),
    )
}

pub(super) fn page(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let rows = used_entries()
        .zip(&s.permissions)
        .enumerate()
        .map(|(i, (entry, status))| entry_row(s, k, i, entry, *status))
        .collect();
    vec![
        card(k, hair, rows),
        note(
            k,
            "Dupin only asks for access when you request it here or use a feature that needs it. \
             File search reads the Spotlight index and needs no permission.",
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
