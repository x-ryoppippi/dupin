//! Whether Dupin may use the Accessibility API, and asking for it at the point of use.

use std::sync::Once;

use objc2::rc::autoreleasepool;
use objc2_core_foundation::{CFBoolean, CFDictionary, CFString};
use objc2_foundation::{NSString, NSUserDefaults};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const CFDictionary<CFString, CFBoolean>) -> u8;
    static kAXTrustedCheckOptionPrompt: *const CFString;
}

pub const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

/// Reads the current grant without prompting.
pub fn is_trusted() -> bool {
    // SAFETY: A status query with no arguments.
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Shows macOS's prompt at most once per user, so a denial is answered with an explanation
/// rather than repeated prompts.
pub fn request_once() {
    static REQUEST: Once = Once::new();
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
        });
    });
}
