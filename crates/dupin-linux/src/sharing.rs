//! Sharing files with other apps, which Dupin has no Linux targets for yet.

use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::NonNull;

use dupin_core::ShareTarget;

pub fn targets() -> [(&'static str, &'static str, ShareTarget); 0] {
    []
}

#[derive(Default)]
pub struct Picker(());

impl Picker {
    pub fn is_active(&self) -> bool {
        false
    }

    pub fn close(&mut self) {}

    /// # Safety
    /// Nothing is dereferenced; the signature matches macOS's.
    pub unsafe fn share(
        &mut self,
        _target: ShareTarget,
        _paths: &[PathBuf],
        _view: NonNull<c_void>,
        _on_picker_finished: impl FnOnce() + 'static,
    ) -> Result<(), String> {
        Err("Sharing isn't supported on Linux yet.".into())
    }
}
