use std::ptr::{self, NonNull};

use objc2::MainThreadMarker;
use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFPreferencesCopyAppValue, CFRetained, CFString, CFType,
};
use rune::{PhysicalKeyCode, Shortcut};

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn CopySymbolicHotKeys(out: *mut *const CFArray) -> i32;
}

const COMMAND: u32 = 1 << 8;
const SHIFT: u32 = 1 << 9;
const OPTION: u32 = 1 << 11;
const CONTROL: u32 = 1 << 12;
const MODIFIERS: u32 = COMMAND | SHIFT | OPTION | CONTROL;

const SYSTEM: &[(Option<&str>, u32, u32, &str)] = &[
    (Some("64"), 49, COMMAND, "Spotlight"),
    (Some("65"), 49, COMMAND | OPTION, "Finder search"),
    (Some("60"), 49, CONTROL, "Input Sources"),
    (Some("61"), 49, CONTROL | OPTION, "Input Sources"),
    (Some("28"), 20, COMMAND | SHIFT, "Screenshot"),
    (Some("29"), 20, CONTROL | COMMAND | SHIFT, "Screenshot"),
    (Some("30"), 21, COMMAND | SHIFT, "Screenshot"),
    (Some("31"), 21, CONTROL | COMMAND | SHIFT, "Screenshot"),
    (Some("184"), 23, COMMAND | SHIFT, "Screenshot"),
    (Some("32"), 126, CONTROL, "Mission Control"),
    (Some("33"), 125, CONTROL, "Mission Control"),
    (Some("79"), 123, CONTROL, "Mission Control"),
    (Some("81"), 124, CONTROL, "Mission Control"),
    (None, 48, COMMAND, "App Switcher"),
    (None, 48, COMMAND | SHIFT, "App Switcher"),
];

type Definition = CFDictionary<CFString, CFType>;

/// The enabled macOS shortcut on the chord, independent of any Dupin registration.
pub fn owner(shortcut: &Shortcut) -> Option<&'static str> {
    if rune::tooling::is_headless() {
        return None;
    }
    let _main = MainThreadMarker::new()?;
    let mods = shortcut.modifiers();
    let chord = (
        keycode(shortcut.physical_key())?,
        carbon_modifiers([
            (mods.cmd, COMMAND),
            (mods.shift, SHIFT),
            (mods.alt, OPTION),
            (mods.ctrl, CONTROL),
        ]),
    );
    if !is_enabled(chord) {
        return None;
    }

    let preferences = CFPreferencesCopyAppValue(
        &CFString::from_str("AppleSymbolicHotKeys"),
        &CFString::from_str("com.apple.symbolichotkeys"),
    );
    let preferences = preferences.as_deref().and_then(dictionary);
    let owner = SYSTEM.iter().find(|(id, code, flags, _)| {
        let configured = id
            .and_then(|id| preferences?.get(&CFString::from_str(id)))
            .and_then(|entry| configured_chord(dictionary(&entry)?));
        configured.unwrap_or(Some((*code, *flags))) == Some(chord)
    });
    Some(owner.map_or("macOS", |(_, _, _, name)| *name))
}

fn carbon_modifiers(flags: [(bool, u32); 4]) -> u32 {
    flags
        .into_iter()
        .filter_map(|(enabled, flag)| enabled.then_some(flag))
        .sum()
}

fn dictionary(value: &CFType) -> Option<&Definition> {
    let value = value.downcast_ref::<CFDictionary>()?;
    // SAFETY: A property-list dictionary has CFString keys and CFType values.
    Some(unsafe { value.cast_unchecked::<CFString, CFType>() })
}

fn number(value: &CFType) -> Option<i64> {
    value.downcast_ref::<CFNumber>()?.as_i64()
}

fn enabled(binding: &Definition, key: &CFString) -> Option<bool> {
    Some(binding.get(key)?.downcast::<CFBoolean>().ok()?.as_bool())
}

