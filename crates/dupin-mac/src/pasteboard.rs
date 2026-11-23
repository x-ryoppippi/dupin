//! The general pasteboard's plain text.

use objc2_app_kit::{NSPasteboard, NSPasteboardType, NSPasteboardTypeString};
use objc2_foundation::NSString;

fn plain_text() -> &'static NSPasteboardType {
    // SAFETY: An AppKit constant, initialized before any code runs and never changed.
    unsafe { NSPasteboardTypeString }
}

/// The text the clipboard holds, if any.
pub fn text() -> Option<String> {
    NSPasteboard::generalPasteboard()
        .stringForType(plain_text())
        .map(|text| text.to_string())
}

/// Replace what the clipboard holds with `text`.
pub fn set_text(text: &str) -> Result<(), String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    if pasteboard.setString_forType(&NSString::from_str(text), plain_text()) {
        Ok(())
    } else {
        Err("Couldn't copy the snippet to the clipboard.".into())
    }
}
