//! Native display selection and panel window settings.

use std::ffi::c_void;
use std::ptr::NonNull;

use dupin_core::DisplaySelection;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSEvent, NSScreen, NSView, NSWorkspace};
use objc2_foundation::{NSArray, NSIntersectionRect, NSMouseInRect, NSNumber, NSRect, NSString};

use crate::{accessibility, accessibility_window};

/// # Safety
///
/// `ns_view` must point to a live AppKit `NSView` on the main thread.
pub unsafe fn is_key_window(ns_view: NonNull<c_void>) -> bool {
    let Some(_main) = MainThreadMarker::new() else {
        return false;
    };
    // SAFETY: The caller guarantees a live NSView for this borrow.
    let view = unsafe { ns_view.cast::<NSView>().as_ref() };
    view_is_key(view)
}

pub(crate) fn view_is_key(view: &NSView) -> bool {
    view.window().is_some_and(|window| window.isKeyWindow())
}

/// A single screen's identity and bounds in AppKit points.
pub struct DisplayGeometry {
    pub native_id: u32,
    pub frame: NSRect,
    pub visible_frame: NSRect,
    pub notice: Option<DisplayNotice>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DisplayNotice {
    AccessibilityRequired,
    FocusedWindowUnavailable,
}

/// Resolve the display when the panel opens, falling back to the primary screen.
pub fn display_geometry(selection: DisplaySelection) -> Option<DisplayGeometry> {
    let main = MainThreadMarker::new()?;
    let screens = NSScreen::screens(main);
    let primary = screens.firstObject()?;
    let mouse_screen = || {
        let pointer = NSEvent::mouseLocation();
        screens
            .iter()
            .find(|screen| NSMouseInRect(pointer, screen.frame(), false))
    };
    let (selected, notice) = match selection {
        DisplaySelection::Mouse => (mouse_screen(), None),
        DisplaySelection::Main => (None, None),
        DisplaySelection::Active => match focused_screen(main, &screens, primary.frame().max().y) {
            Ok(screen) => (Some(screen), None),
            Err(notice) => (None, Some(notice)),
        },
    };
    let screen = selected.unwrap_or(primary);
    let key = NSString::from_str("NSScreenNumber");
    let number = screen.deviceDescription().objectForKey(&key)?;
    Some(DisplayGeometry {
        native_id: number.downcast::<NSNumber>().ok()?.as_u32(),
        frame: screen.frame(),
        visible_frame: screen.visibleFrame(),
        notice,
    })
}

fn focused_screen(
    main: MainThreadMarker,
    screens: &NSArray<NSScreen>,
    primary_top: f64,
) -> Result<Retained<NSScreen>, DisplayNotice> {
    use DisplayNotice::{AccessibilityRequired, FocusedWindowUnavailable};

    let pid = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
        .filter(|pid| *pid > 0)
        .ok_or(FocusedWindowUnavailable)?;
    if pid as u32 == std::process::id() {
        return NSApplication::sharedApplication(main)
            .keyWindow()
            .and_then(|window| window.screen())
            .ok_or(FocusedWindowUnavailable);
    }
    if !accessibility::is_trusted() {
        return Err(AccessibilityRequired);
    }
    let frame = accessibility_window::frame(pid)
        .and_then(|frame| appkit_frame(frame, primary_top))
        .ok_or_else(|| {
            if accessibility::is_trusted() {
                FocusedWindowUnavailable
            } else {
                AccessibilityRequired
            }
        })?;
    let index = screen_with_largest_overlap(screens.iter().map(|screen| screen.frame()), frame)
        .ok_or(FocusedWindowUnavailable)?;
    Ok(screens.objectAtIndex(index))
}

fn screen_with_largest_overlap(screens: impl Iterator<Item = NSRect>, window: NSRect) -> Option<usize> {
    screens
        .enumerate()
        .filter_map(|(index, frame)| {
            let size = NSIntersectionRect(frame, window).size;
            let area = size.width * size.height;
            (area > 0.0).then_some((index, area))
        })
        .reduce(|best, candidate| if candidate.1 > best.1 { candidate } else { best })
        .map(|(index, _)| index)
}

fn appkit_frame(mut frame: NSRect, primary_top: f64) -> Option<NSRect> {
    if frame.is_empty()
        || ![
            frame.origin.x,
            frame.origin.y,
            frame.size.width,
            frame.size.height,
        ]
        .into_iter()
        .all(f64::is_finite)
    {
        return None;
    }
    // Accessibility measures from the primary display's top edge.
    frame.origin.y = primary_top - frame.max().y;
    Some(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::{NSPoint, NSSize};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }

    #[test]
    fn focused_window_uses_the_screen_with_most_overlap() {
        let screens = [rect(0.0, 0.0, 1440.0, 900.0), rect(-1280.0, 100.0, 1280.0, 800.0)];
        let window = appkit_frame(rect(-600.0, 150.0, 800.0, 450.0), 900.0).unwrap();
        assert_eq!(window, rect(-600.0, 300.0, 800.0, 450.0));
        assert_eq!(screen_with_largest_overlap(screens.into_iter(), window), Some(1));
        assert_eq!(
            screen_with_largest_overlap(screens.into_iter(), rect(1500.0, 100.0, 200.0, 200.0)),
            None
        );
        assert!(appkit_frame(rect(0.0, 0.0, 0.0, 450.0), 900.0).is_none());
    }

    #[test]
    fn shared_pointer_edges_belong_to_one_screen() {
        let upper = rect(0.0, 800.0, 1000.0, 900.0);
        let lower = rect(0.0, 0.0, 1000.0, 800.0);
        let pointer = NSPoint::new(500.0, 800.0);
        assert!(!NSMouseInRect(pointer, upper, false));
        assert!(NSMouseInRect(pointer, lower, false));
    }
}