fn configured_chord(entry: &Definition) -> Option<Option<(u32, u32)>> {
    if enabled(entry, &CFString::from_str("enabled")) == Some(false) {
        return Some(None);
    }
    let value = entry.get(&CFString::from_str("value"))?;
    let parameters = dictionary(&value)?.get(&CFString::from_str("parameters"))?;
    let parameters = parameters.downcast_ref::<CFArray>()?;
    // SAFETY: Property-list array elements are CFType values.
    let parameters = unsafe { parameters.cast_unchecked::<CFType>() };
    let parameter = |index| parameters.get(index).as_deref().and_then(number);
    let code = u32::try_from(parameter(1)?).ok()?;
    let flags = parameter(2)? as u32;
    Some((code != 65535).then(|| {
        (
            code,
            carbon_modifiers([
                (flags & (1 << 20) != 0, COMMAND),
                (flags & (1 << 17) != 0, SHIFT),
                (flags & (1 << 19) != 0, OPTION),
                (flags & (1 << 18) != 0, CONTROL),
            ]),
        )
    }))
}

fn is_enabled((code, modifiers): (u32, u32)) -> bool {
    let mut array = ptr::null();
    // SAFETY: HIToolbox requires the main thread and transfers ownership on success.
    let bindings = unsafe {
        if CopySymbolicHotKeys(&mut array) != 0 {
            return false;
        }
        let Some(array) = NonNull::new(array.cast_mut()) else {
            return false;
        };
        CFRetained::from_raw(array)
    };
    // SAFETY: CopySymbolicHotKeys returns dictionaries with CFString keys.
    let bindings = unsafe { bindings.cast_unchecked::<Definition>() };
    let [key, flags, is_enabled] = [
        "kHISymbolicHotKeyCode",
        "kHISymbolicHotKeyModifiers",
        "kHISymbolicHotKeyEnabled",
    ]
    .map(CFString::from_str);
    bindings.iter().any(|binding| {
        let matches = |key: &CFString| binding.get(key).as_deref().and_then(number);
        matches(&key) == Some(i64::from(code))
            && matches(&flags).is_some_and(|value| value as u32 & MODIFIERS == modifiers)
            && enabled(&binding, &is_enabled) == Some(true)
    })
}

fn keycode(key: PhysicalKeyCode) -> Option<u32> {
    use PhysicalKeyCode::*;

    // Rune's Carbon backend uses these virtual key codes, including for Insert.
    Some(match key {
        KeyA => 0,
        KeyS => 1,
        KeyD => 2,
        KeyF => 3,
        KeyH => 4,
        KeyG => 5,
        KeyZ => 6,
        KeyX => 7,
        KeyC => 8,
        KeyV => 9,
        KeyB => 11,
        KeyQ => 12,
        KeyW => 13,
        KeyE => 14,
        KeyR => 15,
        KeyY => 16,
        KeyT => 17,
        Digit1 => 18,
        Digit2 => 19,
        Digit3 => 20,
        Digit4 => 21,
        Digit6 => 22,
        Digit5 => 23,
        Equal => 24,
        Digit9 => 25,
        Digit7 => 26,
        Minus => 27,
        Digit8 => 28,
        Digit0 => 29,
        BracketRight => 30,
        KeyO => 31,
        KeyU => 32,
        BracketLeft => 33,
        KeyI => 34,
        KeyP => 35,
        Enter => 36,
        KeyL => 37,
        KeyJ => 38,
        Quote => 39,
        KeyK => 40,
        Semicolon => 41,
        Backslash => 42,
        Comma => 43,
        Slash => 44,
        KeyN => 45,
        KeyM => 46,
        Period => 47,
        Tab => 48,
        Space => 49,
        Backquote => 50,
        Backspace => 51,
        Escape => 53,
        F17 => 64,
        F18 => 79,
        F19 => 80,
        F20 => 90,
        F5 => 96,
        F6 => 97,
        F7 => 98,
        F3 => 99,
        F8 => 100,
        F9 => 101,
        F11 => 103,
        F13 => 105,
        F16 => 106,
        F14 => 107,
        F10 => 109,
        F12 => 111,
        F15 => 113,
        Insert => 114,
        Home => 115,
        PageUp => 116,
        Delete => 117,
        F4 => 118,
        End => 119,
        F2 => 120,
        PageDown => 121,
        F1 => 122,
        ArrowLeft => 123,
        ArrowRight => 124,
        ArrowDown => 125,
        ArrowUp => 126,
        _ => return None,
    })
}
