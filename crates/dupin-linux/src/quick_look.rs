//! Quick Look is macOS's. On Linux no result carries the preview action, and no previewer
//! exists.

use std::path::Path;

pub const ACTION_ID: &str = "quick-look";

/// A preview panel. None exists on Linux.
pub enum Previewer {}

impl Previewer {
    pub fn show(&self, _: &Path) -> bool {
        match *self {}
    }

    pub fn panel_is_key(&self) -> bool {
        match *self {}
    }

    pub fn trace_picker_input(&self) {
        match *self {}
    }

    pub fn owner_is_key(&self) -> bool {
        match *self {}
    }

    pub fn finish_focus_transfer(&self) -> bool {
        match *self {}
    }

    pub fn close(&self) {
        match *self {}
    }
}
