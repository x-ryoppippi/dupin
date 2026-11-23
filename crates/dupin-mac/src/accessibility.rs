//! Whether Dupin may use the Accessibility API.

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
}

pub const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

/// Reads the current grant without prompting.
pub fn is_trusted() -> bool {
    // SAFETY: A status query with no arguments.
    unsafe { AXIsProcessTrusted() != 0 }
}
