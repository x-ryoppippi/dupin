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

#[derive(Clone, Copy)]
pub enum Permission {
    Accessibility,
}

impl Permission {
    pub fn is_granted(self) -> bool {
        match self {
            Self::Accessibility => unsafe { AXIsProcessTrusted() != 0 },
        }
    }

    pub fn request_once(self) {
        match self {
            Self::Accessibility => {
                static REQUEST: Once = Once::new();
                REQUEST.call_once(|| {
                    autoreleasepool(|_| {
                        let defaults = NSUserDefaults::standardUserDefaults();
                        let key = NSString::from_str("RequestedAccessibility");
                        if defaults.boolForKey(&key) {
                            return;
                        }
                        defaults.setBool_forKey(true, &key);
                        unsafe {
                            let options = CFDictionary::from_slices(
                                &[&*kAXTrustedCheckOptionPrompt],
                                &[CFBoolean::new(true)],
                            );
                            AXIsProcessTrustedWithOptions(&*options);
                        }
                    });
                });
            }
        }
    }

    pub fn settings_url(self) -> &'static str {
        match self {
            Self::Accessibility => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
        }
    }
}
