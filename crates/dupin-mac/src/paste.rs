//! Pasting into the frontmost app by posting ⌘V, which macOS allows only with Accessibility
//! access. Checking never asks; asking is a separate, deliberate call.

use std::ffi::c_void;

/// System Settings › Privacy & Security › Accessibility.
pub const ACCESSIBILITY_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

type CGEventRef = *mut c_void;

const HID_EVENT_TAP: u32 = 0;
const COMMAND_FLAG: u64 = 0x0010_0000;
/// `kVK_ANSI_V`: the key in the V position, as other apps' Paste shortcuts expect.
const V_KEY: u16 = 9;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> CGEventRef;
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventPost(tap: u32, event: CGEventRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: *const c_void);
}

/// Whether Dupin may post key events, without asking.
pub fn allowed() -> bool {
    // SAFETY: A status check with no arguments.
    unsafe { CGPreflightPostEventAccess() }
}

/// Ask macOS for access, which shows its prompt when the user hasn't decided yet.
pub fn request() {
    // SAFETY: As `allowed`; the answer arrives through System Settings, not the return value.
    unsafe { CGRequestPostEventAccess() };
}

/// Press ⌘V in whichever app is frontmost.
pub fn paste() -> Result<(), String> {
    for down in [true, false] {
        // SAFETY: A null source is allowed; the event is released after it is posted.
        unsafe {
            let event = CGEventCreateKeyboardEvent(std::ptr::null(), V_KEY, down);
            if event.is_null() {
                return Err("Couldn't paste the snippet into the front app.".into());
            }
            CGEventSetFlags(event, COMMAND_FLAG);
            CGEventPost(HID_EVENT_TAP, event);
            CFRelease(event);
        }
    }
    Ok(())
}
