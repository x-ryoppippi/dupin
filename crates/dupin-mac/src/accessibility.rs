//! Shared access to the macOS Accessibility API.

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
    fn AXUIElementGetTypeID() -> CFTypeID;
    fn AXUIElementCreateApplication(pid: i32) -> *mut CFType;
    fn AXUIElementCreateSystemWide() -> *mut CFType;
    fn AXUIElementSetMessagingTimeout(element: &CFType, timeout: f32) -> i32;
    fn AXUIElementCopyAttributeValue(element: &CFType, attribute: &CFString, value: &mut *mut CFType) -> i32;
}

pub const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

/// Reads the current grant without prompting.
pub fn is_trusted() -> bool {
    // SAFETY: A status query with no arguments.
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Requests the macOS prompt at most once per user. Returns whether this call requested it.
pub fn request_once() -> bool {
    static REQUEST: Once = Once::new();
    let mut requested = false;
    REQUEST.call_once(|| {
        autoreleasepool(|_| {
            let defaults = NSUserDefaults::standardUserDefaults();
            let key = NSString::from_str("RequestedAccessibility");
            if defaults.boolForKey(&key) {
                return;
            }
            defaults.setBool_forKey(true, &key);
            // SAFETY: The options dictionary outlives the call.
            unsafe {
                let options =
                    CFDictionary::from_slices(&[&*kAXTrustedCheckOptionPrompt], &[CFBoolean::new(true)]);
                AXIsProcessTrustedWithOptions(&*options);
            }
            requested = true;
        });
    });
    requested
}

pub(crate) fn application(pid: i32) -> Option<CFRetained<CFType>> {
    // SAFETY: Create returns an owned AXUIElement reference for this PID.
    Some(unsafe { CFRetained::from_raw(NonNull::new(AXUIElementCreateApplication(pid))?) })
}

pub(crate) fn system_wide() -> Option<CFRetained<CFType>> {
    // SAFETY: Create returns an owned AXUIElement reference.
    Some(unsafe { CFRetained::from_raw(NonNull::new(AXUIElementCreateSystemWide())?) })
}

fn is_element(element: &CFType) -> bool {
    // SAFETY: The type ID query takes no arguments.
    CFGetTypeID(Some(element)) == unsafe { AXUIElementGetTypeID() }
}

pub(crate) fn set_timeout(element: &CFType, seconds: f32) -> bool {
    // SAFETY: The type check establishes that this is an AXUIElement.
    is_element(element) && unsafe { AXUIElementSetMessagingTimeout(element, seconds) == 0 }
}

pub(crate) fn attribute(element: &CFType, name: &str) -> Option<CFRetained<CFType>> {
    if !is_element(element) {
        return None;
    }
    let mut value = ptr::null_mut();
    // SAFETY: The element was checked above, and Copy returns an owned CF reference.
    unsafe {
        if AXUIElementCopyAttributeValue(element, &CFString::from_str(name), &mut value) != 0 {
            return None;
        }
        Some(CFRetained::from_raw(NonNull::new(value)?))
    }
}
