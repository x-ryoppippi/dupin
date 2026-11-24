//! Watching the general pasteboard for clipboard history and extensions, putting history
//! entries back on it, and reading and writing it.
//!
//! One monitor thread polls `changeCount` while anything watches, as AppKit posts no
//! notification for pasteboard changes. Concealed copies and ignored apps must be identified
//! before reading the contents, and background reads must never raise the macOS paste prompt.

mod image;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dupin_core::clipboard::{Access, Content, FileRef, History, Policy};
use dupin_core::{Action, Item, ItemAction, View};
use dupin_host_services::clipboard::{self as service, Limits, Pasteboard, Peek, SOURCE_TYPE, Watch, Write};
use dupin_host_services::{OsAccess, ServiceError};
use objc2::rc::autoreleasepool;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::sel;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardAccessBehavior, NSPasteboardTypeFileURL, NSPasteboardTypePNG,
    NSPasteboardTypeString, NSPasteboardTypeURL, NSPasteboardWriting, NSRunningApplication, NSWorkspace,
};
use objc2_foundation::{NSArray, NSData, NSString, NSURL};
use objc2_uniform_type_identifiers::UTType;

use crate::privacy;

const INTERVAL: Duration = Duration::from_millis(500);
const FILE_URL_TYPE: &str = "public.file-url";
const STRING_TYPE: &str = "public.utf8-plain-text";
/// Image types in the order they are preferred when a copy offers several.
const IMAGE_TYPES: [&str; 5] = [
    "public.png",
    "public.tiff",
    "public.heic",
    "public.heif",
    "public.jpeg",
];
/// The largest image read off the pasteboard to convert; an uncompressed 5K TIFF fits.
const MAX_SOURCE_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SOURCE_ID_BYTES: usize = 1024;

/// What the monitor needs from a pasteboard, so tests can hand it a fixture instead of the
/// user's clipboard.
trait Board {
    fn change_count(&self) -> isize;
    fn access(&self) -> Access;
    fn types(&self) -> Vec<String>;
    /// The string of type `kind`, or `None` when it is longer than `max_bytes`, measured before
    /// it is copied out.
    fn string(&self, kind: &str, max_bytes: usize) -> Option<String>;
    /// As `string`, for data.
    fn data(&self, kind: &str, max_bytes: usize) -> Option<Vec<u8>>;
    fn file_urls(&self) -> Vec<PathBuf>;
}

impl Board for NSPasteboard {
    fn change_count(&self) -> isize {
        self.changeCount()
    }

    fn access(&self) -> Access {
        background_access(behavior(self))
    }

    fn types(&self) -> Vec<String> {
        self.types()
            .map(|types| types.iter().map(|kind| kind.to_string()).collect())
            .unwrap_or_default()
    }

    fn string(&self, kind: &str, max_bytes: usize) -> Option<String> {
        let string = self.stringForType(&NSString::from_str(kind))?;
        (string.len() <= max_bytes).then(|| string.to_string())
    }

    fn data(&self, kind: &str, max_bytes: usize) -> Option<Vec<u8>> {
        let data = self.dataForType(&NSString::from_str(kind))?;
        (data.length() <= max_bytes).then(|| data.to_vec())
    }

