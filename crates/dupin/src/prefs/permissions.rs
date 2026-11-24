//! Settings → Permissions: each macOS permission a Dupin feature can use, what uses it, and
//! whether macOS grants it, with controls to request or manage access.

use crate::platform::privacy::{Access, CalendarAccess, Permission, PermissionStatus};
use dupin_core::Config;
use rune::{El, Icon, row};
use rune_kit::{Badge, BadgeVariant, Button, Kit};

use super::Prefs;
use super::controls::{card, note, setting};
use crate::features::{Capability, PermissionUse, permission_uses};
use crate::icons;

const ROW_ORDER: [Permission; 3] = [
    Permission::Accessibility,
    Permission::Calendars,
    Permission::Pasteboard,
];
const REQUEST_ORDER: [Capability; 3] = [
    Capability::Permission(Permission::Calendars),
    Capability::Permission(Permission::Accessibility),
    Capability::PostEvent,
];

fn uses_of(permission: Permission) -> impl Iterator<Item = &'static PermissionUse> {
    permission_uses().filter(move |usage| usage.capability.settings_permission() == permission)
}

fn used_permissions() -> impl Iterator<Item = Permission> {
    ROW_ORDER
        .into_iter()
        .filter(|&permission| uses_of(permission).next().is_some())
}

pub(super) fn available() -> bool {
    used_permissions().next().is_some()
}

/// Status reads are cheap and never prompt, so the page rereads them whenever Settings gains
/// focus, such as on returning from System Settings.
pub(super) fn read(calendar: &CalendarAccess) -> Vec<PermissionStatus> {
    used_permissions()
        .map(|permission| {
            if rune::tooling::is_headless() {
                return PermissionStatus::Access(Access::NotDetermined);
            }
            match permission {
                Permission::Accessibility | Permission::Pasteboard => {
                    PermissionStatus::Access(permission.status())
                }
                Permission::Calendars => calendar.status(),
            }
        })
        .collect()
}

fn glyph(permission: Permission) -> &'static Icon {
    match permission {
        Permission::Accessibility => &icons::ACCESSIBILITY,
        Permission::Calendars => &icons::CALENDAR,
        Permission::Pasteboard => &icons::CLIPBOARD,
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

fn request_access(calendar: &CalendarAccess, capability: Capability) {
    if cfg!(test) || rune::tooling::is_headless() {
        return;
    }
    match capability {
        Capability::Permission(Permission::Calendars) => calendar.request(),
        Capability::Permission(permission @ Permission::Accessibility) => {
            if permission.status() == Access::NotDetermined {
                crate::platform::accessibility::request_once();
            }
        }
        Capability::Permission(permission @ Permission::Pasteboard) => {
            if permission.status() == Access::NotDetermined {
                // The read that raises the prompt blocks until it is answered.
                std::thread::spawn(crate::platform::clipboard::request_access);
            }
        }
        Capability::PostEvent => {
            if !crate::platform::paste::allowed() {
                crate::platform::paste::request_once();
            }
        }
    }
}

fn request_enabled(before: &Config, after: &Config, mut request: impl FnMut(Capability)) {
    for capability in REQUEST_ORDER {
        if permission_uses().any(|usage| {
            usage.capability == capability
                && usage.request_on_enable
                && !(usage.enabled)(before)
                && (usage.enabled)(after)
        }) {
            request(capability);
        }
    }
}

pub(super) fn settings_changed(s: &mut Prefs, before: &Config) {
    request_enabled(before, &s.config, |request| {
        request_access(&s.calendar_access, request);
    });
    s.refresh_permissions();
}

fn entry_row(s: &Prefs, k: &Kit, index: usize, permission: Permission, status: PermissionStatus) -> El {
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
                    request_access(&s.calendar_access, Capability::Permission(permission));
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
    let features: Vec<_> = uses_of(permission).map(|usage| usage.label).collect();
    let mut detail = used_by(&features).unwrap_or_default();
    if !uses_of(permission).any(|usage| (usage.enabled)(&s.config)) {
        match permission {
            Permission::Accessibility => {
                detail.push_str(". Turn on a feature that uses Accessibility in General or Search.");
            }
            Permission::Calendars => detail = "Turn on Calendar search in Search to use it.".into(),
            Permission::Pasteboard => detail = "Turn on Clipboard history in Search to use it.".into(),
        }
    }
    let extensions = s.extension_consumers(permission);
    if !extensions.is_empty() {
        detail.push_str(&format!(". Extensions: {}", extensions.join(", ")));
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
    let rows = used_permissions()
        .zip(&s.permissions)
        .enumerate()
        .map(|(i, (permission, status))| entry_row(s, k, i, permission, *status))
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
        assert_eq!(
            requests.as_slice(),
            &[Capability::Permission(Permission::Calendars)]
        );

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
                Capability::Permission(Permission::Accessibility),
                Capability::PostEvent,
            ]
        );
    }

    #[test]
    fn retaining_disabling_or_rebinding_features_does_not_request_access() {
        let off = Config::default();
        let mut on = off.clone();
        on.calendar.enabled = true;
        on.snippet_paste = true;
        on.selection_hotkey = "alt+shift+space".into();
        let unexpected = |_| panic!("asked for access without enabling a feature");
        request_enabled(&off, &off, unexpected);
        request_enabled(&on, &on, unexpected);
        request_enabled(&on, &off, unexpected);
        let mut changed = on.clone();
        changed.selection_hotkey = "ctrl+shift+space".into();
        request_enabled(&on, &changed, unexpected);
        changed = off.clone();
        changed.display = dupin_core::config::DisplaySelection::Active;
        request_enabled(&off, &changed, unexpected);
    }

    #[test]
    fn simultaneous_enables_request_each_capability_once_in_order() {
        let off = Config::default();
        let mut on = off.clone();
        on.calendar.enabled = true;
        on.selection_hotkey = "alt+shift+space".into();
        on.snippet_paste = true;
        let mut requests = Vec::new();
        request_enabled(&off, &on, |request| requests.push(request));
        assert_eq!(
            requests,
            [
                Capability::Permission(Permission::Calendars),
                Capability::Permission(Permission::Accessibility),
                Capability::PostEvent,
            ]
        );
    }

    #[test]
    fn permission_rows_list_manifest_features_in_display_order() {
        let rows: Vec<_> = used_permissions()
            .map(|permission| {
                let features: Vec<_> = uses_of(permission).map(|usage| usage.label).collect();
                (permission, features)
            })
            .collect();
        assert_eq!(
            rows,
            [
                (
                    Permission::Accessibility,
                    vec![
                        "extensions that paste or arrange windows",
                        "focused window display",
                        "selected-text search",
                        "snippet pasting",
                    ],
                ),
                (
                    Permission::Calendars,
                    vec!["extensions that read calendars", "calendar search"],
                ),
                (
                    Permission::Pasteboard,
                    vec!["extensions that read the clipboard", "clipboard history"],
                ),
            ]
        );
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
