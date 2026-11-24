//! Watching the general pasteboard for clipboard history and extensions, and reading and
//! writing it.
//!
//! One monitor thread polls `changeCount` while anything watches, as AppKit posts no
//! notification for pasteboard changes. Concealed copies and ignored apps must be identified
//! before reading the contents, and background reads must never raise the macOS paste prompt.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dupin_core::clipboard::{Access, Content, History};
use dupin_core::{Action, Item, ItemAction, View};
use dupin_host_services::clipboard::{
    self as service, FileRef, Limits, Pasteboard, Peek, SOURCE_TYPE, Watch, Write,
};
use dupin_host_services::{OsAccess, ServiceError};
use objc2::rc::autoreleasepool;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::sel;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardAccessBehavior, NSPasteboardTypeFileURL, NSPasteboardTypePNG,
    NSPasteboardTypeString, NSPasteboardTypeTIFF, NSPasteboardTypeURL, NSPasteboardWriting, NSWorkspace,
};
use objc2_foundation::{NSArray, NSString, NSURL};

use crate::privacy;

const INTERVAL: Duration = Duration::from_millis(500);

enum Tick<'a> {
    Access(Access),
    Copied {
        board: &'a NSPasteboard,
        count: isize,
        access: Access,
        /// The apps frontmost around the copy, which happened at some point since the last poll.
        apps: &'a [Option<String>],
    },
}

type Listener = Arc<dyn Fn(&Tick) + Send + Sync>;

struct Subscriber {
    id: u64,
    listener: Listener,
    reported: Option<Access>,
    /// The change count this subscriber last heard about, so it never hears of a copy made
    /// before it subscribed.
    seen: isize,
}

/// The apps frontmost around the copy with this change count, which only the monitor can know.
struct Origin {
    count: isize,
    apps: Vec<String>,
}

struct Hub {
    subscribers: Vec<Subscriber>,
    running: bool,
    next: u64,
    origin: Option<Origin>,
}

static HUB: Mutex<Hub> = Mutex::new(Hub {
    subscribers: Vec::new(),
    running: false,
    next: 0,
    origin: None,
});

/// Calls `listener` on the monitor thread with each access change, starting with the current
/// access, and each copy made after it subscribed, until the subscription is dropped.
fn subscribe(listener: Listener) -> std::io::Result<Subscription> {
    let seen = autoreleasepool(|_| NSPasteboard::generalPasteboard().changeCount());
    let mut hub = HUB.lock().unwrap();
    hub.next += 1;
    let id = hub.next;
    if !hub.running {
        std::thread::Builder::new()
            .name("dupin-clipboard".into())
            .spawn(poll)?;
        hub.running = true;
    }
    hub.subscribers.push(Subscriber {
        id,
        listener,
        reported: None,
        seen,
    });
    Ok(Subscription(id))
}

/// The apps frontmost around the copy with change count `count`, if the monitor saw it land.
fn origin(count: isize) -> Option<Vec<String>> {
    HUB.lock()
        .unwrap()
        .origin
        .as_ref()
        .filter(|origin| origin.count == count)
        .map(|origin| origin.apps.clone())
}

struct Subscription(u64);

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Ok(mut hub) = HUB.lock() {
            hub.subscribers.retain(|subscriber| subscriber.id != self.0);
        }
    }
}

fn poll() {
    let board = NSPasteboard::generalPasteboard();
    let mut seen = board.changeCount();
    let mut previous = frontmost();
    loop {
        std::thread::sleep(INTERVAL);
        let stop = autoreleasepool(|_| {
            let access = access(&board);
            let current = frontmost();
            let count = board.changeCount();
            let apps = if count == seen {
                [None, None]
            } else {
                [previous.take(), current.clone()]
            };
            let listeners = {
                let mut hub = HUB.lock().unwrap();
                if hub.subscribers.is_empty() {
                    hub.running = false;
                    return true;
                }
                if count != seen {
                    hub.origin = Some(Origin {
                        count,
                        apps: apps.iter().flatten().cloned().collect(),
                    });
                }
                hub.subscribers
                    .iter_mut()
                    .map(|subscriber| {
                        let report = subscriber.reported.replace(access) != Some(access);
                        let copied = std::mem::replace(&mut subscriber.seen, count) != count;
                        (subscriber.listener.clone(), report, copied)
                    })
                    .collect::<Vec<_>>()
            };
            seen = count;
            // Listeners run without the hub's lock, so one may unsubscribe while holding a
            // lock another listener takes.
            for (listener, report, _) in &listeners {
                if *report {
                    listener(&Tick::Access(access));
                }
            }
            for (listener, _, copied) in &listeners {
                if *copied {
                    listener(&Tick::Copied {
                        board: &board,
                        count,
                        access,
                        apps: &apps,
                    });
                }
            }
            previous = current;
            false
        });
        if stop {
            return;
        }
    }
}

