//! Watching the general pasteboard for clipboard history, and writing files back to it.
//!
//! The monitor polls `changeCount` on its own thread, as AppKit posts no notification for
//! pasteboard changes. It reads a change only after the history accepts its types and source,
//! so concealed copies and ignored apps are never read at all, and only while macOS lets Dupin
//! read without raising its paste prompt.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dupin_core::clipboard::{Access, Content, History, SOURCE_TYPE};
use objc2::rc::autoreleasepool;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::sel;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardAccessBehavior, NSPasteboardTypeFileURL, NSPasteboardTypeString,
    NSPasteboardWriting, NSWorkspace,
};
use objc2_foundation::{NSArray, NSString, NSURL};

const INTERVAL: Duration = Duration::from_millis(500);

/// macOS keeps reporting the default access behavior after a read where it doesn't enforce its
/// paste prompt, so a deliberate request is what lets the monitor read in that state.
static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Records clipboard changes into a history until dropped.
pub struct Monitor {
    _stop: mpsc::Sender<()>,
}

impl Monitor {
    pub fn start(history: Arc<Mutex<History>>) -> std::io::Result<Monitor> {
        let (stop, stopped) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("dupin-clipboard".into())
            .spawn(move || {
                let board = NSPasteboard::generalPasteboard();
                // Only what is copied while the history is on, and readable, is recorded.
                let mut seen = board.changeCount();
                let mut reported = None;
                let mut previous = frontmost();
                while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(INTERVAL) {
                    autoreleasepool(|_| {
                        let access = access(&board);
                        if reported != Some(access) {
                            let Ok(mut history) = history.lock() else {
                                return;
                            };
                            if !running(&stopped) {
                                return;
                            }
                            history.set_access(access);
                            reported = Some(access);
                        }
                        let current = frontmost();
                        let count = board.changeCount();
                        if count != seen {
                            seen = count;
                            // The copy happened at some point since the last poll, so both apps
                            // frontmost around it must be allowed.
                            let apps = [previous.take(), current.clone()];
                            if access == Access::Allowed {
                                capture(&board, count, &apps, &history, &stopped);
                            }
                        }
                        previous = current;
                    });
                }
            })?;
        Ok(Monitor { _stop: stop })
    }
}

fn access(board: &NSPasteboard) -> Access {
    // `accessBehavior` arrived with the paste prompt in macOS 15.4; earlier systems never prompt.
    if !board.respondsToSelector(sel!(accessBehavior)) {
        return Access::Allowed;
    }
    match board.accessBehavior() {
        NSPasteboardAccessBehavior::AlwaysAllow => Access::Allowed,
        NSPasteboardAccessBehavior::Default if REQUESTED.load(Ordering::Acquire) => Access::Allowed,
        NSPasteboardAccessBehavior::Default => Access::Unasked,
        _ => Access::Restricted,
    }
}

/// Read the clipboard once, discarding what it holds, so macOS asks whether Dupin may read it.
/// It blocks while the prompt shows.
pub fn request_access() {
    autoreleasepool(|_| {
        drop(NSPasteboard::generalPasteboard().stringForType(unsafe { NSPasteboardTypeString }));
    });
    REQUESTED.store(true, Ordering::Release);
}

fn frontmost() -> Option<String> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .and_then(|app| app.bundleIdentifier())
        .map(|id| id.to_string())
}

fn capture(
    board: &NSPasteboard,
    count: isize,
    apps: &[Option<String>],
    history: &Mutex<History>,
    stopped: &mpsc::Receiver<()>,
) {
    let types: Vec<String> = board
        .types()
        .map(|types| types.iter().map(|kind| kind.to_string()).collect())
        .unwrap_or_default();
    let Ok(guard) = history.lock() else {
        return;
    };
    if !running(stopped) || !apps.iter().all(|app| guard.accepts(&types, app.as_deref())) {
        return;
    }
    let max_bytes = guard.policy().max_entry_bytes;
    drop(guard);

    if types.iter().any(|kind| kind == SOURCE_TYPE) {
        let source = board
            .stringForType(&NSString::from_str(SOURCE_TYPE))
            .map(|id| id.to_string());
        let accepted = history
            .lock()
            .is_ok_and(|history| history.accepts(&types, source.as_deref()));
        if !accepted {
            return;
        }
    }
    let file_type = unsafe { NSPasteboardTypeFileURL }.to_string();
    let content = if types.contains(&file_type) {
        Content::Files(file_paths(board))
    } else {
        let Some(text) = board.stringForType(unsafe { NSPasteboardTypeString }) else {
            return;
        };
        // Measure before copying the string out, so an oversized copy is never duplicated.
        if text.len() > max_bytes {
            return;
        }
        Content::Text(text.to_string())
    };
    // Another copy landed while this one was read; the next poll records that one instead.
    if board.changeCount() != count {
        return;
    }
    // The owner drops the monitor while holding the history's lock as it clears it, so checking
    // under the lock keeps a copy read before turning the history off from landing after.
    if let Ok(mut history) = history.lock()
        && running(stopped)
    {
        history.push(content, dupin_core::session::unix_now());
    }
}

fn running(stopped: &mpsc::Receiver<()>) -> bool {
    matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Empty))
}

fn file_paths(board: &NSPasteboard) -> Vec<PathBuf> {
    let Some(items) = board.pasteboardItems() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let url = item.stringForType(unsafe { NSPasteboardTypeFileURL })?;
            // Finder copies file reference URLs (`file:///.file/id=…`); resolve them to paths.
            let url = NSURL::URLWithString(&url)?.filePathURL()?;
            Some(PathBuf::from(url.path()?.to_string()))
        })
        .collect()
}

/// Put files on the general pasteboard, as Finder's Copy does.
pub fn copy_files(paths: &[PathBuf]) -> Result<(), String> {
    autoreleasepool(|_| {
        let urls: Vec<_> = paths
            .iter()
            .map(|path| {
                ProtocolObject::<dyn NSPasteboardWriting>::from_retained(NSURL::fileURLWithPath(
                    &NSString::from_str(&path.to_string_lossy()),
                ))
            })
            .collect();
        let board = NSPasteboard::generalPasteboard();
        board.clearContents();
        board
            .writeObjects(&NSArray::from_retained_slice(&urls))
            .then_some(())
            .ok_or_else(|| "Couldn't copy the files.".to_owned())
    })
}
