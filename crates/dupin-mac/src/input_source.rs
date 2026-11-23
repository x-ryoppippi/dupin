//! Switching the keyboard input source while the panel shows, as Alfred's "Force Keyboard"
//! does: typing a query through a Japanese input method leaves it in composition, which a
//! launcher never sees until it is committed.

use std::ffi::{c_ulong, c_void};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_foundation::{NSDictionary, NSString};

/// Which input source the panel types with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Leave a source that types ASCII alone; from one that doesn't (a Japanese input
    /// method's kana mode), switch to the most recently used one that does.
    Ascii,
    /// A source by its identifier, such as `com.apple.keylayout.ABC`.
    Id(String),
}

type CFTypeRef = *const c_void;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> CFTypeRef;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> CFTypeRef;
    fn TISCopyCurrentASCIICapableKeyboardInputSource() -> CFTypeRef;
    fn TISCreateInputSourceList(properties: CFTypeRef, include_all_installed: u8) -> CFTypeRef;
    fn TISSelectInputSource(source: CFTypeRef) -> i32;
    fn TISGetInputSourceProperty(source: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    static kTISPropertyInputSourceID: CFTypeRef;
    static kTISPropertyInputSourceIsASCIICapable: CFTypeRef;
    static kTISPropertyUnicodeKeyLayoutData: CFTypeRef;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        key: u16,
        action: u16,
        modifiers: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: c_ulong,
        length: *mut c_ulong,
        characters: *mut u16,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: CFTypeRef);
    fn CFRetain(object: CFTypeRef) -> CFTypeRef;
    fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> u8;
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
    fn CFBooleanGetValue(boolean: CFTypeRef) -> u8;
    fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
}

const ANSI_V: u16 = 9;
const KEY_ACTION_DOWN: u16 = 0;
const COMMAND_MODIFIER_STATE: u32 = 1;
const NO_DEAD_KEYS_MASK: u32 = 1;

/// An input source this module holds a reference to. Main thread only, like the TIS calls.
struct Source(CFTypeRef);

impl Drop for Source {
    fn drop(&mut self) {
        // SAFETY: Every `Source` owns one reference, from a TIS Copy/Create call or CFRetain.
        unsafe { CFRelease(self.0) }
    }
}

impl Source {
    /// Takes ownership of a +1 reference; None for null.
    fn owned(ptr: CFTypeRef) -> Option<Source> {
        (!ptr.is_null()).then_some(Source(ptr))
    }

    fn same(&self, other: &Source) -> bool {
        // SAFETY: Both are live CF objects.
        unsafe { CFEqual(self.0, other.0) != 0 }
    }

    /// Whether the source types ASCII as it is (ABC, US, an input method's Roman mode).
    fn types_ascii(&self) -> bool {
        // SAFETY: A live input source and a property key constant; the returned CFBoolean is
        // owned by the source (a Get, not a Copy).
        unsafe {
            let value = TISGetInputSourceProperty(self.0, kTISPropertyInputSourceIsASCIICapable);
            !value.is_null() && CFBooleanGetValue(value) != 0
        }
    }
}

/// The input source that was active before the panel switched it, to put back on hide.
pub struct Previous(Source);

impl Previous {
    pub fn restore(self) {
        if MainThreadMarker::new().is_some() {
            // SAFETY: On the main thread, with a live input source reference.
            unsafe { TISSelectInputSource(self.0.0) };
        }
    }
}

/// Switch to `target`, returning the source to restore, or None when nothing changed (it is
/// already active, can't be found, or this isn't the main thread).
pub fn select(target: &Target) -> Option<Previous> {
    MainThreadMarker::new()?;
    // SAFETY: TIS on the main thread; every returned reference is owned by a `Source`.
    unsafe {
        let current = Source::owned(TISCopyCurrentKeyboardInputSource())?;
        let wanted = match target {
            // Switch only away from a source that can't type ASCII. Switching between two that
            // can (US to ABC) gains nothing, and keys typed while the switch lands can be lost:
            // the end-to-end test typed "calculator" right after the hotkey and got "caclator".
            Target::Ascii if current.types_ascii() => return None,
            Target::Ascii => Source::owned(TISCopyCurrentASCIICapableKeyboardInputSource())?,
            Target::Id(id) => find(id)?,
        };
        if wanted.same(&current) {
            return None;
        }
        (TISSelectInputSource(wanted.0) == 0).then_some(Previous(current))
    }
}

pub(crate) fn command_v_key() -> u16 {
    if MainThreadMarker::new().is_none() {
        return ANSI_V;
    }
    // SAFETY: TIS runs on the main thread, and the retained source owns the layout data.
    unsafe {
        let Some(source) = Source::owned(TISCopyCurrentKeyboardLayoutInputSource()) else {
            return ANSI_V;
        };
        let data = TISGetInputSourceProperty(source.0, kTISPropertyUnicodeKeyLayoutData);
        if data.is_null() {
            return ANSI_V;
        }
        let layout = CFDataGetBytePtr(data).cast::<c_void>();
        if layout.is_null() {
            return ANSI_V;
        }
        let keyboard_type = u32::from(LMGetKbdType());
        key_for_v(|key| {
            let (mut dead_key_state, mut length, mut character) = (0, 0, 0);
            // Dvorak–QWERTY ⌘ uses a separate layer when Command is held.
            let status = UCKeyTranslate(
                layout,
                key,
                KEY_ACTION_DOWN,
                COMMAND_MODIFIER_STATE,
                keyboard_type,
                NO_DEAD_KEYS_MASK,
                &mut dead_key_state,
                1,
                &mut length,
                &mut character,
            );
            (status == 0 && length == 1).then_some(character)
        })
    }
}

fn key_for_v(mut translate: impl FnMut(u16) -> Option<u16>) -> u16 {
    (0..128)
        .find(|&key| translate(key) == Some(u16::from(b'v')))
        .unwrap_or(ANSI_V)
}

/// The installed, enabled input source with this identifier.
///
/// # Safety
///
/// Main thread only.
unsafe fn find(id: &str) -> Option<Source> {
    // SAFETY: kTISPropertyInputSourceID is a CFString constant, toll-free bridged to NSString.
    let key: &NSString = unsafe { &*kTISPropertyInputSourceID.cast::<NSString>() };
    let value = NSString::from_str(id);
    let filter: Retained<NSDictionary<NSString, NSString>> = NSDictionary::from_slices(&[key], &[&*value]);
    let filter_ptr: CFTypeRef = Retained::as_ptr(&filter).cast();
    // SAFETY: The filter is a live CFDictionary (bridged); the list is owned by `Source`.
    let list = Source::owned(unsafe { TISCreateInputSourceList(filter_ptr, 0) })?;
    // SAFETY: `list` is a CFArray of input sources; the element is retained before the list
    // is released.
    unsafe {
        if CFArrayGetCount(list.0) == 0 {
            return None;
        }
        Source::owned(CFRetain(CFArrayGetValueAtIndex(list.0, 0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_uses_the_key_that_translates_to_v() {
        let dvorak = |key| match key {
            9 => Some(u16::from(b'k')),
            47 => Some(u16::from(b'v')),
            _ => None,
        };
        assert_eq!(key_for_v(dvorak), 47);
        assert_eq!(key_for_v(|_| None), 9);
    }
}
