//! Pressing the paste shortcut in the front app, which Dupin doesn't do on Linux yet.

const UNSUPPORTED: &str = "Pasting into the front app isn't supported on Linux yet.";

pub fn allowed() -> bool {
    false
}

pub fn request_once() -> bool {
    false
}

pub fn key_code() -> u16 {
    0
}

pub fn paste(_key: u16, _text: &str) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

pub fn press(_key: u16) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}
