//! The Accessibility side of the extensions' `windows` capability: the displays, and the
//! focused window of the app that was in front of Dupin when the panel opened.

use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::Mutex;

use dupin_host_services::ServiceError;
use dupin_host_services::windows::{self, App, Display, Frame, Window};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSWorkspace};
use objc2_core_foundation::{CFRetained, CFString, CFType};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use crate::accessibility::{self, AX_NO_VALUE, AX_POINT, AX_SIZE, attribute};
use crate::privacy::Permission;

const MESSAGING_TIMEOUT_SECONDS: f32 = 0.25;
const AX_ATTRIBUTE_UNSUPPORTED: i32 = -25205;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementIsAttributeSettable(element: &CFType, name: &CFString, settable: &mut u8) -> i32;
    fn AXUIElementSetAttributeValue(element: &CFType, name: &CFString, value: &CFType) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> *mut CFType;
}

#[derive(Clone)]
struct Target {
    pid: i32,
    app: App,
}

#[derive(Clone, Default)]
struct Captured {
    displays: Vec<Display>,
    /// None while Dupin itself was in front, since its own windows are never arranged.
    target: Option<Target>,
}

impl Captured {
    fn new(displays: Vec<Display>, front: Option<Target>) -> Self {
        let target = front.filter(|target| target.pid > 0 && target.pid as u32 != std::process::id());
        Self { displays, target }
    }
}

/// What was in front of Dupin when the panel last opened: the app whose focused window the
/// `windows` capability reads and moves, and the displays then, which AppKit reads only on the
/// main thread.
#[derive(Default)]
pub struct Front(Mutex<Captured>);

impl Front {
    /// Notes the displays and the frontmost app. Call it on the main thread as the panel opens,
    /// while the app the user was in is still in front.
    pub fn capture(&self) {
        let Some(main) = MainThreadMarker::new() else {
            return;
        };
        let front = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| Target {
                pid: app.processIdentifier(),
                app: App {
                    name: app
                        .localizedName()
                        .map(|name| name.to_string())
                        .unwrap_or_default(),
                    bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
                },
            });
        *self.lock() = Captured::new(displays(main), front);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Captured> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn displays(&self) -> Vec<Display> {
        self.lock().displays.clone()
    }

    fn target(&self) -> Option<Target> {
        self.lock().target.clone()
    }
}

/// Display bounds in top-left coordinates, with the primary display first.
fn displays(main: MainThreadMarker) -> Vec<Display> {
    let screens = NSScreen::screens(main);
    let Some(primary) = screens.firstObject() else {
        return Vec::new();
    };
    let primary_top = primary.frame().max().y;
    let convert = |rect: NSRect| {
        Frame::new(
            rect.origin.x,
            primary_top - rect.origin.y - rect.size.height,
            rect.size.width,
            rect.size.height,
        )
    };
    screens
        .iter()
        .map(|screen| Display {
            frame: convert(screen.frame()),
            visible_frame: convert(screen.visibleFrame()),
            scale: screen.backingScaleFactor(),
        })
        .collect()
}

/// Reads and moves windows through the Accessibility API. Each call may wait on the app in
/// front, so it runs on a worker, never on the main thread.
pub struct Windows(pub std::sync::Arc<Front>);

impl windows::Windows for Windows {
    fn access(&self) -> dupin_host_services::OsAccess {
        Permission::Accessibility.status().os_access()
    }

    fn displays(&self) -> Result<Vec<Display>, ServiceError> {
        Ok(self.0.displays())
    }

    fn focused(&self) -> Result<Option<Window>, ServiceError> {
        let Some(target) = self.0.target() else {
            return Ok(None);
        };
        let Some(window) = trusted(focused_window(target.pid))? else {
            return Ok(None);
        };
        let frame = read_frame(&window).ok_or_else(unreadable)?;
        Ok(Some(Window {
            frame,
            app: target.app,
        }))
    }

    fn set_frame(&self, frame: Frame) -> Result<Frame, ServiceError> {
        let target = self.0.target().ok_or_else(no_window)?;
        let window = trusted(focused_window(target.pid))?.ok_or_else(no_window)?;
        let initial = read_frame(&window).ok_or_else(unreadable)?;
        trusted(update_frame(&window, initial, frame).ok_or_else(|| {
            ServiceError::Failed("The window couldn't be moved or resized as requested.".into())
        }))?;
        Ok(read_frame(&window).unwrap_or(frame))
    }
}

fn no_window() -> ServiceError {
    ServiceError::Failed("There is no focused window to arrange.".into())
}

fn unreadable() -> ServiceError {
    ServiceError::Failed("Dupin couldn’t read the focused window.".into())
}

/// A failure while macOS no longer trusts Dupin is that, not whatever the call reported.
fn trusted<T>(result: Result<T, ServiceError>) -> Result<T, ServiceError> {
    result.map_err(|error| {
        ServiceError::from_access(
            windows::PERMISSION,
            Permission::Accessibility.status().os_access(),
        )
        .unwrap_or(error)
    })
}

/// The app's focused window, or its main window when it reports no focused one.
fn focused_window(pid: i32) -> Result<Option<CFRetained<CFType>>, ServiceError> {
    let app = accessibility::application(pid).ok_or_else(unreadable)?;
    let app = timed(app)?;
    let window = match attribute(&app, "AXFocusedWindow") {
        Err(AX_NO_VALUE | AX_ATTRIBUTE_UNSUPPORTED) => match attribute(&app, "AXMainWindow") {
            Err(AX_NO_VALUE | AX_ATTRIBUTE_UNSUPPORTED) => return Ok(None),
            result => result.map_err(|_| unreadable())?,
        },
        result => result.map_err(|_| unreadable())?,
    };
    timed(window).map(Some)
}

fn timed(element: CFRetained<CFType>) -> Result<CFRetained<CFType>, ServiceError> {
    // A timeout on the system-wide element would affect other Accessibility features.
    accessibility::set_timeout(&element, MESSAGING_TIMEOUT_SECONDS)
        .then_some(element)
        .ok_or_else(unreadable)
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

#[cfg(test)]
mod tests {
    use dupin_host_services::windows::Windows as _;

    use super::*;

    fn display() -> Display {
        let frame = Frame::new(0.0, 0.0, 1440.0, 900.0);
        Display {
            frame,
            visible_frame: frame,
            scale: 2.0,
        }
    }

    #[test]
    fn with_dupin_in_front_the_displays_are_still_read_but_there_is_no_window() {
        let dupin = Target {
            pid: std::process::id() as i32,
            app: App {
                name: "Dupin".into(),
                bundle_id: None,
            },
        };
        let front = Front::default();
        *front.lock() = Captured::new(vec![display()], Some(dupin));
        let windows = Windows(std::sync::Arc::new(front));
        assert_eq!(windows.displays(), Ok(vec![display()]));
        assert_eq!(windows.focused(), Ok(None));
        assert_eq!(
            windows.set_frame(Frame::new(0.0, 0.0, 100.0, 100.0)),
            Err(no_window())
        );
    }

    #[test]
    fn nothing_captured_reads_as_no_window_and_no_displays() {
        let windows = Windows(Default::default());
        assert_eq!(windows.displays(), Ok(Vec::new()));
        assert_eq!(windows.focused(), Ok(None));
        assert!(matches!(
            windows.set_frame(Frame::new(0.0, 0.0, 100.0, 100.0)),
            Err(ServiceError::Failed(_))
        ));
    }
}
