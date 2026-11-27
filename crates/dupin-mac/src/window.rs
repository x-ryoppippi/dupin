//! Native display selection and panel window settings.

use std::ffi::c_void;
use std::ptr::NonNull;

use dupin_core::DisplaySelection;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_app_kit::{NSApplication, NSEvent, NSScreen, NSView, NSWorkspace};
use objc2_foundation::{
    NSArray, NSIntersectionRect, NSMouseInRect, NSNumber, NSPoint, NSRect, NSSize, NSString,
};

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

/// Move and size the view's window to the frame AppKit saved under `name`, if it saved one, and
/// save it there again whenever it moves or resizes. Either way the window is kept inside its
/// screen's visible area, so a default size or a frame left on a larger display still fits.
/// False when the window has no such name.
///
/// # Safety
///
/// `ns_view` must point to a live AppKit `NSView` on the main thread.
pub unsafe fn autosave_frame(ns_view: NonNull<c_void>, name: &str) -> bool {
    let Some(main) = MainThreadMarker::new() else {
        return false;
    };
    // SAFETY: The caller guarantees a live NSView for this borrow.
    let view = unsafe { ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else {
        return false;
    };
    let name = NSString::from_str(name);
    window.setFrameUsingName(&name);
    if let Some(screen) = window.screen().or_else(|| NSScreen::mainScreen(main)) {
        let frame = window.frame();
        let fitted = fit_within(frame, screen.visibleFrame());
        if fitted != frame {
            window.setFrame_display(fitted, false);
        }
    }
    window.setFrameAutosaveName(&name)
}

/// `frame` shrunk to fit `area` and moved inside it, keeping its top edge where it can.
fn fit_within(frame: NSRect, area: NSRect) -> NSRect {
    let width = frame.size.width.min(area.size.width);
    let height = frame.size.height.min(area.size.height);
    let x = frame.origin.x.clamp(area.origin.x, area.max().x - width);
    let top = frame.max().y.clamp(area.origin.y + height, area.max().y);
    NSRect::new(NSPoint::new(x, top - height), NSSize::new(width, height))
}

/// Keep the bottom edge of the view's window at `bottom`, in AppKit's bottom-up screen points,
/// whenever its frame is set, so it grows and shrinks from its top; `None` lets it go.
///
/// Rune keeps a window's top edge where it is when the window resizes to fit its content, and
/// has no way to keep another. Every frame change Rune makes goes through
/// `-[NSWindow setFrame:display:]`, so the window's class is swapped for a subclass of its own
/// whose `setFrame:display:` moves the requested frame onto the pinned edge: the new size and
/// position land in the one call, with no move in between.
///
/// # Safety
///
/// `ns_view` must point to a live AppKit `NSView` on the main thread.
pub unsafe fn pin_bottom_edge(ns_view: NonNull<c_void>, bottom: Option<f64>) {
    let Some(_main) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: The caller guarantees a live NSView for this borrow.
    let view = unsafe { ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else {
        return;
    };
    let object: &AnyObject = &window;
    let key = std::ptr::from_ref(object) as usize;
    PINNED.with_borrow_mut(|pinned| match bottom {
        Some(bottom) => {
            pinned.insert(key, bottom);
        }
        None => {
            pinned.remove(&key);
        }
    });
    if bottom.is_some() {
        // SAFETY: on the main thread, with the window alive for this borrow.
        unsafe { adopt_pinning_class(object) };
    }
    if let Some(bottom) = bottom {
        let frame = window.frame();
        let pinned = pinned_frame(frame, Some(bottom));
        if pinned != frame {
            window.setFrame_display(pinned, false);
        }
    }
}

thread_local! {
    /// Each pinned window's bottom edge, by the window's address.
    static PINNED: std::cell::RefCell<std::collections::HashMap<usize, f64>> = Default::default();
}

const PINNING_PREFIX: &str = "DupinPinnedBottom_";

/// `frame` on the pinned bottom edge, keeping its size and horizontal place.
fn pinned_frame(mut frame: NSRect, bottom: Option<f64>) -> NSRect {
    if let Some(bottom) = bottom {
        frame.origin.y = bottom;
    }
    frame
}

/// Swaps `window`'s class for one that keeps its pinned edge, made once per original class.
///
/// # Safety
///
/// `window` must be a live `NSWindow`, on the main thread.
unsafe fn adopt_pinning_class(window: &AnyObject) {
    let class = window.class();
    if class.name().to_string_lossy().starts_with(PINNING_PREFIX) {
        return;
    }
    let name = std::ffi::CString::new(format!("{PINNING_PREFIX}{}", class.name().to_string_lossy()))
        .expect("class names have no NUL");
    let pinning = AnyClass::get(&name).or_else(|| {
        let mut builder = ClassBuilder::new(&name, class)?;
        // SAFETY: the method has `-[NSWindow setFrame:display:]`'s argument and return types.
        unsafe {
            builder.add_method(
                sel!(setFrame:display:),
                set_frame_display as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, NSRect, Bool),
            );
        }
        Some(builder.register())
    });
    if let Some(pinning) = pinning {
        // SAFETY: the subclass adds a method and no ivars, so the instance's layout still fits.
        unsafe { AnyObject::set_class(window, pinning) };
    }
}

unsafe extern "C-unwind" fn set_frame_display(this: *mut AnyObject, _: Sel, frame: NSRect, display: Bool) {
    // SAFETY: AppKit sends the message to a live window.
    let Some(this) = (unsafe { this.as_ref() }) else {
        return;
    };
    let key = std::ptr::from_ref(this) as usize;
    let frame = pinned_frame(frame, PINNED.with_borrow(|pinned| pinned.get(&key).copied()));
    // A subclass made over the pinning one (KVO's, say) still reaches the original's method.
    let mut class = this.class();
    while !class.name().to_string_lossy().starts_with(PINNING_PREFIX) {
        let Some(superclass) = class.superclass() else {
            return;
        };
        class = superclass;
    }
    let Some(original) = class.superclass() else {
        return;
    };
    // SAFETY: the original class is an NSWindow subclass, which takes `setFrame:display:`.
    unsafe {
        let _: () = msg_send![super(this, original), setFrame: frame, display: display];
    }
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
mod pin_tests {
    use super::pinned_frame;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    #[test]
    fn a_pinned_frame_keeps_its_bottom_edge_and_grows_upwards() {
        let rect = |y, h| NSRect::new(NSPoint::new(40.0, y), NSSize::new(680.0, h));
        // Rune keeps the top: a 64-point panel at y 300 asked to grow to 400 points.
        let asked = rect(300.0 + 64.0 - 400.0, 400.0);
        assert_eq!(pinned_frame(asked, Some(300.0)), rect(300.0, 400.0));
        assert_eq!(pinned_frame(rect(250.0, 64.0), Some(300.0)), rect(300.0, 64.0));
        assert_eq!(pinned_frame(asked, None), asked);
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
    fn a_window_too_large_or_off_its_screen_is_fitted_inside_it() {
        let visible = rect(0.0, 80.0, 1280.0, 720.0);
        let inside = rect(100.0, 200.0, 960.0, 500.0);
        assert_eq!(fit_within(inside, visible), inside);
        // Taller and wider than the screen: it shrinks to the visible area.
        assert_eq!(fit_within(rect(-50.0, 0.0, 2000.0, 1200.0), visible), visible);
        // Left on a display to the right: it comes back, top edge kept.
        assert_eq!(
            fit_within(rect(1500.0, 300.0, 960.0, 400.0), visible),
            rect(320.0, 300.0, 960.0, 400.0)
        );
        // Above the menu bar: its top comes down to the visible area's.
        assert_eq!(
            fit_within(rect(100.0, 600.0, 960.0, 640.0), visible),
            rect(100.0, 160.0, 960.0, 640.0)
        );
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