/// Records clipboard changes into a history until dropped.
pub struct Monitor {
    active: Arc<AtomicBool>,
    _subscription: Subscription,
}

impl Monitor {
    pub fn start(history: Arc<Mutex<History>>) -> std::io::Result<Monitor> {
        let active = Arc::new(AtomicBool::new(true));
        let subscription = subscribe(Arc::new({
            let active = active.clone();
            move |tick: &Tick| match *tick {
                Tick::Access(access) => {
                    if let Ok(mut history) = history.lock()
                        && active.load(Ordering::Acquire)
                    {
                        history.set_access(access);
                    }
                }
                Tick::Copied {
                    board,
                    count,
                    access: Access::Allowed,
                    apps,
                } => capture(board, count, apps, &history, &active),
                Tick::Copied { .. } => {}
            }
        }))?;
        Ok(Monitor {
            active,
            _subscription: subscription,
        })
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

fn access(board: &NSPasteboard) -> Access {
    background_access(behavior(board))
}

fn behavior(board: &NSPasteboard) -> Option<NSPasteboardAccessBehavior> {
    board
        .respondsToSelector(sel!(accessBehavior))
        .then(|| board.accessBehavior())
}

fn background_access(policy: Option<NSPasteboardAccessBehavior>) -> Access {
    match policy {
        None | Some(NSPasteboardAccessBehavior::AlwaysAllow) => Access::Allowed,
        Some(NSPasteboardAccessBehavior::Default) => Access::Unasked,
        _ => Access::Restricted,
    }
}

/// The pasteboard policy as Settings → Permissions shows it, read without prompting.
pub(crate) fn policy_access() -> privacy::Access {
    policy_status(behavior(&NSPasteboard::generalPasteboard()))
}

fn policy_status(policy: Option<NSPasteboardAccessBehavior>) -> privacy::Access {
    match policy {
        None | Some(NSPasteboardAccessBehavior::AlwaysAllow) => privacy::Access::Allowed,
        Some(NSPasteboardAccessBehavior::Default) => privacy::Access::NotDetermined,
        Some(NSPasteboardAccessBehavior::Ask) => privacy::Access::NotAllowed,
        Some(NSPasteboardAccessBehavior::AlwaysDeny) => privacy::Access::Denied,
        Some(_) => privacy::Access::Unknown,
    }
}

pub fn access_item(access: Access) -> Option<Item> {
    let (title, subtitle, action) = match access {
        Access::Allowed => return None,
        Access::Unasked => (
            "Clipboard history needs permission to read the clipboard",
            "Enter asks macOS once; nothing is recorded until access is always allowed",
            ItemAction::new("open", "Ask for Access", Action::RequestClipboardAccess),
        ),
        Access::Restricted => (
            "Clipboard history isn't recording",
            "Always allow Dupin to paste from other apps in System Settings › Privacy & Security",
            ItemAction::new(
                "open",
                "Open Privacy & Security",
                Action::OpenUrl(privacy::Permission::Pasteboard.settings_url().into()),
            ),
        ),
    };
    Some(Item {
        payload: Default::default(),
        id: "clipboard-access".into(),
        view: View::Text {
            title: title.into(),
            subtitle: subtitle.into(),
        },
        record_usage: false,
        icon: None,
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: Some("open".into()),
        alternate: None,
        actions: vec![action],
    })
}

/// Read the clipboard once, discarding what it holds, so macOS asks whether Dupin may read it.
/// It blocks while the prompt shows.
pub fn request_access() {
    autoreleasepool(|_| {
        drop(NSPasteboard::generalPasteboard().stringForType(unsafe { NSPasteboardTypeString }));
    });
}

fn frontmost() -> Option<String> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .and_then(|app| app.bundleIdentifier())
        .map(|id| id.to_string())
}

fn types(board: &NSPasteboard) -> Vec<String> {
    board
        .types()
        .map(|types| types.iter().map(|kind| kind.to_string()).collect())
        .unwrap_or_default()
}

fn declared_source(board: &NSPasteboard) -> Option<String> {
    board
        .stringForType(&NSString::from_str(SOURCE_TYPE))
        .map(|id| id.to_string())
}

fn capture(
    board: &NSPasteboard,
    count: isize,
    apps: &[Option<String>],
    history: &Mutex<History>,
    active: &AtomicBool,
) {
    let types = types(board);
    if service::withheld(&types) {
        return;
    }
    let Ok(guard) = history.lock() else {
        return;
    };
    if !active.load(Ordering::Acquire) || !apps.iter().all(|app| guard.accepts_source(app.as_deref())) {
        return;
    }
    let max_bytes = guard.policy().max_entry_bytes;
    drop(guard);

    if access(board) != Access::Allowed {
        return;
    }
    if types.iter().any(|kind| kind == SOURCE_TYPE) {
        let source = declared_source(board);
        let accepted = history
            .lock()
            .is_ok_and(|history| history.accepts_source(source.as_deref()));
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
        && active.load(Ordering::Acquire)
    {
        history.push(content, dupin_core::session::unix_now());
    }
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
    write_urls(|| {
        paths
            .iter()
            .map(|path| NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())))
            .collect()
    })
    .ok_or_else(|| "Couldn't copy the files.".to_owned())
}

