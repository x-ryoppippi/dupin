//! Pasting into the frontmost app by posting ⌘V, which macOS allows only with Accessibility
//! access. Checking never asks; asking is a separate, deliberate call.

use std::ffi::c_void;
use std::sync::Once;

use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

type CGEventRef = *mut c_void;

const HID_EVENT_TAP: u32 = 0;
const COMMAND_FLAG: u64 = 0x0010_0000;

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

/// Ask macOS for access at most once per launch. Returns whether this call requested it.
pub fn request_once() -> bool {
    static REQUEST: Once = Once::new();
    let mut requested = false;
    REQUEST.call_once(|| {
        // SAFETY: As in `allowed`, the API takes no arguments.
        unsafe { CGRequestPostEventAccess() };
        requested = true;
    });
    requested
}

/// Resolve the front app's paste shortcut on the main thread, after restoring its input source.
pub fn key_code() -> u16 {
    crate::input_source::command_v_key()
}

/// Copy text and press ⌘V in the frontmost app using its resolved key code.
pub fn paste(key: u16, text: &str) -> Result<(), String> {
    // Rune queues ordinary writes without a completion callback, but ⌘V needs the new text in place.
    autoreleasepool(|_| {
        let board = NSPasteboard::generalPasteboard();
        board.clearContents();
        // SAFETY: NSPasteboardTypeString is an immutable AppKit constant.
        if board.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString }) {
            Ok(())
        } else {
            Err("Couldn't copy the snippet to the clipboard.".to_owned())
        }
    })?;
    press(key)
}

/// Press ⌘V in the frontmost app, pasting what the clipboard holds.
pub fn press(key: u16) -> Result<(), String> {
    for down in [true, false] {
        // SAFETY: A null source is allowed; the event is released after it is posted.
        unsafe {
            let event = CGEventCreateKeyboardEvent(std::ptr::null(), key, down);
            if event.is_null() {
                return Err("Couldn't paste into the front app.".into());
            }
            CGEventSetFlags(event, COMMAND_FLAG);
            CGEventPost(HID_EVENT_TAP, event);
            CFRelease(event);
        }
    }
    Ok(())
}
