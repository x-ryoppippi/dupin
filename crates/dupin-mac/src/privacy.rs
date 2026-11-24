//! Read macOS privacy grants without prompting, and link to the panes that change them.

use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::{MainThreadMarker, rc::Retained, runtime::Bool};
use objc2_app_kit::NSApplication;
use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};
use objc2_foundation::{NSBundle, NSError, NSOperationQueue, NSString};

use dupin_host_services::{OsAccess, OsPermission};

use crate::accessibility;

const PRIVACY_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.security";
const CALENDARS_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Accessibility,
    Calendars,
    /// Pasting from other apps without the system's prompt: the pasteboard access policy.
    Pasteboard,
}

/// What macOS reports for a permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Allowed,
    /// Accessibility is untrusted after Dupin previously requested access, or the pasteboard
    /// asks on every read.
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
            Permission::Pasteboard => "Paste from Other Apps",
        }
    }

    /// The row in Settings → Permissions an extension capability's OS permission joins.
    pub const fn of(permission: OsPermission) -> Self {
        match permission {
            OsPermission::Calendars => Self::Calendars,
            OsPermission::Pasteboard => Self::Pasteboard,
        }
    }

    /// The current state, without asking for access.
    pub fn status(self) -> Access {
        match self {
            Permission::Accessibility if accessibility::is_trusted() => Access::Allowed,
            Permission::Accessibility if accessibility::was_requested() => Access::NotAllowed,
            Permission::Accessibility => Access::NotDetermined,
            Permission::Calendars => calendar_access(calendar_authorization().0),
            Permission::Pasteboard => crate::clipboard::policy_access(),
        }
    }

    pub(crate) fn settings_url(self) -> &'static str {
        match self {
            Permission::Accessibility => accessibility::SETTINGS_URL,
            Permission::Calendars => CALENDARS_SETTINGS_URL,
            // The per-app pasteboard list has no documented anchor.
            Permission::Pasteboard => PRIVACY_SETTINGS_URL,
        }
    }

    /// Open Privacy & Security at this permission's list.
    pub fn open_settings(self) -> bool {
        crate::workspace::open_url(self.settings_url())
    }
}

impl Access {
    /// What the extension broker is told. Anything short of allowed that only System
    /// Settings can change counts as denied, so the broker never offers a prompt for it.
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
    /// What the extension broker is told. A request in flight or unfinished is still
    /// undetermined; an unavailable request can't be granted from this build.
    pub fn os_access(self) -> OsAccess {
        match self {
            PermissionStatus::Access(access) => access.os_access(),
            PermissionStatus::Requesting | PermissionStatus::RequestIncomplete => OsAccess::NotDetermined,
            PermissionStatus::Unavailable => OsAccess::Restricted,
        }
    }
}

#[derive(Clone, Copy, Default)]
enum Prompt {
    #[default]
    Ready,
    Pending,
    Finished,
    Unavailable,
}

impl Prompt {
    fn status(self, access: Access) -> PermissionStatus {
        if access != Access::NotDetermined {
            return PermissionStatus::Access(access);
        }
        match self {
            Self::Ready => PermissionStatus::Access(access),
            Self::Pending => PermissionStatus::Requesting,
            Self::Finished => PermissionStatus::RequestIncomplete,
            Self::Unavailable => PermissionStatus::Unavailable,
        }
    }
}

struct CalendarAccessInner {
    prompt: Mutex<Prompt>,
    changed: Box<dyn Fn() + Send + Sync>,
}

/// Coordinates the Calendar prompt across the launcher and Settings.
#[derive(Clone)]
pub struct CalendarAccess(Arc<CalendarAccessInner>);

impl CalendarAccess {
    pub fn new(changed: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(CalendarAccessInner {
            prompt: Mutex::default(),
            changed: Box::new(changed),
        }))
    }

    pub fn status(&self) -> PermissionStatus {
        self.0
            .prompt
            .lock()
            .unwrap()
            .status(Permission::Calendars.status())
    }

    /// Request full access after an explicit press in Settings.
    pub fn request(&self) {
        self.start(true, || Permission::Calendars.status(), queue_calendar_request);
    }

    pub(crate) fn request_on_use(&self) -> PermissionStatus {
        self.start(false, || Permission::Calendars.status(), queue_calendar_request)
    }

    fn start(
        &self,
        explicit: bool,
        status: impl FnOnce() -> Access,
        request: impl FnOnce(Self) -> bool,
    ) -> PermissionStatus {
        let mut prompt = self.0.prompt.lock().unwrap();
        let status = prompt.status(status());
        if status != PermissionStatus::Access(Access::NotDetermined)
            && !(explicit && status == PermissionStatus::RequestIncomplete)
        {
            return status;
        }
        *prompt = Prompt::Pending;
        drop(prompt);
        if request(self.clone()) {
            PermissionStatus::Requesting
        } else {
            self.finish(Prompt::Unavailable);
            PermissionStatus::Unavailable
        }
    }

    fn finish(&self, prompt: Prompt) {
        *self.0.prompt.lock().unwrap() = prompt;
        (self.0.changed)();
    }
}

