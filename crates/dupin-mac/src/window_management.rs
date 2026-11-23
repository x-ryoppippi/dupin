use std::ffi::c_void;
use std::ptr::{self, NonNull};

use dupin_core::window_management::{Display, Frame, Layout, target_frame};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSWorkspace};
use objc2_core_foundation::{CFGetTypeID, CFRetained, CFString, CFType, CFTypeID};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use crate::permission::Permission;

const MESSAGING_TIMEOUT_SECONDS: f32 = 0.25;
const AX_ATTRIBUTE_UNSUPPORTED: i32 = -25205;
const AX_NO_VALUE: i32 = -25212;
const AX_POINT: u32 = 1;
const AX_SIZE: u32 = 2;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> *mut CFType;
    fn AXUIElementGetTypeID() -> CFTypeID;
    fn AXUIElementSetMessagingTimeout(element: &CFType, seconds: f32) -> i32;
    fn AXUIElementCopyAttributeValue(element: &CFType, name: &CFString, value: &mut *mut CFType) -> i32;
    fn AXUIElementIsAttributeSettable(element: &CFType, name: &CFString, settable: &mut u8) -> i32;
    fn AXUIElementSetAttributeValue(element: &CFType, name: &CFString, value: &CFType) -> i32;
    fn AXValueGetTypeID() -> CFTypeID;
    fn AXValueGetValue(value: &CFType, kind: u32, out: *mut c_void) -> u8;
    fn AXValueCreate(kind: u32, value: *const c_void) -> *mut CFType;
}

#[derive(Clone, Copy)]
pub enum Error {
    PermissionRequired,
    NoWindow,
    NoDisplay,
    WindowUnavailable,
    FrameRejected,
}

impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Self::PermissionRequired => "Dupin needs Accessibility access to move and resize windows.",
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
    let permission = Permission::Accessibility;
    if !permission.is_granted() {
        permission.request_once();
        return Err(Error::PermissionRequired);
    }

    arrange_trusted(pid, layout, displays).map_err(|error| {
        if permission.is_granted() {
            error
        } else {
            Error::PermissionRequired
        }
    })
}

fn arrange_trusted(pid: i32, layout: Layout, displays: &[Display]) -> Result<(), Error> {
    let app = unsafe { NonNull::new(AXUIElementCreateApplication(pid)).map(|app| CFRetained::from_raw(app)) }
        .ok_or(Error::WindowUnavailable)?;
    let app = timed(app)?;
    let window = timed(focused_window(&app)?)?;
    let initial = read_frame(&window).ok_or(Error::WindowUnavailable)?;
    let target = target_frame(layout, initial, displays).ok_or(Error::NoDisplay)?;
    update_frame(&window, initial, target).ok_or(Error::FrameRejected)
}

fn timed(element: CFRetained<CFType>) -> Result<CFRetained<CFType>, Error> {
    if CFGetTypeID(Some(&element)) != unsafe { AXUIElementGetTypeID() } {
        return Err(Error::WindowUnavailable);
    }
    // A timeout on the system-wide element would affect other Accessibility features.
    (unsafe { AXUIElementSetMessagingTimeout(&element, MESSAGING_TIMEOUT_SECONDS) } == 0)
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

fn attribute(element: &CFType, name: &str) -> Result<CFRetained<CFType>, i32> {
    let mut value = ptr::null_mut();
    let status = unsafe { AXUIElementCopyAttributeValue(element, &CFString::from_str(name), &mut value) };
    if status != 0 {
        return Err(status);
    }
    Ok(unsafe { CFRetained::from_raw(NonNull::new(value).ok_or(AX_NO_VALUE)?) })
}

fn read_frame(window: &CFType) -> Option<Frame> {
    let position = attribute(window, "AXPosition").ok()?;
    let size = attribute(window, "AXSize").ok()?;
    let value_type = unsafe { AXValueGetTypeID() };
    if CFGetTypeID(Some(&position)) != value_type || CFGetTypeID(Some(&size)) != value_type {
        return None;
    }
    let mut point = NSPoint::ZERO;
    let mut extent = NSSize::ZERO;
    unsafe {
        if AXValueGetValue(&position, AX_POINT, ptr::from_mut(&mut point).cast()) == 0
            || AXValueGetValue(&size, AX_SIZE, ptr::from_mut(&mut extent).cast()) == 0
        {
            return None;
        }
    }
    let frame = Frame::new(point.x, point.y, extent.width, extent.height);
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
