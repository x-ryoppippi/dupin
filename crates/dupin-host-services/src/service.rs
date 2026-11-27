//! What the OS-backed services share: the OS access they read without prompting, and the
//! structured error a guest receives.

use std::fmt;

use dupin_broker::{OsAccess, OsPermission};
use serde::{Serialize, Serializer};

/// Why an OS-backed operation didn't run. It serializes as `{ "code": "needs-permission",
/// "permission": "calendars", "message": "…" }`, so a guest can branch on `code` and the host
/// can turn `needs-permission` into a Request Access row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// macOS hasn't been asked yet. Only the app asks, from a deliberate action such as Request
    /// Access, never on a guest call.
    NeedsPermission(OsPermission),
    PermissionDenied(OsPermission),
    PermissionRestricted(OsPermission),
    NeedsActivation,
    Invalid(String),
    OutOfScope(String),
    Unsupported(String),
    Failed(String),
}

impl ServiceError {
    /// The error for OS access that isn't [`OsAccess::Allowed`].
    pub fn from_access(permission: OsPermission, access: OsAccess) -> Option<Self> {
        match access {
            OsAccess::Allowed => None,
            OsAccess::NotDetermined => Some(Self::NeedsPermission(permission)),
            OsAccess::Denied => Some(Self::PermissionDenied(permission)),
            OsAccess::Restricted => Some(Self::PermissionRestricted(permission)),
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::NeedsPermission(_) => "needs-permission",
            Self::PermissionDenied(_) => "permission-denied",
            Self::PermissionRestricted(_) => "permission-restricted",
            Self::NeedsActivation => "needs-activation",
            Self::Invalid(_) => "invalid",
            Self::OutOfScope(_) => "out-of-scope",
            Self::Unsupported(_) => "unsupported",
            Self::Failed(_) => "failed",
        }
    }

    pub fn permission(&self) -> Option<OsPermission> {
        match self {
            Self::NeedsPermission(permission)
            | Self::PermissionDenied(permission)
            | Self::PermissionRestricted(permission) => Some(*permission),
            _ => None,
        }
    }
}

/// The identifier a guest sees for an OS permission.
pub fn permission_id(permission: OsPermission) -> &'static str {
    match permission {
        OsPermission::Calendars => "calendars",
        OsPermission::Pasteboard => "pasteboard",
        OsPermission::Accessibility => "accessibility",
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedsPermission(permission) => write!(
                f,
                "Dupin needs {} access. Choose Request Access in Settings → Permissions.",
                permission.name()
            ),
            Self::PermissionDenied(permission) => write!(
                f,
                "macOS denied {} access for Dupin. Change it in System Settings → Privacy & Security.",
                permission.name()
            ),
            Self::PermissionRestricted(permission) => {
                write!(f, "macOS restricts {} access for Dupin.", permission.name())
            }
            Self::NeedsActivation => f.write_str("This needs you to choose the action."),
            Self::Invalid(reason) => write!(f, "The request is invalid: {reason}."),
            Self::OutOfScope(reason) => write!(f, "Not allowed: {reason}."),
            Self::Unsupported(reason) => write!(f, "Not supported: {reason}."),
            Self::Failed(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for ServiceError {}

impl Serialize for ServiceError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let permission = self.permission();
        let mut state = serializer.serialize_struct("ServiceError", 2 + usize::from(permission.is_some()))?;
        state.serialize_field("code", self.code())?;
        if let Some(permission) = permission {
            state.serialize_field("permission", permission_id(permission))?;
        }
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

/// `text` cut to at most `max` bytes on a character boundary.
pub(crate) fn bounded(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}
