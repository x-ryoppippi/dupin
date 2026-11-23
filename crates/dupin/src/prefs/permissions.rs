//! Settings → Permissions: each macOS permission a Dupin feature can use, what uses it, and
//! whether macOS grants it, with controls to request or manage access.

use dupin_core::Config;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    Permission(Permission),
    PostEvent,
}

fn request_access(calendar: &CalendarAccess, request: Request) {
    if cfg!(test) || rune::tooling::is_headless() {
        return;
    }
    match request {
        Request::Permission(Permission::Calendars) => calendar.request(),
        Request::Permission(permission @ Permission::Accessibility) => {
            if permission.status() == Access::NotDetermined {
                dupin_mac::accessibility::request_once();
            }
        }
        Request::PostEvent => {
            if !dupin_mac::paste::allowed() {
                dupin_mac::paste::request_once();
            }
        }
    }
}

fn request_enabled(before: &Config, after: &Config, mut request: impl FnMut(Request)) {
    if !before.calendar.enabled && after.calendar.enabled {
        request(Request::Permission(Permission::Calendars));
    }
    if (!before.window_management && after.window_management)
        || (before.selection_hotkey.trim().is_empty() && !after.selection_hotkey.trim().is_empty())
    {
        request(Request::Permission(Permission::Accessibility));
    }
    if !before.snippet_paste && after.snippet_paste {
        request(Request::PostEvent);
    }
}

pub(super) fn settings_changed(s: &mut Prefs, before: &Config) {
    request_enabled(before, &s.config, |request| {
        request_access(&s.calendar_access, request);
    });
    s.refresh_permissions();
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
                    request_access(&s.calendar_access, Request::Permission(permission));
                    s.refresh_permissions();
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
        && s.config.selection_hotkey.trim().is_empty()
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
            "Dupin can ask for access when you enable or first use a feature that needs it, \
             or when you choose Request Access here. File search needs no permission.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabling_each_permission_dependent_setting_requests_its_grant() {
        let off = Config::default();
        let mut on = off.clone();
        let mut requests = Vec::new();
        on.calendar.enabled = true;
        request_enabled(&off, &on, |request| requests.push(request));
        assert_eq!(requests.as_slice(), &[Request::Permission(Permission::Calendars)]);

        on = off.clone();
        on.window_management = true;
        request_enabled(&off, &on, |request| requests.push(request));
        on = off.clone();
        on.selection_hotkey = "alt+shift+space".into();
        let blank = Config {
            selection_hotkey: "  ".into(),
            ..off.clone()
        };
        request_enabled(&blank, &on, |request| requests.push(request));
        on = off.clone();
        on.snippet_paste = true;
        request_enabled(&off, &on, |request| requests.push(request));
        assert_eq!(
            &requests[1..],
            [
                Request::Permission(Permission::Accessibility),
                Request::Permission(Permission::Accessibility),
                Request::PostEvent,
            ]
        );
    }

    #[test]
    fn retaining_disabling_or_rebinding_features_does_not_request_access() {
        let off = Config::default();
        let mut on = off.clone();
        on.calendar.enabled = true;
        on.window_management = true;
        on.snippet_paste = true;
        on.selection_hotkey = "alt+shift+space".into();
        let unexpected = |_| panic!("asked for access without enabling a feature");
        request_enabled(&off, &off, unexpected);
        request_enabled(&on, &on, unexpected);
        request_enabled(&on, &off, unexpected);
        let mut changed = on.clone();
        changed.selection_hotkey = "ctrl+shift+space".into();
        request_enabled(&on, &changed, unexpected);
    }

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
