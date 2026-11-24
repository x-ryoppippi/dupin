//! The `windows` capability: the displays, the focused window of the app that was in front of
//! Dupin, and moving and resizing that window.
//!
//! Frames are in points, in one space for every display: the origin is the top-left corner of
//! the primary display, which comes first, and `y` grows downwards, as the Accessibility API
//! places windows. Reading the focused window and moving it need macOS Accessibility access.
//! The service only reads its status, which never prompts: while macOS hasn't been asked, a
//! call fails with [`ServiceError::NeedsPermission`], and only the app asks, from a deliberate
//! action such as Request Access.

use std::sync::Mutex;

use dupin_broker::{OsAccess, OsPermission};
use serde::{Deserialize, Serialize};

use crate::service::{ServiceError, bounded};

pub const PERMISSION: OsPermission = OsPermission::Accessibility;
pub const MAX_DISPLAYS: usize = 32;
/// How far from the primary display's origin a frame may start, in points.
pub const MAX_COORDINATE: f64 = 1_000_000.0;
/// The widest or tallest a frame may be, in points.
pub const MAX_LENGTH: f64 = 100_000.0;
pub const MAX_NAME_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    /// Finite, within [`MAX_COORDINATE`] of the origin and with a size from just above 0 to
    /// [`MAX_LENGTH`].
    pub fn is_valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .into_iter()
            .all(f64::is_finite)
            && self.x.abs() <= MAX_COORDINATE
            && self.y.abs() <= MAX_COORDINATE
            && self.width > 0.0
            && self.height > 0.0
            && self.width <= MAX_LENGTH
            && self.height <= MAX_LENGTH
    }

    fn overlaps(self, other: Self) -> bool {
        let width = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let height = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        width > 0.0 && height > 0.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Display {
    pub frame: Frame,
    /// The part windows may use, outside the menu bar and the Dock.
    pub visible_frame: Frame,
    /// Pixels per point.
    pub scale: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct App {
    pub name: String,
    #[serde(default)]
    pub bundle_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub frame: Frame,
    pub app: App,
}

/// Where windows are read and moved: the Accessibility API on macOS, [`Fixture`] in tests.
pub trait Windows: Send + Sync {
    /// macOS's answer for Accessibility access, read without prompting.
    fn access(&self) -> OsAccess;
    /// Every display, the primary one first.
    fn displays(&self) -> Result<Vec<Display>, ServiceError>;
    /// The focused window of the app in front of Dupin, if it has one.
    fn focused(&self) -> Result<Option<Window>, ServiceError>;
    /// Moves and resizes that window to `frame`, and returns the frame it took, which an app
    /// may round or constrain.
    fn set_frame(&self, frame: Frame) -> Result<Frame, ServiceError>;
}

pub struct Service<W> {
    backend: W,
}

impl<W: Windows> Service<W> {
    pub fn new(backend: W) -> Self {
        Self { backend }
    }

    pub fn backend(&self) -> &W {
        &self.backend
    }

    /// The displays, which reading needs no access to.
    pub fn displays(&self) -> Result<Vec<Display>, ServiceError> {
        let mut displays: Vec<Display> = self
            .backend
            .displays()?
            .into_iter()
            .filter(|display| {
                display.frame.is_valid()
                    && display.visible_frame.is_valid()
                    && display.scale.is_finite()
                    && display.scale > 0.0
            })
            .collect();
        displays.truncate(MAX_DISPLAYS);
        Ok(displays)
    }

    pub fn focused(&self) -> Result<Option<Window>, ServiceError> {
        self.allowed()?;
        Ok(self
            .backend
            .focused()?
            .filter(|window| window.frame.is_valid())
            .map(|window| Window {
                app: App {
                    name: bounded(window.app.name, MAX_NAME_BYTES),
                    bundle_id: window.app.bundle_id.filter(|id| id.len() <= MAX_NAME_BYTES),
                },
                ..window
            }))
    }

    /// Moves the focused window to `frame`, which must lie at least partly on a display.
    pub fn set_frame(&self, frame: Frame) -> Result<Frame, ServiceError> {
        if !frame.is_valid() {
            return Err(ServiceError::Invalid(format!(
                "a frame is finite, within {MAX_COORDINATE} points of the primary display and \
                 from above 0 to {MAX_LENGTH} points wide and tall"
            )));
        }
        let displays = self.displays()?;
        if !displays.iter().any(|display| display.frame.overlaps(frame)) {
            return Err(ServiceError::OutOfScope(
                "the frame lies off every display".into(),
            ));
        }
        self.allowed()?;
        self.backend.set_frame(frame)
    }

    fn allowed(&self) -> Result<(), ServiceError> {
        match ServiceError::from_access(PERMISSION, self.backend.access()) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

/// Fixed displays and a focused window that [`Windows::set_frame`] moves, recording each move.
#[derive(Debug)]
pub struct Fixture {
    pub access: OsAccess,
    pub displays: Vec<Display>,
    pub focused: Mutex<Option<Window>>,
    pub moves: Mutex<Vec<Frame>>,
}

impl Fixture {
    pub fn new(displays: Vec<Display>, focused: Option<Window>) -> Self {
        Self {
            access: OsAccess::Allowed,
            displays,
            focused: Mutex::new(focused),
            moves: Mutex::default(),
        }
    }
}

impl Windows for Fixture {
    fn access(&self) -> OsAccess {
        self.access
    }

    fn displays(&self) -> Result<Vec<Display>, ServiceError> {
        Ok(self.displays.clone())
    }

    fn focused(&self) -> Result<Option<Window>, ServiceError> {
        Ok(self
            .focused
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone())
    }

    fn set_frame(&self, frame: Frame) -> Result<Frame, ServiceError> {
        let mut focused = self
            .focused
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let window = focused
            .as_mut()
            .ok_or_else(|| ServiceError::Failed("There is no focused window to arrange.".into()))?;
        window.frame = frame;
        self.moves
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(frame);
        Ok(frame)
    }
}

#[cfg(test)]
mod tests;
