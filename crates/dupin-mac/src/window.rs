//! Native display selection and panel window settings.

use std::ffi::c_void;
use std::ptr::NonNull;

use dupin_core::DisplaySelection;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSScreen, NSView, NSWindow, NSWindowAnimationBehavior};
use objc2_foundation::{NSIntersectionRect, NSMouseInRect, NSNumber, NSRect, NSString};

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

/// A single screen's identity and bounds in AppKit points.
pub struct DisplayGeometry {
    pub native_id: u32,
    pub frame: NSRect,
    pub visible_frame: NSRect,
    pub active_unavailable: bool,
}

/// Resolve the display when the panel opens, falling back to the primary screen.
pub fn display_geometry(selection: DisplaySelection) -> Option<DisplayGeometry> {
    let screens = NSScreen::screens(MainThreadMarker::new()?);
    let primary = screens.firstObject()?;
    let selected = match selection {
        DisplaySelection::Mouse => {
            let pointer = NSEvent::mouseLocation();
            screens
                .iter()
                .find(|screen| NSMouseInRect(pointer, screen.frame(), false))
        }
        DisplaySelection::Main => None,
        DisplaySelection::Active => accessibility::focused_window_frame().and_then(|frame| {
            let frame = appkit_frame(frame, primary.frame().size.height)?;
            let index = screen_with_largest_overlap(screens.iter().map(|screen| screen.frame()), frame)?;
            Some(screens.objectAtIndex(index))
        }),
    };
    let active_unavailable = selection == DisplaySelection::Active && selected.is_none();
    let screen = selected.unwrap_or(primary);
    let key = NSString::from_str("NSScreenNumber");
    let number = screen.deviceDescription().objectForKey(&key)?;
    Some(DisplayGeometry {
        native_id: number.downcast::<NSNumber>().ok()?.as_u32(),
        frame: screen.frame(),
        visible_frame: screen.visibleFrame(),
        active_unavailable,
    })
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

fn appkit_frame(mut frame: NSRect, primary_height: f64) -> Option<NSRect> {
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
    frame.origin.y = primary_height - frame.max().y;
    Some(frame)
}

mod accessibility {
    use std::ffi::c_void;
    use std::ptr::{self, NonNull};

    use objc2_app_kit::NSWorkspace;
    use objc2_core_foundation::{CFGetTypeID, CFRetained, CFString, CFType, CFTypeID};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    const MESSAGING_TIMEOUT_SECONDS: f32 = 0.1;
    const AX_POINT: u32 = 1;
    const AX_SIZE: u32 = 2;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
        fn AXUIElementGetTypeID() -> CFTypeID;
        fn AXUIElementCreateApplication(pid: i32) -> *mut CFType;
        fn AXUIElementSetMessagingTimeout(element: &CFType, timeout: f32) -> i32;
        fn AXUIElementCopyAttributeValue(
            element: &CFType,
            attribute: &CFString,
            value: &mut *mut CFType,
        ) -> i32;
        fn AXValueGetTypeID() -> CFTypeID;
        fn AXValueGetValue(value: &CFType, kind: u32, out: *mut c_void) -> u8;
    }

    pub(super) fn focused_window_frame() -> Option<NSRect> {
        // SAFETY: AXIsProcessTrusted only queries the current grant.
        if unsafe { AXIsProcessTrusted() } == 0 {
            return None;
        }
        let pid = NSWorkspace::sharedWorkspace()
            .frontmostApplication()?
            .processIdentifier();
        // SAFETY: The Create call gives us an owned AXUIElement reference for this PID.
        let app = unsafe { CFRetained::from_raw(NonNull::new(AXUIElementCreateApplication(pid))?) };
        let app = timed(app)?;
        let window = attribute(&app, "AXFocusedWindow")?;
        let window = timed(window)?;
        let position = attribute(&window, "AXPosition")?;
        let size = attribute(&window, "AXSize")?;
        read_frame(&position, &size)
    }

    fn timed(element: CFRetained<CFType>) -> Option<CFRetained<CFType>> {
        // SAFETY: Check the CF type before passing the object to the element-specific API.
        if CFGetTypeID(Some(&element)) != unsafe { AXUIElementGetTypeID() } {
            return None;
        }
        // SAFETY: The check above establishes that this is an AXUIElement.
        // Bound calls to the foreground app without changing the process-wide timeout.
        (unsafe { AXUIElementSetMessagingTimeout(&element, MESSAGING_TIMEOUT_SECONDS) } == 0)
            .then_some(element)
    }

    fn attribute(element: &CFType, name: &str) -> Option<CFRetained<CFType>> {
        let mut value = ptr::null_mut();
        // SAFETY: Callers pass elements checked by `timed`. Copy returns an owned CF reference.
        unsafe {
            if AXUIElementCopyAttributeValue(element, &CFString::from_str(name), &mut value) != 0 {
                return None;
            }
            Some(CFRetained::from_raw(NonNull::new(value)?))
        }
    }

    fn read_frame(position: &CFType, size: &CFType) -> Option<NSRect> {
        // SAFETY: Check both CF types before passing them to AXValueGetValue.
        let value_type = unsafe { AXValueGetTypeID() };
        if CFGetTypeID(Some(position)) != value_type || CFGetTypeID(Some(size)) != value_type {
            return None;
        }
        let mut origin = NSPoint::ZERO;
        let mut dimensions = NSSize::ZERO;
        // SAFETY: AXValueGetValue checks the requested kind and writes only its corresponding type.
        unsafe {
            (AXValueGetValue(position, AX_POINT, ptr::from_mut(&mut origin).cast()) != 0
                && AXValueGetValue(size, AX_SIZE, ptr::from_mut(&mut dimensions).cast()) != 0)
                .then_some(NSRect::new(origin, dimensions))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rejects_non_accessibility_objects() {
            let string: CFRetained<CFType> = CFString::from_str("not an AX object").into();
            assert!(read_frame(&string, &string).is_none());
            assert!(timed(string).is_none());
        }
    }
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
