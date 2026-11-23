use std::ffi::c_void;
use std::ptr::{self, NonNull};

use dupin_core::window_management::{Display, Frame, Layout, target_frame};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSWorkspace};
use objc2_core_foundation::{CFRetained, CFString, CFType};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use crate::accessibility::{self, AX_NO_VALUE, AX_POINT, AX_SIZE, attribute};

const MESSAGING_TIMEOUT_SECONDS: f32 = 0.25;
const AX_ATTRIBUTE_UNSUPPORTED: i32 = -25205;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementIsAttributeSettable(element: &CFType, name: &CFString, settable: &mut u8) -> i32;
    fn AXUIElementSetAttributeValue(element: &CFType, name: &CFString, value: &CFType) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> *mut CFType;
}

#[derive(Clone, Copy)]
pub enum Error {
    PermissionRequested,
    PermissionRequired,
    NoWindow,
    NoDisplay,
    WindowUnavailable,
    FrameRejected,
}

impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Self::PermissionRequested | Self::PermissionRequired => {
                "Dupin needs Accessibility access to move and resize windows."
            }
            Self::NoWindow => "There is no focused window to arrange.",
            Self::NoDisplay => "No display is available to arrange this window.",
            Self::WindowUnavailable => "Dupin couldn’t read the focused window.",
            Self::FrameRejected => "The window couldn’t be moved or resized as requested.",
        }
    }
}

pub fn frontmost_pid() -> Option<i32> {
    let pid = NSWorkspace::sharedWorkspace()
        .frontmostApplication()?
        .processIdentifier();
    (pid > 0 && pid as u32 != std::process::id()).then_some(pid)
}

pub fn displays() -> Vec<Display> {
    let Some(main) = MainThreadMarker::new() else {
        return Vec::new();
    };
    let screens = NSScreen::screens(main);
    let Some(primary) = screens.firstObject() else {
        return Vec::new();
    };
    let primary_top = primary.frame().max().y;
    let convert = |rect: NSRect| {
        Frame::new(rect.origin.x, rect.origin.y, rect.size.width, rect.size.height).flip_y(primary_top)
    };
    screens
        .iter()
        .map(|screen| Display {
            frame: convert(screen.frame()),
            visible: convert(screen.visibleFrame()),
        })
        .collect()
}

/// Run on a worker so an unresponsive application cannot delay the panel.
pub fn arrange(pid: i32, layout: Layout, displays: &[Display]) -> Result<(), Error> {
    if displays.is_empty() {
        return Err(Error::NoDisplay);
    }
    if !accessibility::is_trusted() {
        return Err(if accessibility::request_once() {
            Error::PermissionRequested
        } else {
            Error::PermissionRequired
        });
    }

    arrange_trusted(pid, layout, displays).map_err(|error| {
        if accessibility::is_trusted() {
            error
        } else {
            Error::PermissionRequired
        }
    })
}

fn arrange_trusted(pid: i32, layout: Layout, displays: &[Display]) -> Result<(), Error> {
    let app = accessibility::application(pid).ok_or(Error::WindowUnavailable)?;
    let app = timed(app)?;
    let window = timed(focused_window(&app)?)?;
    let initial = read_frame(&window).ok_or(Error::WindowUnavailable)?;
    let target = target_frame(layout, initial, displays).ok_or(Error::NoDisplay)?;
    update_frame(&window, initial, target).ok_or(Error::FrameRejected)
}

fn timed(element: CFRetained<CFType>) -> Result<CFRetained<CFType>, Error> {
    // A timeout on the system-wide element would affect other Accessibility features.
    accessibility::set_timeout(&element, MESSAGING_TIMEOUT_SECONDS)
        .then_some(element)
        .ok_or(Error::WindowUnavailable)
}

fn focused_window(app: &CFType) -> Result<CFRetained<CFType>, Error> {
    match attribute(app, "AXFocusedWindow") {
        Err(AX_NO_VALUE | AX_ATTRIBUTE_UNSUPPORTED) => match attribute(app, "AXMainWindow") {
            Err(AX_NO_VALUE | AX_ATTRIBUTE_UNSUPPORTED) => Err(Error::NoWindow),
            result => result.map_err(|_| Error::WindowUnavailable),
        },
        result => result.map_err(|_| Error::WindowUnavailable),
    }
}

fn read_frame(window: &CFType) -> Option<Frame> {
    let position = attribute(window, "AXPosition").ok()?;
    let size = attribute(window, "AXSize").ok()?;
    let rect = accessibility::read_frame(&position, &size)?;
    let frame = Frame::new(rect.origin.x, rect.origin.y, rect.size.width, rect.size.height);
    frame.is_valid().then_some(frame)
}

fn update_frame(window: &CFType, initial: Frame, target: Frame) -> Option<()> {
    let size_changed = initial.width != target.width || initial.height != target.height;
    let position_changed = initial.x != target.x || initial.y != target.y;
    let position_attribute = CFString::from_str("AXPosition");
    let size_attribute = CFString::from_str("AXSize");

    // Check both attributes before changing either, since some apps expose fixed windows.
    if (size_changed && !is_settable(window, &size_attribute))
        || (position_changed && !is_settable(window, &position_attribute))
    {
        return None;
    }

    if size_changed {
        let size = NSSize::new(target.width, target.height);
        let value = unsafe {
            CFRetained::from_raw(NonNull::new(AXValueCreate(AX_SIZE, ptr::from_ref(&size).cast()))?)
        };
        if unsafe { AXUIElementSetAttributeValue(window, &size_attribute, &value) } != 0 {
            return None;
        }
    }
    if position_changed {
        let position = NSPoint::new(target.x, target.y);
        let value = unsafe {
            CFRetained::from_raw(NonNull::new(AXValueCreate(
                AX_POINT,
                ptr::from_ref(&position).cast(),
            ))?)
        };
        if unsafe { AXUIElementSetAttributeValue(window, &position_attribute, &value) } != 0 {
            return None;
        }
    }
    Some(())
}

fn is_settable(window: &CFType, attribute: &CFString) -> bool {
    let mut settable = 0;
    (unsafe { AXUIElementIsAttributeSettable(window, attribute, &mut settable) } == 0) && settable != 0
}
