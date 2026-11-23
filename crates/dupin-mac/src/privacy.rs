//! macOS privacy permissions (TCC) that Dupin's features can use: whether each is granted,
//! read without prompting, and the System Settings pane that changes it. Asking for access
//! belongs to the feature that needs it, when it is used (#12), never to this module.

use objc2::msg_send;
use objc2::runtime::AnyClass;
use objc2_foundation::{NSBundle, NSString};

use crate::accessibility;

const CALENDARS_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Accessibility,
    Calendars,
}

/// What macOS reports for a permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Allowed,
    /// Accessibility reports only trusted or not, so "never asked" and "turned off" read alike.
    NotAllowed,
    NotDetermined,
    Denied,
    /// A configuration profile or parental controls decide, not the user.
    Restricted,
    /// Calendars: Dupin may add events but not read them.
    WriteOnly,
    Unknown,
}

impl Permission {
    /// The name System Settings lists it under.
    pub fn name(self) -> &'static str {
        match self {
            Permission::Accessibility => "Accessibility",
            Permission::Calendars => "Calendars",
        }
    }

    /// The current state, without asking for access.
    pub fn status(self) -> Access {
        match self {
            Permission::Accessibility if accessibility::is_trusted() => Access::Allowed,
            Permission::Accessibility => Access::NotAllowed,
            Permission::Calendars => calendars(),
        }
    }

    fn settings_url(self) -> &'static str {
        match self {
            Permission::Accessibility => accessibility::SETTINGS_URL,
            Permission::Calendars => CALENDARS_SETTINGS_URL,
        }
    }

    /// Open Privacy & Security at this permission's list.
    pub fn open_settings(self) -> bool {
        crate::workspace::open_url(self.settings_url())
    }
}

/// EventKit is loaded on first use rather than linked, so a launch that never looks at
/// Calendars doesn't load it.
fn calendars() -> Access {
    let class = AnyClass::get(c"EKEventStore").or_else(|| {
        NSBundle::bundleWithPath(&NSString::from_str(
            "/System/Library/Frameworks/EventKit.framework",
        ))
        // SAFETY: A system framework, loaded only for its Objective-C classes.
        .filter(|bundle| unsafe { bundle.load() })
        .and_then(|_| AnyClass::get(c"EKEventStore"))
    });
    let Some(class) = class else {
        return Access::Unknown;
    };
    const EK_ENTITY_TYPE_EVENT: usize = 0;
    // SAFETY: `+[EKEventStore authorizationStatusForEntityType:]` takes an `EKEntityType`
    // (NSUInteger) and returns an `EKAuthorizationStatus` (NSInteger); it only reads the status.
    let raw: isize = unsafe { msg_send![class, authorizationStatusForEntityType: EK_ENTITY_TYPE_EVENT] };
    calendar_access(raw)
}

/// `EKAuthorizationStatus`; macOS 14 renamed `authorized` (3) to `fullAccess` and added
/// `writeOnly` (4).
fn calendar_access(raw: isize) -> Access {
    match raw {
        0 => Access::NotDetermined,
        1 => Access::Restricted,
        2 => Access::Denied,
        3 => Access::Allowed,
        4 => Access::WriteOnly,
        _ => Access::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_statuses_map_to_access() {
        assert_eq!(calendar_access(0), Access::NotDetermined);
        assert_eq!(calendar_access(2), Access::Denied);
        assert_eq!(calendar_access(3), Access::Allowed);
        assert_eq!(calendar_access(4), Access::WriteOnly);
        assert_eq!(calendar_access(9), Access::Unknown);
    }

    #[test]
    fn each_permission_links_to_its_privacy_pane() {
        for (permission, anchor) in [
            (Permission::Accessibility, "Privacy_Accessibility"),
            (Permission::Calendars, "Privacy_Calendars"),
        ] {
            let url = permission.settings_url();
            assert!(url.starts_with("x-apple.systempreferences:com.apple.preference.security?"));
            assert!(url.ends_with(anchor), "{url}");
        }
    }
}
