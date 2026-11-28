//! Native display selection and panel window settings.

use std::ffi::c_void;
use std::ptr::NonNull;

use dupin_core::DisplaySelection;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
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
    // Not `clamp`, which panics when rounding leaves the far bound a hair below the near one.
    let x = frame.origin.x.min(area.max().x - width).max(area.origin.x);
    let top = frame.max().y.max(area.origin.y + height).min(area.max().y);
    NSRect::new(NSPoint::new(x, top - height), NSSize::new(width, height))
}

/// Keep the bottom edge of the view's window at `bottom`, in AppKit's bottom-up screen points,
/// whenever its frame is set, so it grows and shrinks from its top; `None` lets it go. False
/// when the edge can't be held, so the window grows downwards as Rune sizes it.
///
/// Rune keeps a window's top edge where it is when the window resizes to fit its content, and
/// has no way to keep another. Every frame change Rune makes goes through
/// `-[NSWindow setFrame:display:]`, so the window's class gets a `setFrame:display:` of its own
/// that moves the requested frame onto the pinned edge: the new size and position land in the
/// one call, with no move in between.
///
/// # Safety
///
/// `ns_view` must point to a live AppKit `NSView` on the main thread.
pub unsafe fn pin_bottom_edge(ns_view: NonNull<c_void>, bottom: Option<f64>) -> bool {
    let Some(_main) = MainThreadMarker::new() else {
        return false;
    };
    // SAFETY: The caller guarantees a live NSView for this borrow.
    let view = unsafe { ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else {
        return false;
    };
    // SAFETY: a live NSWindow, on the main thread.
    unsafe { pin_window(&window, bottom) }
}

/// # Safety
///
/// `window` must be a live object that answers `frame` and `setFrame:display:` as an
/// `NSWindow` does, on the thread that sets its frames.
unsafe fn pin_window(window: &AnyObject, bottom: Option<f64>) -> bool {
    let key = std::ptr::from_ref(window) as usize;
    let Some(bottom) = bottom else {
        PINNED.with_borrow_mut(|pinned| pinned.remove(&key));
        return true;
    };
    // SAFETY: the caller guarantees a live window.
    if !unsafe { hold_frames(window) } {
        PINNED.with_borrow_mut(|pinned| pinned.remove(&key));
        return false;
    }
    PINNED.with_borrow_mut(|pinned| pinned.insert(key, bottom));
    // SAFETY: the caller guarantees the window answers both messages.
    unsafe {
        let frame: NSRect = msg_send![window, frame];
        let pinned = pinned_frame(frame, Some(bottom));
        if pinned != frame {
            let _: () = msg_send![window, setFrame: pinned, display: Bool::NO];
        }
    }
    true
}

thread_local! {
    /// Each pinned window's bottom edge, by the window's address.
    static PINNED: std::cell::RefCell<std::collections::HashMap<usize, f64>> = Default::default();
    /// Each class given the pinning `setFrame:display:`, by address, with the superclass whose
    /// method it passes the frame on to.
    static HELD: std::cell::RefCell<std::collections::HashMap<usize, usize>> = Default::default();
}

/// `frame` on the pinned bottom edge, keeping its size and horizontal place.
fn pinned_frame(mut frame: NSRect, bottom: Option<f64>) -> NSRect {
    if let Some(bottom) = bottom {
        frame.origin.y = bottom;
    }
    frame
}

/// Gives the class `window` was declared with a `setFrame:display:` that keeps a pinned
/// window's edge, once per class. Windows of the class that aren't pinned pass their frames on
/// unchanged. False when the class has a `setFrame:display:` of its own, which adding one would
/// replace.
///
/// The method goes on the declared class, `-class`, rather than on a subclass swapped in as the
/// window's class: once KVO observes the window, its class is KVO's subclass of the declared
/// one, whose `-class` answers nil for any subclass made over it, and `-[NSWindow animator]`
/// throws on a nil class.
///
/// # Safety
///
/// `window` must be a live object, on the thread that sets its frames.
unsafe fn hold_frames(window: &AnyObject) -> bool {
    // SAFETY: every NSObject answers `-class` with its class.
    let declared: &AnyClass = unsafe { msg_send![window, class] };
    let key = std::ptr::from_ref(declared) as usize;
    if HELD.with_borrow(|held| held.contains_key(&key)) {
        return true;
    }
    let selector = sel!(setFrame:display:);
    let Some(parent) = declared.superclass() else {
        return false;
    };
    let (Some(own), Some(inherited)) = (
        declared.instance_method(selector),
        parent.instance_method(selector),
    ) else {
        return false;
    };
    if !std::ptr::eq(own, inherited) {
        return false;
    }
    let method: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, NSRect, Bool) = set_frame_display;
    // SAFETY: the method takes `setFrame:display:`'s arguments, under the inherited method's
    // type encoding, and the runtime only adds it to a class that doesn't implement it itself.
    let added = unsafe {
        objc2::ffi::class_addMethod(
            std::ptr::from_ref(declared).cast_mut(),
            selector,
            std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, NSRect, Bool), Imp>(
                method,
            ),
            objc2::ffi::method_getTypeEncoding(own),
        )
    };
    if added.as_bool() {
        HELD.with_borrow_mut(|held| held.insert(key, std::ptr::from_ref(parent) as usize));
    }
    added.as_bool()
}

