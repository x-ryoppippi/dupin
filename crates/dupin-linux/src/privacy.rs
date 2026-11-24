//! The permissions macOS asks for. Linux grants none of them through Dupin, so each reads as
//! restricted: Settings shows it, and the extension broker never offers a prompt.

use dupin_host_services::{OsAccess, OsPermission};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Accessibility,
    Calendars,
    Pasteboard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Allowed,
    NotAllowed,
    NotDetermined,
    Denied,
    Restricted,
    WriteOnly,
    Unknown,
}

impl Permission {
    pub fn name(self) -> &'static str {
        match self {
            Permission::Accessibility => "Accessibility",
            Permission::Calendars => "Calendars",
            Permission::Pasteboard => "Paste from Other Apps",
        }
    }

    pub const fn of(permission: OsPermission) -> Self {
        match permission {
            OsPermission::Accessibility => Self::Accessibility,
            OsPermission::Calendars => Self::Calendars,
            OsPermission::Pasteboard => Self::Pasteboard,
        }
    }

    pub fn status(self) -> Access {
        Access::Restricted
    }

    /// Linux has no settings pane for these, so there is no address to open.
    pub fn settings_url(self) -> &'static str {
        ""
    }

    /// Linux has no settings pane for these.
    pub fn open_settings(self) -> bool {
        false
    }
}

impl Access {
    pub fn os_access(self) -> OsAccess {
        match self {
            Access::Allowed => OsAccess::Allowed,
            Access::NotDetermined => OsAccess::NotDetermined,
            Access::Restricted => OsAccess::Restricted,
            Access::NotAllowed | Access::Denied | Access::WriteOnly | Access::Unknown => OsAccess::Denied,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionStatus {
    Access(Access),
    Requesting,
    RequestIncomplete,
    Unavailable,
}

impl PermissionStatus {
    pub fn os_access(self) -> OsAccess {
        match self {
            PermissionStatus::Access(access) => access.os_access(),
            PermissionStatus::Requesting | PermissionStatus::RequestIncomplete => OsAccess::NotDetermined,
            PermissionStatus::Unavailable => OsAccess::Restricted,
        }
    }
}

/// Calendar access, which Linux can't grant: no calendar store is read.
#[derive(Clone, Default)]
pub struct CalendarAccess(());

impl CalendarAccess {
    pub fn new(_changed: impl Fn() + Send + Sync + 'static) -> Self {
        Self(())
    }

    pub fn status(&self) -> PermissionStatus {
        PermissionStatus::Unavailable
    }

    pub fn request_on_use(&self) -> PermissionStatus {
        PermissionStatus::Unavailable
    }

    pub fn request(&self) {}
}
