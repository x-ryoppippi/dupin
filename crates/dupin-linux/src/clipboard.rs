//! The clipboard. Dupin doesn't read or write it on Linux yet, so the extension `clipboard`
//! capability reads as restricted.

use std::path::PathBuf;

use dupin_core::Item;
use dupin_host_services::clipboard::{self as service, Limits, Pasteboard, Peek, Watch, Write};
use dupin_host_services::{OsAccess, OsPermission, ServiceError};

const UNSUPPORTED: &str = "The clipboard isn't supported on Linux yet.";

/// Whether the system lets Dupin read the clipboard in the background.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    #[default]
    Allowed,
    Unasked,
    Restricted,
}

/// Linux has no clipboard permission, so nothing keeps clipboard history from asking.
pub fn access() -> Access {
    Access::Allowed
}

/// No item: Linux has no clipboard permission to ask for.
pub fn access_item(_access: Access) -> Option<Item> {
    None
}

pub fn request_access() {}

pub fn copy_files(_paths: &[PathBuf]) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

#[derive(Default)]
pub struct GeneralPasteboard(());

impl Pasteboard for GeneralPasteboard {
    fn access(&self) -> OsAccess {
        OsAccess::Restricted
    }

    fn peek(&self) -> Peek {
        Peek {
            change: 0,
            types: Vec::new(),
            apps: None,
        }
    }

    fn source(&self) -> Option<String> {
        None
    }

    fn read(&self, _change: i64, _limits: &Limits) -> Option<service::Content> {
        None
    }

    fn write(&self, _content: &Write) -> Result<(), ServiceError> {
        Err(ServiceError::PermissionRestricted(OsPermission::Pasteboard))
    }

    fn watch(&self, _changed: Box<dyn Fn(Peek) + Send + Sync>) -> Result<Watch, ServiceError> {
        Err(ServiceError::PermissionRestricted(OsPermission::Pasteboard))
    }
}
