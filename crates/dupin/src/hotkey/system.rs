//! The platform reservation must be checked even while Rune owns the same chord.

use std::ptr::{self, NonNull};

use objc2::MainThreadMarker;
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use rune::Shortcut;
use winit::platform::scancode::PhysicalKeyExtScancode;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn CopySymbolicHotKeys(out: *mut *const CFArray) -> i32;
}

const COMMAND: u32 = 1 << 8;
const SHIFT: u32 = 1 << 9;
const OPTION: u32 = 1 << 11;
const CONTROL: u32 = 1 << 12;
const MODIFIERS: u32 = COMMAND | SHIFT | OPTION | CONTROL;

pub(super) fn owns(shortcut: &Shortcut) -> bool {
    let Some(_main_thread) = MainThreadMarker::new() else {
        return false;
    };
    let Some(code) = shortcut.physical_key().to_scancode() else {
        return false;
    };
    let mods = shortcut.modifiers();
    let modifiers = [
        (mods.cmd, COMMAND),
        (mods.shift, SHIFT),
        (mods.alt, OPTION),
        (mods.ctrl, CONTROL),
    ]
    .into_iter()
    .filter_map(|(enabled, flag)| enabled.then_some(flag))
    .sum::<u32>();

    let mut array = ptr::null();
    // SAFETY: HIToolbox requires the main thread and transfers ownership on success.
    let array = unsafe {
        if CopySymbolicHotKeys(&mut array) != 0 {
            return false;
        }
        let Some(array) = NonNull::new(array.cast_mut()) else {
            return false;
        };
        CFRetained::from_raw(array)
    };
    // SAFETY: CopySymbolicHotKeys returns dictionaries with CFString keys.
    let array = unsafe { array.cast_unchecked::<CFDictionary<CFString, CFType>>() };
    let [key, flags, enabled] = [
        "kHISymbolicHotKeyCode",
        "kHISymbolicHotKeyModifiers",
        "kHISymbolicHotKeyEnabled",
    ]
    .map(CFString::from_str);
    array.iter().any(|binding| {
        let number = |name: &CFString| binding.get(name)?.downcast::<CFNumber>().ok()?.as_i64();
        number(&key) == Some(i64::from(code))
            && number(&flags).is_some_and(|value| value as u32 & MODIFIERS == modifiers)
            && binding
                .get(&enabled)
                .and_then(|value| value.downcast::<CFBoolean>().ok())
                .is_some_and(|value| value.as_bool())
    })
}