fn write_urls(urls: impl FnOnce() -> Vec<objc2::rc::Retained<NSURL>>) -> Option<()> {
    autoreleasepool(|_| {
        let objects: Vec<_> = urls()
            .into_iter()
            .map(ProtocolObject::<dyn NSPasteboardWriting>::from_retained)
            .collect();
        if objects.is_empty() {
            return None;
        }
        let board = NSPasteboard::generalPasteboard();
        board.clearContents();
        board
            .writeObjects(&NSArray::from_retained_slice(&objects))
            .then_some(())
    })
}

/// The general pasteboard, for the extension `clipboard` capability. It keeps the monitor
/// running while it exists, because only the monitor sees which app was frontmost as a copy
/// landed; a copy it didn't see has no known origin, so it is read only while no app is ignored.
pub struct GeneralPasteboard {
    _origins: Option<Subscription>,
}

impl Default for GeneralPasteboard {
    fn default() -> Self {
        Self {
            _origins: subscribe(Arc::new(|_: &Tick| {})).ok(),
        }
    }
}

impl Pasteboard for GeneralPasteboard {
    fn access(&self) -> OsAccess {
        policy_access().os_access()
    }

    fn peek(&self) -> Peek {
        autoreleasepool(|_| {
            let board = NSPasteboard::generalPasteboard();
            let count = board.changeCount();
            Peek {
                change: count as i64,
                types: types(&board),
                apps: origin(count),
            }
        })
    }

    fn source(&self) -> Option<String> {
        autoreleasepool(|_| declared_source(&NSPasteboard::generalPasteboard()))
    }

    fn read(&self, change: i64, limits: &Limits) -> Option<service::Content> {
        autoreleasepool(|_| {
            let board = NSPasteboard::generalPasteboard();
            let content = read_content(&board, limits)?;
            (board.changeCount() as i64 == change).then_some(content)
        })
    }

    fn write(&self, content: &Write) -> Result<(), ServiceError> {
        let failed = || ServiceError::Failed("Couldn't copy to the clipboard.".into());
        match content {
            Write::Text(text) => autoreleasepool(|_| {
                let board = NSPasteboard::generalPasteboard();
                board.clearContents();
                board
                    .setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
                    .then_some(())
                    .ok_or_else(failed)
            }),
            Write::Url(url) => write_urls(|| {
                NSURL::URLWithString(&NSString::from_str(url))
                    .into_iter()
                    .collect()
            })
            .ok_or_else(failed),
            Write::Files(files) => copy_files(files.paths()).map_err(ServiceError::Failed),
        }
    }