fn queue_calendar_request(access: CalendarAccess) -> bool {
    if NSBundle::mainBundle()
        .objectForInfoDictionaryKey(&NSString::from_str("NSCalendarsFullAccessUsageDescription"))
        .is_none()
    {
        return false;
    }

    let request = RcBlock::new(move || {
        if Permission::Calendars.status() != Access::NotDetermined {
            access.finish(Prompt::Finished);
            return;
        }
        // The launcher panel can be key while another app is active.
        let main = MainThreadMarker::new().expect("NSOperationQueue.mainQueue runs on the main thread");
        let app = NSApplication::sharedApplication(main);
        if !app.isActive() {
            app.activate();
        }
        let access = access.clone();
        let completion = RcBlock::new(move |_: Bool, _: *mut NSError| {
            access.finish(Prompt::Finished);
        });
        thread_local! {
            static REQUEST_STORE: Retained<EKEventStore> = unsafe { EKEventStore::new() };
        }
        // EventKit copies the completion block. Keep the store on the main thread for its lifetime.
        REQUEST_STORE.with(|store| unsafe {
            store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&completion));
        });
    });
    // SAFETY: The copied block only captures thread-safe state.
    unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&request) };
    true
}

pub(crate) fn calendar_authorization() -> EKAuthorizationStatus {
    // SAFETY: This status query neither requests access nor reads calendar events.
    unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) }
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
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn calendar_requests_are_shared_and_only_an_explicit_press_retries() {
        let notifications = Arc::new(AtomicUsize::new(0));
        let access = CalendarAccess::new({
            let notifications = notifications.clone();
            move || {
                notifications.fetch_add(1, Ordering::Relaxed);
            }
        });
        let status = Cell::new(Access::NotDetermined);
        let pending = Cell::new(None);
        let request = |access| {
            assert!(pending.replace(Some(access)).is_none());
            true
        };

        assert_eq!(
            access.start(false, || status.get(), request),
            PermissionStatus::Requesting
        );
        assert_eq!(
            access.clone().start(true, || status.get(), request),
            PermissionStatus::Requesting
        );
        pending.take().unwrap().finish(Prompt::Finished);
        assert_eq!(notifications.load(Ordering::Relaxed), 1);
        assert_eq!(
            access.start(false, || status.get(), |_| panic!("search retried the request")),
            PermissionStatus::RequestIncomplete
        );
        assert_eq!(
            access.start(true, || status.get(), request),
            PermissionStatus::Requesting
        );
        status.set(Access::Allowed);
        pending.take().unwrap().finish(Prompt::Finished);
        assert_eq!(notifications.load(Ordering::Relaxed), 2);
        assert_eq!(
            access.start(false, || status.get(), |_| panic!("asked after full access")),
            PermissionStatus::Access(Access::Allowed)
        );
    }

    #[test]
    fn settled_calendar_permissions_never_start_a_request() {
        let access = CalendarAccess::new(|| {});
        for status in [
            Access::Allowed,
            Access::Denied,
            Access::Restricted,
            Access::WriteOnly,
        ] {
            for explicit in [false, true] {
                assert_eq!(
                    access.start(explicit, || status, |_| panic!("asked after macOS decided")),
                    PermissionStatus::Access(status)
                );
            }
        }
    }

    #[test]
    fn calendar_request_unavailable_does_not_leave_a_pending_prompt() {
        let access = CalendarAccess::new(|| {});
        assert_eq!(
            access.start(false, || Access::NotDetermined, |_| false),
            PermissionStatus::Unavailable
        );
        assert_eq!(
            access.start(
                true,
                || Access::NotDetermined,
                |_| panic!("retried unavailable request")
            ),
            PermissionStatus::Unavailable
        );
    }

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
            (Permission::Calendars, "?Privacy_Calendars"),
            (Permission::Pasteboard, "com.apple.preference.security"),
        ] {
            let url = permission.settings_url();
            assert!(url.starts_with("x-apple.systempreferences:com.apple.preference.security"));
            assert!(url.ends_with(anchor), "{url}");
        }
    }

    #[test]
    fn extension_permissions_join_their_settings_rows() {
        assert_eq!(Permission::of(OsPermission::Calendars), Permission::Calendars);
        assert_eq!(Permission::of(OsPermission::Pasteboard), Permission::Pasteboard);
        for permission in OsPermission::ALL {
            assert_eq!(Permission::of(permission).name(), permission.name());
        }
    }

    #[test]
    fn only_allowed_access_reaches_the_broker_as_allowed() {
        for (status, expected) in [
            (PermissionStatus::Access(Access::Allowed), OsAccess::Allowed),
            (
                PermissionStatus::Access(Access::NotDetermined),
                OsAccess::NotDetermined,
            ),
            (PermissionStatus::Requesting, OsAccess::NotDetermined),
            (PermissionStatus::RequestIncomplete, OsAccess::NotDetermined),
            (PermissionStatus::Access(Access::Restricted), OsAccess::Restricted),
            (PermissionStatus::Unavailable, OsAccess::Restricted),
            (PermissionStatus::Access(Access::Denied), OsAccess::Denied),
            (PermissionStatus::Access(Access::WriteOnly), OsAccess::Denied),
            (PermissionStatus::Access(Access::NotAllowed), OsAccess::Denied),
            (PermissionStatus::Access(Access::Unknown), OsAccess::Denied),
        ] {
            assert_eq!(status.os_access(), expected, "{status:?}");
        }
    }
}
