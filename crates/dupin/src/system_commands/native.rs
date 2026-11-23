use std::ffi::{CStr, c_long, c_void};
use std::ptr::{self, NonNull};

use dupin_core::SystemCommand;

const TYPE_APPLICATION_BUNDLE_ID: u32 = 0x6275_6e64;
// Finder replies after it empties the Trash. Large deletions can outlast the default timeout.
const EMPTY_TRASH_REPLY_TIMEOUT_TICKS: c_long = 5 * 60 * 60;

pub fn run(command: SystemCommand) -> Result<(), String> {
    use SystemCommand::*;
    match command {
        Sleep => sleep(),
        LockScreen => lock_screen(),
        SleepDisplays => sleep_displays(),
        Restart => loginwindow(*b"rest"),
        ShutDown => loginwindow(*b"shut"),
        LogOut => loginwindow(*b"rlgo"),
        EmptyTrash => {
            let finder = Descriptor::address(TYPE_APPLICATION_BUNDLE_ID, b"com.apple.finder")?;
            send_event(
                &finder,
                "Finder",
                *b"fndr",
                *b"empt",
                Some(EMPTY_TRASH_REPLY_TIMEOUT_TICKS),
            )
        }
        ToggleDarkMode => toggle_dark_mode(),
    }
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMFindPowerManagement(port: u32) -> u32;
    fn IOPMSleepSystem(connection: u32) -> i32;
    fn IOServiceClose(connection: u32) -> i32;
}

fn sleep() -> Result<(), String> {
    // SAFETY: The API accepts MACH_PORT_NULL, and the resulting connection is closed after use.
    unsafe {
        let connection = IOPMFindPowerManagement(0);
        if connection == 0 {
            return Err("Couldn't connect to macOS power management.".into());
        }
        let status = IOPMSleepSystem(connection);
        IOServiceClose(connection);
        if status != 0 {
            return Err(format!(
                "macOS rejected the sleep request (IOKit {status:#010x})."
            ));
        }
    }
    Ok(())
}

fn sleep_displays() -> Result<(), String> {
    let output = std::process::Command::new("/usr/bin/pmset")
        .arg("displaysleepnow")
        .output()
        .map_err(|error| error.to_string())?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    // pmset reports some display sleep failures on stderr with a zero exit status.
    if !stderr.is_empty() {
        return Err(stderr.into());
    }
    if !output.status.success() {
        return Err(format!("pmset returned {}.", output.status));
    }
    Ok(())
}

struct Framework(NonNull<c_void>);

impl Framework {
    fn open(path: &CStr) -> Result<Self, String> {
        // SAFETY: The path is NUL-terminated and the handle is owned until Drop.
        NonNull::new(unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LOCAL | libc::RTLD_LAZY) })
            .map(Self)
            .ok_or_else(|| "The macOS framework is unavailable.".into())
    }

    fn symbol(&self, name: &CStr) -> Result<NonNull<c_void>, String> {
        // SAFETY: The framework is still open, and the symbol name is NUL-terminated.
        NonNull::new(unsafe { libc::dlsym(self.0.as_ptr(), name.as_ptr()) })
            .ok_or_else(|| format!("This macOS version does not provide {}.", name.to_string_lossy()))
    }
}

impl Drop for Framework {
    fn drop(&mut self) {
        // SAFETY: A successful dlopen owns one reference to this handle.
        unsafe { libc::dlclose(self.0.as_ptr()) };
    }
}

fn lock_screen() -> Result<(), String> {
    let login = Framework::open(c"/System/Library/PrivateFrameworks/login.framework/login")?;
    let lock = login.symbol(c"SACLockScreenImmediate")?;
    // SAFETY: login.framework exports this function with this signature.
    let lock: unsafe extern "C" fn() = unsafe { std::mem::transmute(lock.as_ptr()) };
    unsafe { lock() };
    Ok(())
}

