//! The standard Open panel, for the file, folder or program an extension scope is bound to.

use std::path::PathBuf;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSModalResponseOK, NSOpenPanel};
use objc2_foundation::NSString;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
    /// An executable file.
    Program,
}

/// Asks the user to pick one item, with `message` saying what for. Returns `None` when they
/// cancel, or off the main thread, where AppKit can't show the panel.
pub fn choose(kind: Kind, message: &str) -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(kind != Kind::Folder);
    panel.setCanChooseDirectories(kind == Kind::Folder);
    panel.setAllowsMultipleSelection(false);
    panel.setMessage(Some(&NSString::from_str(message)));
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let path = panel.URL()?.path()?.to_string();
    Some(PathBuf::from(path))
}
