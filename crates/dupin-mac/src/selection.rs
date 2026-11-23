use std::ptr::{self, NonNull};
use std::sync::Once;

use objc2::rc::autoreleasepool;
use objc2_core_foundation::{CFBoolean, CFDictionary, CFGetTypeID, CFRetained, CFString, CFType, CFTypeID};
use objc2_foundation::{NSString, NSUserDefaults};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const CFDictionary<CFString, CFBoolean>) -> u8;
    static kAXTrustedCheckOptionPrompt: *const CFString;
    fn AXUIElementCreateSystemWide() -> *mut CFType;
    fn AXUIElementGetTypeID() -> CFTypeID;
    fn AXUIElementSetMessagingTimeout(element: *const CFType, seconds: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: *const CFType,
        attribute: *const CFString,
        value: *mut *mut CFType,
    ) -> i32;
}

pub const ACCESSIBILITY_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

fn is_accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

fn request_accessibility_once() {
    static REQUEST: Once = Once::new();
    REQUEST.call_once(|| {
        autoreleasepool(|_| {
            let defaults = NSUserDefaults::standardUserDefaults();
            let key = NSString::from_str("RequestedAccessibility");
            if !defaults.boolForKey(&key) {
                defaults.setBool_forKey(true, &key);
                unsafe {
                    let options =
                        CFDictionary::from_slices(&[&*kAXTrustedCheckOptionPrompt], &[CFBoolean::new(true)]);
                    AXIsProcessTrustedWithOptions(&*options);
                }
            }
        })
    });
}

pub enum Selection {
    Text(String),
    Empty,
    PermissionRequired,
}

/// Reads the focused app's selection, requesting Accessibility on first use if needed.
/// Call on a worker because the focused app can be slow to respond.
pub fn read() -> Selection {
    read_with(
        is_accessibility_trusted,
        request_accessibility_once,
        selected_text,
    )
}

fn read_with(
    trusted: impl Fn() -> bool,
    request: impl FnOnce(),
    fetch: impl FnOnce() -> Option<String>,
) -> Selection {
    if !trusted() {
        request();
        return Selection::PermissionRequired;
    }
    if let Some(text) = fetch().filter(|text| !text.trim().is_empty()) {
        Selection::Text(text)
    } else if !trusted() {
        Selection::PermissionRequired
    } else {
        Selection::Empty
    }
}

fn selected_text() -> Option<String> {
    let system = unsafe { CFRetained::from_raw(NonNull::new(AXUIElementCreateSystemWide())?) };
    // The system-wide element changes the process default, so restore it after the focus lookup.
    let focused = unsafe {
        if AXUIElementSetMessagingTimeout(&*system, 0.12) != 0 {
            return None;
        }
        let focused = attribute(&system, "AXFocusedUIElement");
        AXUIElementSetMessagingTimeout(&*system, 0.0);
        focused?
    };
    unsafe {
        if CFGetTypeID(Some(&focused)) != AXUIElementGetTypeID()
            || AXUIElementSetMessagingTimeout(&*focused, 0.12) != 0
        {
            return None;
        }
    }
    let text = attribute(&focused, "AXSelectedText")?
        .downcast::<CFString>()
        .ok()?;
    Some(text.to_string())
}

fn attribute(element: &CFType, name: &str) -> Option<CFRetained<CFType>> {
    let name = CFString::from_str(name);
    let mut value = ptr::null_mut();
    let status = unsafe { AXUIElementCopyAttributeValue(element, &*name, &mut value) };
    if status != 0 {
        return None;
    }
    Some(unsafe { CFRetained::from_raw(NonNull::new(value)?) })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn denied_access_never_reads_the_selection() {
        let requested = Cell::new(false);
        let result = read_with(
            || false,
            || requested.set(true),
            || panic!("must check access before reading"),
        );
        assert!(requested.get());
        assert!(matches!(result, Selection::PermissionRequired));
    }

    #[test]
    fn trusted_access_reads_without_prompting() {
        let result = read_with(
            || true,
            || panic!("must already have access"),
            || Some("選択 text".into()),
        );
        assert!(matches!(result, Selection::Text(text) if text == "選択 text"));
    }

    #[test]
    fn access_revoked_during_the_read_reports_the_missing_permission() {
        let trusted = Cell::new(true);
        let result = read_with(
            || trusted.get(),
            || panic!("do not prompt again after a failed read"),
            || {
                trusted.set(false);
                None
            },
        );
        assert!(matches!(result, Selection::PermissionRequired));
    }
}