fn toggle_dark_mode() -> Result<(), String> {
    let skylight = Framework::open(c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight")?;
    let get = skylight.symbol(c"SLSGetAppearanceThemeLegacy")?;
    let set = skylight.symbol(c"SLSSetAppearanceThemeLegacy")?;
    // SAFETY: SkyLight exports these functions with these signatures.
    let get: unsafe extern "C" fn() -> bool = unsafe { std::mem::transmute(get.as_ptr()) };
    let set: unsafe extern "C" fn(bool) = unsafe { std::mem::transmute(set.as_ptr()) };
    unsafe {
        let dark = !get();
        set(dark);
        if get() != dark {
            return Err("macOS did not change the appearance.".into());
        }
    }
    Ok(())
}

#[repr(C)]
struct Descriptor {
    kind: u32,
    data: *mut c_void,
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn AECreateDesc(kind: u32, data: *const c_void, size: c_long, result: *mut Descriptor) -> i16;
    fn AECreateAppleEvent(
        class: u32,
        id: u32,
        target: *const Descriptor,
        return_id: i16,
        transaction_id: i32,
        result: *mut Descriptor,
    ) -> i16;
    fn AESendMessage(event: *const Descriptor, reply: *mut Descriptor, mode: i32, timeout: c_long) -> i32;
    fn AEGetParamPtr(
        event: *const Descriptor,
        keyword: u32,
        kind: u32,
        actual_kind: *mut u32,
        data: *mut c_void,
        size: c_long,
        actual_size: *mut c_long,
    ) -> i16;
    fn AEDisposeDesc(descriptor: *mut Descriptor) -> i16;
}

impl Descriptor {
    const fn empty() -> Self {
        Self {
            kind: u32::from_be_bytes(*b"null"),
            data: ptr::null_mut(),
        }
    }

    fn address(kind: u32, bytes: &[u8]) -> Result<Self, String> {
        let mut address = Self::empty();
        // SAFETY: AECreateDesc copies the address bytes during the call.
        let status =
            unsafe { AECreateDesc(kind, bytes.as_ptr().cast(), bytes.len() as c_long, &mut address) };
        event_status("the event target", i32::from(status))?;
        Ok(address)
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: This is either a null descriptor or an owned Apple Event Manager descriptor.
        unsafe { AEDisposeDesc(self) };
    }
}

fn loginwindow(id: [u8; 4]) -> Result<(), String> {
    // The reserved system process number targets this GUI session's loginwindow.
    let mut psn = [0u8; 8];
    psn[4..].copy_from_slice(&1u32.to_ne_bytes());
    let target = Descriptor::address(u32::from_be_bytes(*b"psn "), &psn)?;
    send_event(&target, "loginwindow", *b"aevt", id, None)
}

fn send_event(
    target: &Descriptor,
    name: &str,
    class: [u8; 4],
    id: [u8; 4],
    reply_timeout_ticks: Option<c_long>,
) -> Result<(), String> {
    const CAN_INTERACT: i32 = 0x20;
    const NO_REPLY: i32 = 1;
    const WAIT_REPLY: i32 = 3;
    const DEFAULT_TIMEOUT: c_long = -1;
    const PARAM_NOT_FOUND: i16 = -1701;
    let mut event = Descriptor::empty();
    // SAFETY: The address is live and the event and reply point to writable descriptors.
    unsafe {
        let status = AECreateAppleEvent(
            u32::from_be_bytes(class),
            u32::from_be_bytes(id),
            target,
            -1,
            0,
            &mut event,
        );
        event_status(name, i32::from(status))?;
        let Some(timeout) = reply_timeout_ticks else {
            // The session can end before loginwindow could send a reply.
            return event_status(
                name,
                AESendMessage(&event, ptr::null_mut(), NO_REPLY | CAN_INTERACT, DEFAULT_TIMEOUT),
            );
        };
        let mut reply = Descriptor::empty();
        let status = AESendMessage(&event, &mut reply, WAIT_REPLY | CAN_INTERACT, timeout);
        if status != 0 {
            // Apple Event Manager only transfers reply ownership on a successful send.
            std::mem::forget(reply);
            return event_status(name, status);
        }
        let mut application_error = 0i32;
        let status = AEGetParamPtr(
            &reply,
            u32::from_be_bytes(*b"errn"),
            u32::from_be_bytes(*b"long"),
            ptr::null_mut(),
            ptr::from_mut(&mut application_error).cast(),
            size_of::<i32>() as c_long,
            ptr::null_mut(),
        );
        if status == PARAM_NOT_FOUND {
            return Ok(());
        }
        event_status(name, i32::from(status))?;
        event_status(name, application_error)
    }
}

fn event_status(target: &str, status: i32) -> Result<(), String> {
    match status {
        0 => Ok(()),
        -1743 => Err(format!(
            "macOS denied Automation access to {target}. Review Dupin in System Settings → Privacy & Security → Automation."
        )),
        _ => Err(format!("Apple event for {target} failed (OSStatus {status}).")),
    }
}
