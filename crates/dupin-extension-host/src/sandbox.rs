//! Confines the helper before it runs guest code. Everything it needs, its executable, its
//! libraries and the socket to Dupin, is open or mapped by then, so the profile denies the rest.

/// Lowers resource limits that no guest needs, as a second layer under the sandbox.
pub fn limit_resources() {
    let limits = [
        (libc::RLIMIT_CORE, 0),
        (libc::RLIMIT_FSIZE, 0),
        (libc::RLIMIT_NPROC, 0),
        // The socket and the standard streams are already open; nothing else may be.
        (libc::RLIMIT_NOFILE, 8),
        #[cfg(target_os = "linux")]
        (libc::RLIMIT_AS, 512 * 1024 * 1024),
    ];
    for (resource, value) in limits {
        let limit = libc::rlimit {
            rlim_cur: value,
            rlim_max: value,
        };
        // SAFETY: `limit` is a valid rlimit for the duration of the call.
        unsafe { libc::setrlimit(resource, &limit) };
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{CStr, CString, c_char, c_int};

    unsafe extern "C" {
        fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
        fn sandbox_free_error(errorbuf: *mut c_char);
    }

    /// Seatbelt's deny-by-default profile, with nothing allowed back: no file, network,
    /// process, Mach service or IPC operation. Reading and writing descriptors that are already
    /// open, allocating memory and running threads need no permission.
    const PROFILE: &str = "(version 1)\n(deny default)\n";

    pub fn enter() -> Result<(), String> {
        let profile = CString::new(PROFILE).expect("no NUL in the profile");
        let mut error = std::ptr::null_mut();
        // SAFETY: `profile` is NUL-terminated and outlives the call; `error` receives a
        // string that only `sandbox_free_error` releases. Flags 0 means `profile` is SBPL.
        let status = unsafe { sandbox_init(profile.as_ptr(), 0, &mut error) };
        if status == 0 {
            return Ok(());
        }
        let reason = if error.is_null() {
            "the sandbox profile was refused".to_owned()
        } else {
            // SAFETY: a non-null `error` is a NUL-terminated string from sandbox_init.
            let reason = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
            // SAFETY: `error` came from sandbox_init and is freed once.
            unsafe { sandbox_free_error(error) };
            reason
        };
        Err(reason)
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    /// Without a sandbox the helper would run third-party code with the user's authority, so it
    /// refuses. Linux needs its own confinement (seccomp-bpf with Landlock or namespaces) first.
    pub fn enter() -> Result<(), String> {
        Err("extensions need a sandbox, and Dupin has none for this platform yet".into())
    }
}

pub use platform::enter;