    fn file_urls(&self) -> Vec<PathBuf> {
        let Some(items) = self.pasteboardItems() else {
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
}

/// An app that was frontmost around a copy.
#[derive(Clone, Debug, PartialEq, Eq)]
struct App {
    id: String,
    name: Option<String>,
}

enum Tick<'a> {
    Access(Access),
    Copied {
        board: &'a NSPasteboard,
        count: isize,
        access: Access,
        /// The apps frontmost around the copy, which happened at some point since the last poll.
        apps: &'a [Option<App>],
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

fn app_ids(apps: &[Option<App>]) -> Vec<String> {
    apps.iter().flatten().map(|app| app.id.clone()).collect()
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
            let access = Board::access(&*board);
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
                        apps: app_ids(&apps),
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

fn frontmost() -> Option<App> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(App {
        id: app.bundleIdentifier()?.to_string(),
        name: app.localizedName().map(|name| name.to_string()),
    })
}

/// The name of a running app, for a copy that names its source.
fn app_named(id: String) -> App {
    let name = NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(&id))
        .iter()
        .find_map(|app| app.localizedName())
        .map(|name| name.to_string());
    App { id, name }
}

fn capture(
    board: &impl Board,
    count: isize,
    apps: &[Option<App>],
    history: &Mutex<History>,
    active: &AtomicBool,
) {
    let types = board.types();
    if service::withheld(&types) {
        return;
    }
    let Ok(guard) = history.lock() else {
        return;
    };
    let ids = apps.iter().map(|app| app.as_ref().map(|app| app.id.as_str()));
    if !active.load(Ordering::Acquire) || !ids.into_iter().all(|id| guard.accepts_source(id)) {
        return;
    }
    let policy = guard.policy().clone();
    drop(guard);

    if board.access() != Access::Allowed {
        return;
    }
    let mut source = apps.iter().rev().flatten().next().cloned();
    if types.iter().any(|kind| kind == SOURCE_TYPE) {
        let named = board.string(SOURCE_TYPE, MAX_SOURCE_ID_BYTES);
        let accepted = history
            .lock()
            .is_ok_and(|history| history.accepts_source(named.as_deref()));
        if !accepted {
            return;
        }
        source = named.map(app_named).or(source);
    }
    let Some(content) = read(board, &types, &policy) else {
        return;
    };
    // Another copy landed while this one was read; the next poll records that one instead.
    if board.change_count() != count {
        return;
    }
    // The owner drops the monitor while holding the history's lock as it clears it, so checking
    // under the lock keeps a copy read before turning the history off from landing after.
    if let Ok(mut history) = history.lock()
        && active.load(Ordering::Acquire)
    {
        let source = source.map(|app| app.name.unwrap_or(app.id));
        history.push(content, source, dupin_core::session::unix_now());
    }
}

/// The one representation of a copy the history keeps: the files, else the text, else the
/// image. Kinds the policy leaves out are never read.
fn read(board: &impl Board, types: &[String], policy: &Policy) -> Option<Content> {
    let offers = |kind: &str| types.iter().any(|offered| offered == kind);
    if offers(FILE_URL_TYPE) {
        // Finder also offers the names as text; a copy of files is never recorded as that.
        return policy
            .include_files
            .then(|| Content::Files(board.file_urls().into_iter().map(describe).collect()));
    }
    if offers(STRING_TYPE) {
        return board
            .string(STRING_TYPE, policy.max_entry_bytes)
            .map(Content::Text);
    }
    if !policy.include_images {
        return None;
    }
    let kind = IMAGE_TYPES.into_iter().find(|kind| offers(kind))?;
    let data = board.data(kind, MAX_SOURCE_IMAGE_BYTES)?;
    image::to_png(&data, image::MAX_PIXELS, policy.max_image_bytes).map(Content::Image)
}

/// A file as it was when copied: its size from the file system, its type from its name.
fn describe(path: PathBuf) -> FileRef {
    let metadata = std::fs::metadata(&path).ok();
    let kind = path
        .extension()
        .and_then(|extension| {
            UTType::typeWithFilenameExtension(&NSString::from_str(&extension.to_string_lossy()))
        })
        .filter(|kind| !kind.isDynamic())
        .map(|kind| kind.identifier().to_string())
        .or_else(|| {
            metadata
                .as_ref()
                .is_some_and(|metadata| metadata.is_dir())
                .then(|| "public.folder".to_owned())
        });
    FileRef {
        size: metadata
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len()),
        kind,
        path,
    }
}

/// Put a history entry back on the general pasteboard as it was copied: the text, the image as
/// PNG, or the files as Finder's Copy offers them.
pub fn write(content: &Content) -> Result<(), String> {
    autoreleasepool(|_| {
        let board = NSPasteboard::generalPasteboard();
        let written = match content {
            Content::Text(text) => {
                board.clearContents();
                board.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
            }
            Content::Image(image) => {
                board.clearContents();
                board.setData_forType(Some(&NSData::with_bytes(&image.png)), unsafe {
                    NSPasteboardTypePNG
                })
            }
            Content::Files(files) => return copy_files(&restorable(files, |path| path.exists())?),
        };
        written
            .then_some(())
            .ok_or_else(|| "Couldn't copy to the clipboard.".to_owned())
    })
}

/// The files of an entry still there to put back, as its row offers them.
fn restorable(files: &[FileRef], exists: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>, String> {
    let paths = FileRef::present(files, exists);
    if paths.is_empty() {
        return Err("Those files are no longer there.".to_owned());
    }
    Ok(paths)
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
                types: Board::types(&*board),
                apps: origin(count),
            }
        })
    }

    fn source(&self) -> Option<String> {
        autoreleasepool(|_| {
            Board::string(
                &*NSPasteboard::generalPasteboard(),
                SOURCE_TYPE,
                MAX_SOURCE_ID_BYTES,
            )
        })
    }

    fn app_name(&self, id: &str) -> Option<String> {
        autoreleasepool(|_| app_named(id.to_owned()).name)
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
            Write::Image(png) => autoreleasepool(|_| {
                let board = NSPasteboard::generalPasteboard();
                board.clearContents();
                board
                    .setData_forType(Some(&NSData::with_bytes(png)), unsafe { NSPasteboardTypePNG })
                    .then_some(())
                    .ok_or_else(failed)
            }),
            Write::Files(files) => copy_files(files.paths()).map_err(ServiceError::Failed),
            Write::CopiedFiles(paths) => copy_files(paths).map_err(ServiceError::Failed),
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
                        types: Board::types(board),
                        apps: Some(app_ids(apps)),
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

/// Files, then a URL, then text, then an image: the one form of a copy clipboard history keeps.
/// Finder also offers copied files' names as text, and a copy that offers both text and a
/// picture of it is the text. Images are read from PNG, TIFF, HEIC, HEIF or JPEG and converted
/// to a PNG at most [`image::MAX_PIXELS`] on its longest side; files are described, never read.
fn read_content(board: &NSPasteboard, limits: &Limits) -> Option<service::Content> {
    let types = Board::types(board);
    let offers = |kind: &str| types.iter().any(|offered| offered == kind);
    if offers(FILE_URL_TYPE) {
        let paths = board.file_urls();
        return (!paths.is_empty() && paths.len() <= limits.files).then(|| service::Content::Files {
            files: paths.into_iter().map(described).collect(),
        });
    }
    if offers(&unsafe { NSPasteboardTypeURL }.to_string()) {
        let url = board.stringForType(unsafe { NSPasteboardTypeURL })?;
        return (url.len() <= service::MAX_URL_BYTES).then(|| service::Content::Url { url: url.to_string() });
    }
    if offers(STRING_TYPE) {
        // Measured before it is copied out, so an oversized copy is never duplicated.
        return Board::string(board, STRING_TYPE, limits.text_bytes)
            .map(|text| service::Content::Text { text });
    }
    let kind = IMAGE_TYPES.into_iter().find(|kind| offers(kind))?;
    let data = Board::data(board, kind, MAX_SOURCE_IMAGE_BYTES)?;
    let image = image::to_png(&data, image::MAX_PIXELS, limits.image_bytes)?;
    Some(service::Content::Image {
        media_type: "image/png".into(),
        width: image.width,
        height: image.height,
        data: image.png.clone(),
    })
}

/// A copied file as the service describes it: its size from the file system and its type from
/// its name, so its contents are never read.
fn described(path: PathBuf) -> service::FileRef {
    let file = describe(path);
    service::FileRef {
        folder: file
            .path
            .parent()
            .map(|parent| dupin_core::item::tilde(&parent.to_string_lossy())),
        size: file.size,
        kind: file.kind,
        ..service::FileRef::new(&file.path)
    }
}

#[cfg(test)]
mod tests;
