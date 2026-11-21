//! The running app's own bundle: where it is and what its Info.plist says.

use std::path::PathBuf;

use objc2_foundation::{NSBundle, NSString};

/// The main bundle's directory: `…/Dupin.app` when Dupin runs as an app, the executable's
/// directory when it runs bare (`cargo run`).
pub fn path() -> PathBuf {
    PathBuf::from(NSBundle::mainBundle().bundlePath().to_string())
}

/// A string value of the main bundle's Info.plist.
pub fn info_string(key: &str) -> Option<String> {
    let value = NSBundle::mainBundle().objectForInfoDictionaryKey(&NSString::from_str(key))?;
    value.downcast::<NSString>().ok().map(|s| s.to_string())
}
