use std::ffi::c_void;
use std::ptr::{self, NonNull};

use objc2_core_foundation::{CFArray, CFGetTypeID, CFRetained, CFString, CFType, CFTypeID};
use objc2_foundation::{NSPoint, NSRect, NSSize};

// The launcher waits for two messages before it can open on the chosen display.
const MESSAGING_TIMEOUT_SECONDS: f32 = 0.020;
const STOP_ON_ERROR: u32 = 1;
const AX_POINT: u32 = 1;
const AX_SIZE: u32 = 2;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementGetTypeID() -> CFTypeID;
    fn AXUIElementCreateApplication(pid: i32) -> *mut CFType;
    fn AXUIElementSetMessagingTimeout(element: &CFType, timeout: f32) -> i32;
    fn AXUIElementCopyAttributeValue(element: &CFType, attribute: &CFString, value: &mut *mut CFType) -> i32;
    fn AXUIElementCopyMultipleAttributeValues(
        element: &CFType,
        attributes: &CFArray<CFString>,
        options: u32,
        values: &mut *mut CFArray<CFType>,
    ) -> i32;
    fn AXValueGetTypeID() -> CFTypeID;
    fn AXValueGetValue(value: &CFType, kind: u32, out: *mut c_void) -> u8;
}

pub(crate) fn frame(pid: i32) -> Option<NSRect> {
    // SAFETY: The Create call gives us an owned AXUIElement reference for this PID.
    let app = unsafe { CFRetained::from_raw(NonNull::new(AXUIElementCreateApplication(pid))?) };
    let app = timed(app)?;
    let window = attribute(&app, "AXFocusedWindow")?;
    let window = timed(window)?;
    let attributes =
        CFArray::from_objects(&[&*CFString::from_str("AXPosition"), &*CFString::from_str("AXSize")]);
    let mut values = ptr::null_mut();
    // SAFETY: `timed` checked the element type, and Copy returns an owned CF array.
    let values = unsafe {
        if AXUIElementCopyMultipleAttributeValues(&window, &attributes, STOP_ON_ERROR, &mut values) != 0 {
            return None;
        }
        CFRetained::from_raw(NonNull::new(values)?)
    };
    let position = values.get(0)?;
    let size = values.get(1)?;
    read_frame(&position, &size)
}

fn timed(element: CFRetained<CFType>) -> Option<CFRetained<CFType>> {
    // SAFETY: Check the CF type before passing the object to the element-specific API.
    if CFGetTypeID(Some(&element)) != unsafe { AXUIElementGetTypeID() } {
        return None;
    }
    // SAFETY: The check above establishes that this is an AXUIElement.
    // Bound calls to the foreground app without changing the process-wide timeout.
    (unsafe { AXUIElementSetMessagingTimeout(&element, MESSAGING_TIMEOUT_SECONDS) } == 0).then_some(element)
}

fn attribute(element: &CFType, name: &str) -> Option<CFRetained<CFType>> {
    let mut value = ptr::null_mut();
    // SAFETY: Callers pass elements checked by `timed`. Copy returns an owned CF reference.
    unsafe {
        if AXUIElementCopyAttributeValue(element, &CFString::from_str(name), &mut value) != 0 {
            return None;
        }
        Some(CFRetained::from_raw(NonNull::new(value)?))
    }
}

fn read_frame(position: &CFType, size: &CFType) -> Option<NSRect> {
    // SAFETY: Check both CF types before passing them to AXValueGetValue.
    let value_type = unsafe { AXValueGetTypeID() };
    if CFGetTypeID(Some(position)) != value_type || CFGetTypeID(Some(size)) != value_type {
        return None;
    }
    let mut origin = NSPoint::ZERO;
    let mut dimensions = NSSize::ZERO;
    // SAFETY: AXValueGetValue checks the requested kind and writes only its corresponding type.
    unsafe {
        (AXValueGetValue(position, AX_POINT, ptr::from_mut(&mut origin).cast()) != 0
            && AXValueGetValue(size, AX_SIZE, ptr::from_mut(&mut dimensions).cast()) != 0)
            .then_some(NSRect::new(origin, dimensions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_accessibility_objects() {
        let string: CFRetained<CFType> = CFString::from_str("not an AX object").into();
        assert!(read_frame(&string, &string).is_none());
        assert!(timed(string).is_none());
    }
}
