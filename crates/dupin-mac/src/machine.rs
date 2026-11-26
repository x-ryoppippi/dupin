//! What identifies this Mac to sync: its hardware UUID, which tells a data directory moved to
//! other hardware, and the name the user gave it. Also asks iCloud Drive to download a file
//! that is only a placeholder.

use std::ffi::{CString, c_char, c_void};
use std::ptr::{self, NonNull};

use objc2_core_foundation::{CFRetained, CFString, CFType};
use objc2_foundation::{NSFileManager, NSString, NSURL};

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main_port: u32, matching: *mut c_void) -> u32;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: &CFString,
        allocator: *const c_void,
        options: u32,
    ) -> *const CFType;
    fn IOObjectRelease(object: u32) -> i32;
}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCopyComputerName(store: *const c_void, encoding: *mut u32) -> *const CFString;
}

/// `IOPlatformUUID`, the hardware UUID System Information shows.
pub fn platform_uuid() -> Option<String> {
    let name = CString::new("IOPlatformExpertDevice").expect("no NUL");
    // SAFETY: A NUL-terminated class name; the matching dictionary is consumed by the lookup.
    let service = unsafe { IOServiceGetMatchingService(0, IOServiceMatching(name.as_ptr())) };
    if service == 0 {
        return None;
    }
    let key = CFString::from_str("IOPlatformUUID");
    // SAFETY: A valid registry entry and key; the property follows the Create rule.
    let value = unsafe { IORegistryEntryCreateCFProperty(service, &key, ptr::null(), 0) };
    // SAFETY: The entry was returned retained by `IOServiceGetMatchingService`.
    unsafe { IOObjectRelease(service) };
    let value = NonNull::new(value.cast_mut())?;
    // SAFETY: Owned per the Create rule.
    let value = unsafe { CFRetained::from_raw(value) };
    Some(value.downcast_ref::<CFString>()?.to_string())
}

/// The Mac's name from Sharing settings, such as "Work MacBook Pro".
pub fn computer_name() -> Option<String> {
    // SAFETY: A null store asks for the current one; the name follows the Copy rule.
    let name = unsafe { SCDynamicStoreCopyComputerName(ptr::null(), ptr::null_mut()) };
    let name = NonNull::new(name.cast_mut())?;
    // SAFETY: Owned per the Copy rule.
    let name = unsafe { CFRetained::from_raw(name) };
    Some(name.to_string())
}

/// Asks iCloud Drive to download a file that is only a placeholder. It never blocks on the
/// download, and a path outside iCloud Drive is refused by the system and ignored.
pub fn request_download(path: &std::path::Path) {
    let Some(path) = path.to_str() else {
        return;
    };
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let _ = NSFileManager::defaultManager().startDownloadingUbiquitousItemAtURL_error(&url);
}
