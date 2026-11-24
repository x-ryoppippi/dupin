//! The display under the pointer, read from the X server: Rune reports displays only at
//! startup, and placing the panel at a display's center needs the current one.

use std::ffi::c_void;
use std::ptr::NonNull;

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};

/// Why the panel couldn't open on the display the settings ask for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DisplayNotice {
    AccessibilityRequired,
    FocusedWindowUnavailable,
}

/// Linux panels have no AppKit view, so none is the key window.
///
/// # Safety
///
/// Nothing is dereferenced; the signature matches macOS's.
pub unsafe fn is_key_window(_view: NonNull<c_void>) -> bool {
    false
}

/// The height in points of the display the pointer is on, where Rune's
/// `DisplayTarget::UnderPointer` places the panel. None without an X server (a Wayland
/// session) or when no monitor holds the pointer.
pub fn pointer_display_height() -> Option<f64> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let pointer = conn.query_pointer(root).ok()?.reply().ok()?;
    let (x, y) = (i32::from(pointer.root_x), i32::from(pointer.root_y));
    let monitors = conn.randr_get_monitors(root, true).ok()?.reply().ok()?;
    let monitor = monitors.monitors.iter().find(|m| {
        let (left, top) = (i32::from(m.x), i32::from(m.y));
        (left..left + i32::from(m.width)).contains(&x) && (top..top + i32::from(m.height)).contains(&y)
    })?;
    let physical = physical_scale(
        (u32::from(monitor.width), u32::from(monitor.height)),
        (monitor.width_in_millimeters, monitor.height_in_millimeters),
    );
    Some(f64::from(monitor.height) / scale(&conn, root, physical))
}

/// The scale winit gives X11 windows: `WINIT_X11_SCALE_FACTOR` when it is a number or
/// `randr`, else `Xft.dpi` over 96, else the monitor's physical pixel density.
fn scale(conn: &impl Connection, root: u32, physical: f64) -> f64 {
    match std::env::var("WINIT_X11_SCALE_FACTOR").ok().as_deref() {
        Some(v) if v.eq_ignore_ascii_case("randr") => return physical,
        Some(v) => {
            if let Some(factor) = v.parse::<f64>().ok().filter(|f| *f > 0.0) {
                return factor;
            }
        }
        None => {}
    }
    conn.get_property(
        false,
        root,
        AtomEnum::RESOURCE_MANAGER,
        AtomEnum::STRING,
        0,
        1 << 16,
    )
    .ok()
    .and_then(|cookie| cookie.reply().ok())
    .and_then(|reply| xft_dpi(&String::from_utf8_lossy(&reply.value)))
    .map_or(physical, |dpi| dpi / 96.0)
}

/// winit's scale from a monitor's size in pixels and millimeters: the pixel density over 96 dpi
/// in steps of 1/12, at least 1, and 1 for a monitor reporting no size or an implausible one.
fn physical_scale((width_px, height_px): (u32, u32), (width_mm, height_mm): (u32, u32)) -> f64 {
    if width_mm == 0 || height_mm == 0 {
        return 1.0;
    }
    let ppmm =
        ((f64::from(width_px) * f64::from(height_px)) / (f64::from(width_mm) * f64::from(height_mm))).sqrt();
    let factor = ((ppmm * (12.0 * 25.4 / 96.0)).round() / 12.0).max(1.0);
    if factor <= 20.0 { factor } else { 1.0 }
}

fn xft_dpi(resources: &str) -> Option<f64> {
    resources.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == "Xft.dpi")
            .then(|| value.trim().parse().ok())
            .flatten()
            .filter(|d: &f64| *d > 0.0)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_xft_dpi_from_the_resource_database() {
        assert_eq!(super::xft_dpi("Xft.antialias:\t1\nXft.dpi:\t192\n"), Some(192.0));
        assert_eq!(super::xft_dpi("Xft.antialias:\t1\n"), None);
    }

    #[test]
    fn physical_scale_follows_the_monitor_size() {
        assert_eq!(super::physical_scale((3840, 2160), (508, 286)), 2.0);
        assert_eq!(super::physical_scale((1920, 1080), (527, 296)), 1.0);
        assert_eq!(super::physical_scale((1920, 1080), (0, 0)), 1.0);
    }
}
