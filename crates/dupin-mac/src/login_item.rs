use std::ffi::OsStr;
use std::path::Path;

use objc2::{MainThreadMarker, rc::Retained};
use objc2_service_management::SMAppService;
pub use objc2_service_management::SMAppServiceStatus as Status;

pub struct LoginItem(Retained<SMAppService>);

impl LoginItem {
    /// Resolve the current app's login item when running from an eligible bundle.
    pub fn for_current_app(_: MainThreadMarker) -> Option<Self> {
        let bundle = crate::bundle::path().canonicalize().ok()?;
        if !supports_login_item(&bundle) {
            return None;
        }
        crate::bundle::info_string("CFBundleIdentifier")?;
        // SAFETY: Dupin requires macOS 14, and this is its main application bundle.
        Some(Self(unsafe { SMAppService::mainAppService() }))
    }

    pub fn status(&self) -> Status {
        // SAFETY: The service represents the running app's main bundle.
        unsafe { self.0.status() }
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        // SAFETY: The service represents the running app's main bundle.
        let result = unsafe {
            if enabled {
                self.0.registerAndReturnError()
            } else {
                self.0.unregisterAndReturnError()
            }
        };
        result.map_err(|error| error.localizedDescription().to_string())
    }

    pub fn open_settings(&self) {
        // SAFETY: The service is only constructed on the main thread on supported macOS.
        unsafe { SMAppService::openSystemSettingsLoginItems() }
    }
}

fn supports_login_item(bundle: &Path) -> bool {
    bundle.extension() == Some(OsStr::new("app")) && !bundle.starts_with("/nix/store")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_item_requires_an_app_bundle_outside_the_nix_store() {
        assert!(supports_login_item(Path::new("/Applications/Dupin.app")));
        assert!(!supports_login_item(Path::new("/repo/target/debug")));
        assert!(!supports_login_item(Path::new(
            "/nix/store/hash-dupin/Applications/Dupin.app"
        )));
    }
}
