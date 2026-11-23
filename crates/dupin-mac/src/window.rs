//! AppKit settings for the panel's own window.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSScreen, NSView, NSWindow, NSWindowAnimationBehavior};

/// Make the window holding `ns_view` appear and disappear at once. AppKit animates a panel's
/// ordering in by default, which held Dupin's panel off screen for about 265 ms after its
/// frame was ready; a launcher has to be there the moment its hotkey lands.
///
/// # Safety
///
/// `ns_view` must point to a live `NSView`, as a window's raw AppKit handle does while the
/// window exists.
pub unsafe fn appear_instantly(ns_view: NonNull<c_void>) -> bool {
    let Some(_main) = MainThreadMarker::new() else {
        return false;
    };
    // SAFETY: The caller guarantees a live NSView; we only borrow it for this call.
    let view: &NSView = unsafe { ns_view.cast().as_ref() };
    let Some(window): Option<objc2::rc::Retained<NSWindow>> = view.window() else {
        return false;
    };
    window.setAnimationBehavior(NSWindowAnimationBehavior::None);
    true
}

/// # Safety
///
/// `ns_view` must point to a live AppKit `NSView` on the main thread.
pub unsafe fn is_key_window(ns_view: NonNull<c_void>) -> bool {
    let Some(_main) = MainThreadMarker::new() else {
        return false;
    };
    // SAFETY: The caller guarantees a live NSView for this borrow.
    let view = unsafe { ns_view.cast::<NSView>().as_ref() };
    view.window().is_some_and(|window| window.isKeyWindow())
}

/// The height in points of the display the pointer is on, where Rune's
/// `DisplayTarget::UnderPointer` places the panel. Rune reports displays only at startup, and
/// placing the panel at a display's center needs the current one. None off the main thread or
/// when no screen holds the pointer.
pub fn pointer_display_height() -> Option<f64> {
    let main = MainThreadMarker::new()?;
    let pointer = NSEvent::mouseLocation();
    NSScreen::screens(main)
        .iter()
        .map(|screen| screen.frame())
        .find_map(|frame| {
            let inside = (frame.origin.x..frame.origin.x + frame.size.width).contains(&pointer.x)
                && (frame.origin.y..frame.origin.y + frame.size.height).contains(&pointer.y);
            inside.then_some(frame.size.height)
        })
}
