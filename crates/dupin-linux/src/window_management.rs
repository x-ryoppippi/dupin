//! The extensions' `windows` capability, which Dupin doesn't back on Linux yet: it reads as
//! restricted and moves no window.

use std::sync::Arc;

use dupin_host_services::windows::{self, Display, Frame, Window};
use dupin_host_services::{OsAccess, OsPermission, ServiceError};

/// What was in front of Dupin when the panel last opened. Linux notes nothing.
#[derive(Default)]
pub struct Front(());

impl Front {
    pub fn capture(&self) {}
}

pub struct Windows(pub Arc<Front>);

impl windows::Windows for Windows {
    fn access(&self) -> OsAccess {
        OsAccess::Restricted
    }

    fn displays(&self) -> Result<Vec<Display>, ServiceError> {
        Err(restricted())
    }

    fn focused(&self) -> Result<Option<Window>, ServiceError> {
        Err(restricted())
    }

    fn set_frame(&self, _frame: Frame) -> Result<Frame, ServiceError> {
        Err(restricted())
    }
}

fn restricted() -> ServiceError {
    ServiceError::PermissionRestricted(OsPermission::Accessibility)
}
