//! The local wall-clock time, from the C library's time zone.

use dupin_core::snippets::LocalTime;

pub fn now() -> LocalTime {
    // SAFETY: `time` accepts a null pointer and only returns the current time.
    let seconds = unsafe { libc::time(std::ptr::null_mut()) };
    // SAFETY: `tm` is plain data that `localtime_r` fills in.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: Both pointers are valid for the call; `localtime_r` is the thread-safe variant.
    unsafe { libc::localtime_r(&seconds, &mut tm) };
    LocalTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u8,
        day: tm.tm_mday as u8,
        hour: tm.tm_hour as u8,
        minute: tm.tm_min as u8,
    }
}