unsafe extern "C-unwind" fn set_frame_display(this: *mut AnyObject, _: Sel, frame: NSRect, display: Bool) {
    // SAFETY: AppKit sends the message to a live window.
    let Some(this) = (unsafe { this.as_ref() }) else {
        return;
    };
    let key = std::ptr::from_ref(this) as usize;
    let frame = pinned_frame(frame, PINNED.with_borrow(|pinned| pinned.get(&key).copied()));
    // The window's own class may be a subclass of the one that holds this method (KVO's).
    let mut class = Some(this.class());
    let parent = loop {
        let Some(current) = class else {
            return;
        };
        if let Some(parent) =
            HELD.with_borrow(|held| held.get(&(std::ptr::from_ref(current) as usize)).copied())
        {
            break parent;
        }
        class = current.superclass();
    };
    // SAFETY: `HELD` keeps registered classes, which live for the whole process, and the parent
    // implements `setFrame:display:`, as `hold_frames` found.
    unsafe {
        let parent = &*(parent as *const AnyClass);
        let _: () = msg_send![super(this, parent), setFrame: frame, display: display];
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
    use super::{pin_window, pinned_frame};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, NSObject, Sel};
    use objc2::{ClassType, msg_send, sel};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::ffi::c_void;

    #[test]
    fn a_pinned_frame_keeps_its_bottom_edge_and_grows_upwards() {
        let rect = |y, h| NSRect::new(NSPoint::new(40.0, y), NSSize::new(680.0, h));
        // Rune keeps the top: a 64-point panel at y 300 asked to grow to 400 points.
        let asked = rect(300.0 + 64.0 - 400.0, 400.0);
        assert_eq!(pinned_frame(asked, Some(300.0)), rect(300.0, 400.0));
        assert_eq!(pinned_frame(rect(250.0, 64.0), Some(300.0)), rect(300.0, 64.0));
        assert_eq!(pinned_frame(asked, None), asked);
    }

    thread_local! {
        static FRAME: std::cell::Cell<NSRect> = const { std::cell::Cell::new(NSRect::ZERO) };
    }

    extern "C-unwind" fn frame(_: *mut AnyObject, _: Sel) -> NSRect {
        FRAME.get()
    }
    extern "C-unwind" fn set_frame(_: *mut AnyObject, _: Sel, frame: NSRect, _: Bool) {
        FRAME.set(frame);
    }
    extern "C-unwind" fn title(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
        std::ptr::null_mut()
    }
    extern "C-unwind" fn set_title(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {}

    /// A window class as Rune declares its panel's, inheriting `frame` and `setFrame:display:`.
    fn window_class() -> &'static AnyClass {
        let mut base = ClassBuilder::new(c"DupinPinTestWindowBase", NSObject::class()).unwrap();
        // SAFETY: each function takes its selector's arguments and returns its type.
        unsafe {
            base.add_method(sel!(frame), frame as extern "C-unwind" fn(_, _) -> _);
            base.add_method(
                sel!(setFrame:display:),
                set_frame as extern "C-unwind" fn(_, _, _, _),
            );
            base.add_method(sel!(title), title as extern "C-unwind" fn(_, _) -> _);
            base.add_method(sel!(setTitle:), set_title as extern "C-unwind" fn(_, _, _));
        }
        let base = base.register();
        ClassBuilder::new(c"DupinPinTestWindow", base).unwrap().register()
    }

    /// AppKit or winit observes the panel through KVO before the bottom position pins it, so
    /// the panel's class is already KVO's subclass. The window must still answer `-class`
    /// (`-[NSWindow animator]` throws on nil) and keep its bottom edge.
    #[test]
    fn a_window_kvo_observes_keeps_its_class_and_its_bottom_edge() {
        let class = window_class();
        // SAFETY: registered classes whose `new` returns a live instance; KVO observes a key the
        // class has a getter and setter for, and the observer is removed before both are freed.
        unsafe {
            let window: Retained<AnyObject> = msg_send![class, new];
            let observer = NSObject::new();
            let key = NSString::from_str("title");
            let _: () = msg_send![&*window, addObserver: &*observer, forKeyPath: &*key, options: 0usize, context: std::ptr::null_mut::<c_void>()];
            let kvo = window.class();
            assert!(kvo.name().to_string_lossy().starts_with("NSKVONotifying_"));

            let rect = |y, h| NSRect::new(NSPoint::new(40.0, y), NSSize::new(680.0, h));
            FRAME.set(rect(250.0, 64.0));
            assert!(pin_window(&window, Some(300.0)));
            assert_eq!(FRAME.get(), rect(300.0, 64.0));
            let declared: *const AnyClass = msg_send![&*window, class];
            assert_eq!(declared, std::ptr::from_ref(class));
            assert!(std::ptr::eq(window.class(), kvo));

            let _: () = msg_send![&*window, setFrame: rect(300.0 + 64.0 - 400.0, 400.0), display: Bool::YES];
            assert_eq!(FRAME.get(), rect(300.0, 400.0));

            assert!(pin_window(&window, None));
            let _: () = msg_send![&*window, setFrame: rect(10.0, 64.0), display: Bool::YES];
            assert_eq!(FRAME.get(), rect(10.0, 64.0));

            let _: () = msg_send![&*window, removeObserver: &*observer, forKeyPath: &*key];
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
        // 0.1 + 0.4 - 0.4 rounds below 0.1: the far bound lands left of the near one.
        let fitted = fit_within(rect(0.0, 0.0, 1.0, 1.0), rect(0.1, 0.1, 0.4, 0.4));
        assert_eq!((fitted.origin.x, fitted.size.width), (0.1, 0.4));
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
