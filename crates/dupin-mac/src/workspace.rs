//! Running actions through NSWorkspace.

use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum Action {
    Open(PathBuf),
    Reveal(PathBuf),
    OpenUrl(String),
}
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSArray, NSString, NSURL};

/// Carry out `action`. Returns false when the system refused (a vanished file, say).
pub fn run(action: &Action) -> bool {
    match action {
        Action::Open(path) => open(path),
        Action::OpenUrl(url) => open_url(url),
        Action::Reveal(path) => {
            reveal(path);
            true
        }
    }
}

/// Open a URL with its registered handler.
fn open_url(value: &str) -> bool {
    let Some(url) = NSURL::URLWithString(&NSString::from_str(value)) else {
        return false;
    };
    NSWorkspace::sharedWorkspace().openURL(&url)
}

/// Open with the default handler: launches an app bundle, opens a document or folder.
pub fn open(path: &Path) -> bool {
    NSWorkspace::sharedWorkspace().openURL(&file_url(path))
}

/// Show in Finder with the item selected.
fn reveal(path: &Path) {
    NSWorkspace::sharedWorkspace()
        .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[file_url(path)]));
}

fn file_url(path: &Path) -> objc2::rc::Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}
