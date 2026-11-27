use std::ptr::{self, NonNull};

use objc2_core_foundation::{CFArray, CFRetained, CFString, CFType};
use objc2_foundation::NSRect;

use crate::accessibility::{self, read_frame};

// The launcher waits for two messages before it can open on the chosen display.
const MESSAGING_TIMEOUT_SECONDS: f32 = 0.020;
const STOP_ON_ERROR: u32 = 1;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCopyMultipleAttributeValues(
        element: &CFType,
        attributes: &CFArray<CFString>,
        options: u32,
        values: &mut *mut CFArray<CFType>,
    ) -> i32;
}

pub(crate) fn frame(pid: i32) -> Option<NSRect> {
    let app = timed(accessibility::application(pid)?)?;
    let window = accessibility::attribute(&app, "AXFocusedWindow").ok()?;
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
    // Bound calls to the foreground app without changing the process-wide timeout.
    accessibility::set_timeout(&element, MESSAGING_TIMEOUT_SECONDS).then_some(element)
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
