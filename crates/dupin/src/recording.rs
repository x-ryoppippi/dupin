//! A real launch that is only being filmed, as `scripts/docs-glass.nu` does to show Liquid
//! Glass, which a headless render can't draw. It draws on fixtures as a headless run does: only
//! the folders `DUPIN_HEADLESS_APPS` names are scanned for apps, and the usage history and the
//! other stores live in a scratch storage under a throwaway key, never the Keychain. The
//! clipboard history is the `DUPIN_HEADLESS_CLIPBOARD` fixture seeded into that storage, and the
//! real pasteboard is never watched or read.
//!
//! Both of these hide the user's own apps and history, so a launch is a recording only when it
//! says so with `DUPIN_RECORDING` and its `HOME` is not the account's own: a Finder, login item
//! or shell launch always has the account's `HOME`, so a stray variable changes nothing there.

use std::ffi::{CStr, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::OnceLock;

pub const RECORDING: &str = "DUPIN_RECORDING";

/// Whether this launch is a recording.
pub fn active() -> bool {
    static ACTIVE: OnceLock<bool> = OnceLock::new();
    *ACTIVE.get_or_init(|| {
        std::env::var_os(RECORDING).is_some_and(|value| value == "1")
            && std::env::var_os("HOME")
                .is_some_and(|home| account_home().is_some_and(|account| account != home))
    })
}

/// The home folder the account database gives this user, which `HOME` can't change.
fn account_home() -> Option<PathBuf> {
    let mut buffer = vec![0u8; 4096];
    // SAFETY: A zeroed `passwd` is a valid out-parameter for `getpwuid_r`.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: `entry` and `buffer` outlive the call, and the buffer's length is passed with it.
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() || entry.pw_dir.is_null() {
        return None;
    }
    // SAFETY: On success `pw_dir` points at a NUL-terminated string inside `buffer`.
    let dir = unsafe { CStr::from_ptr(entry.pw_dir) };
    Some(PathBuf::from(OsStr::from_bytes(dir.to_bytes())))
}