    fn watch(&self, changed: Box<dyn Fn(Peek) + Send + Sync>) -> Result<Watch, ServiceError> {
        let active = Arc::new(AtomicBool::new(true));
        let subscription = subscribe(Arc::new({
            let active = active.clone();
            move |tick: &Tick| {
                if let Tick::Copied {
                    board, count, apps, ..
                } = *tick
                    && active.load(Ordering::Acquire)
                {
                    changed(Peek {
                        change: count as i64,
                        types: types(board),
                        apps: Some(apps.iter().flatten().cloned().collect()),
                    });
                }
            }
        }))
        .map_err(|error| ServiceError::Failed(format!("Couldn't watch the clipboard: {error}")))?;
        Ok(Watch::new(Unwatch {
            active,
            _subscription: subscription,
        }))
    }
}

struct Unwatch {
    active: Arc<AtomicBool>,
    _subscription: Subscription,
}

impl Drop for Unwatch {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

/// Files, then a URL, then an image, then text: the most specific form the copy offers.
fn read_content(board: &NSPasteboard, limits: &Limits) -> Option<service::Content> {
    let types = types(board);
    let has = |kind: &NSString| types.iter().any(|offered| *offered == kind.to_string());
    if has(unsafe { NSPasteboardTypeFileURL }) {
        let paths = file_paths(board);
        return (!paths.is_empty() && paths.len() <= limits.files).then(|| service::Content::Files {
            files: paths.iter().map(|path| FileRef::new(path)).collect(),
        });
    }
    if has(unsafe { NSPasteboardTypeURL }) {
        let url = board.stringForType(unsafe { NSPasteboardTypeURL })?;
        return (url.len() <= service::MAX_URL_BYTES).then(|| service::Content::Url { url: url.to_string() });
    }
    for (kind, media_type) in [
        (unsafe { NSPasteboardTypePNG }, "image/png"),
        (unsafe { NSPasteboardTypeTIFF }, "image/tiff"),
    ] {
        if has(kind) {
            let data = board.dataForType(kind)?;
            return (data.len() <= limits.image_bytes).then(|| service::Content::Image {
                media_type: media_type.into(),
                data: data.to_vec(),
            });
        }
    }
    let text = board.stringForType(unsafe { NSPasteboardTypeString })?;
    // Measure before copying the string out, so an oversized copy is never duplicated.
    (text.len() <= limits.text_bytes).then(|| service::Content::Text {
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_rows_offer_the_matching_macos_action() {
        assert!(access_item(Access::Allowed).is_none());
        assert_eq!(
            access_item(Access::Unasked).unwrap().action("open"),
            Some(&Action::RequestClipboardAccess),
        );
        assert_eq!(
            access_item(Access::Restricted).unwrap().action("open"),
            Some(&Action::OpenUrl(
                "x-apple.systempreferences:com.apple.preference.security".into(),
            )),
        );
    }

    #[test]
    fn background_reads_require_always_allow_when_macos_reports_a_policy() {
        for (policy, expected, status, os) in [
            (None, Access::Allowed, privacy::Access::Allowed, OsAccess::Allowed),
            (
                Some(NSPasteboardAccessBehavior::AlwaysAllow),
                Access::Allowed,
                privacy::Access::Allowed,
                OsAccess::Allowed,
            ),
            (
                Some(NSPasteboardAccessBehavior::Default),
                Access::Unasked,
                privacy::Access::NotDetermined,
                OsAccess::NotDetermined,
            ),
            (
                Some(NSPasteboardAccessBehavior::Ask),
                Access::Restricted,
                privacy::Access::NotAllowed,
                OsAccess::Denied,
            ),
            (
                Some(NSPasteboardAccessBehavior::AlwaysDeny),
                Access::Restricted,
                privacy::Access::Denied,
                OsAccess::Denied,
            ),
        ] {
            assert_eq!(background_access(policy), expected);
            assert_eq!(policy_status(policy), status);
            assert_eq!(status.os_access(), os);
        }
    }
}
