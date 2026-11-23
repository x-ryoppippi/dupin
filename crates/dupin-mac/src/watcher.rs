use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// Watches filesystem roots and calls `changed` after their events settle.
///
/// The watched roots may not exist yet. In that case, the nearest existing parent is watched
/// and events are filtered against the requested roots.
pub struct PathWatcher {
    stream: FSEventStreamRef,
    queue: *mut std::ffi::c_void,
    context: *mut WatchContext,
    debounce: Option<std::thread::JoinHandle<()>>,
    started: bool,
}

// SAFETY: the stream delivers only on its serial queue, and Drop drains that queue.
unsafe impl Send for PathWatcher {}

impl PathWatcher {
    pub(crate) fn start(
        roots: Vec<PathBuf>,
        changed: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        Self::start_with_scope(Vec::new(), roots, false, move |_| changed())
    }

    /// Watches directory entries without forwarding descendant events. The callback receives
    /// `true` when a watched root moved and the stream must be rebuilt.
    pub fn start_shallow(
        roots: Vec<PathBuf>,
        changed: impl Fn(bool) + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        Self::start_scoped(roots, Vec::new(), changed)
    }

    /// Watches shallow directory entries and recursive trees in one stream. The callback
    /// receives `true` when a watched root moved and the stream must be rebuilt.
    pub fn start_scoped(
        shallow: Vec<PathBuf>,
        recursive: Vec<PathBuf>,
        changed: impl Fn(bool) + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        Self::start_with_scope(shallow, recursive, true, changed)
    }

    fn start_with_scope(
        shallow: Vec<PathBuf>,
        recursive: Vec<PathBuf>,
        watch_moves: bool,
        changed: impl Fn(bool) + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        let (events, received) = mpsc::channel();
        let debounce = std::thread::Builder::new()
            .name("dupin-path-watch-debounce".into())
            .spawn(move || debounce_events(received, changed))
            .map_err(|_| "could not spawn path watcher debounce")?;
        let roots = shallow
            .into_iter()
            .map(|root| (canonical_root(&root), false))
            .chain(recursive.into_iter().map(|root| (canonical_root(&root), true)))
            .collect();
        match watch_roots(roots, watch_moves, events) {
            Ok(mut watcher) => {
                watcher.debounce = Some(debounce);
                Ok(watcher)
            }
            Err(error) => {
                let _ = debounce.join();
                Err(error)
            }
        }
    }
}

unsafe extern "C" fn drained(_: *mut std::ffi::c_void) {}

impl Drop for PathWatcher {
    fn drop(&mut self) {
        // Stop delivery and drain already queued callbacks before freeing their context.
        unsafe {
            if self.started {
                FSEventStreamStop(self.stream);
            }
            FSEventStreamInvalidate(self.stream);
            dispatch_sync_f(self.queue, std::ptr::null_mut(), drained);
            FSEventStreamRelease(self.stream);
            drop(Box::from_raw(self.context));
            dispatch_release(self.queue);
        }
        if let Some(debounce) = self.debounce.take() {
            let _ = debounce.join();
        }
    }
}

fn debounce_events(events: mpsc::Receiver<bool>, changed: impl Fn(bool)) {
    const QUIET: std::time::Duration = std::time::Duration::from_millis(300);
    while let Ok(mut root_changed) = events.recv() {
        loop {
            match events.recv_timeout(QUIET) {
                Ok(root) => root_changed |= root,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    changed(root_changed);
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

type CFTypeRef = *const std::ffi::c_void;
type FSEventStreamRef = *mut std::ffi::c_void;
type FSEventId = u64;
type FSEventFlags = u32;
type CFTimeInterval = f64;
const USE_CF_TYPES: u32 = 1;
const WATCH_ROOT: u32 = 4;
const ROOT_CHANGED: u32 = 0x20;

#[repr(C)]
struct FSEventStreamContext {
    version: isize,
    info: *mut std::ffi::c_void,
    retain: Option<unsafe extern "C" fn(*const std::ffi::c_void) -> *const std::ffi::c_void>,
    release: Option<unsafe extern "C" fn(*const std::ffi::c_void)>,
    copy_description: Option<unsafe extern "C" fn(*const std::ffi::c_void) -> CFTypeRef>,
}

struct WatchContext {
    events: mpsc::Sender<bool>,
    roots: Vec<(PathBuf, bool)>,
}

type FSEventCallback = unsafe extern "C" fn(
    FSEventStreamRef,
    *mut std::ffi::c_void,
    usize,
    *mut std::ffi::c_void,
    *const FSEventFlags,
    *const FSEventId,
);

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn FSEventStreamCreate(
        allocator: CFTypeRef,
        callback: FSEventCallback,
        context: *mut FSEventStreamContext,
        paths: CFTypeRef,
        since_when: FSEventId,
        latency: CFTimeInterval,
        flags: u32,
    ) -> FSEventStreamRef;
    fn FSEventStreamSetDispatchQueue(stream: FSEventStreamRef, queue: *mut std::ffi::c_void);
    fn FSEventStreamStart(stream: FSEventStreamRef) -> u8;
    fn FSEventStreamStop(stream: FSEventStreamRef);
    fn FSEventStreamInvalidate(stream: FSEventStreamRef);
    fn FSEventStreamRelease(stream: FSEventStreamRef);
}

unsafe extern "C" {
    fn dispatch_queue_create(
        label: *const std::ffi::c_char,
        attr: *const std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn dispatch_sync_f(
        queue: *mut std::ffi::c_void,
        context: *mut std::ffi::c_void,
        work: unsafe extern "C" fn(*mut std::ffi::c_void),
    );
    fn dispatch_release(object: *mut std::ffi::c_void);
}

fn watch_roots(
    roots: Vec<(PathBuf, bool)>,
    watch_moves: bool,
    events: mpsc::Sender<bool>,
) -> Result<PathWatcher, &'static str> {
    use objc2::rc::Retained;
    use objc2_foundation::{NSArray, NSString};

    let mut watched: Vec<_> = roots
        .iter()
        .map(|(root, _)| {
            let mut path = root.as_path();
            while !path.exists() {
                let Some(parent) = path.parent() else { break };
                path = parent;
            }
            path
        })
        .collect();
    watched.sort_unstable();
    watched.dedup();
    let paths: Vec<Retained<NSString>> = watched
        .iter()
        .filter_map(|path| path.to_str().map(NSString::from_str))
        .collect();
    if paths.is_empty() {
        return Err("no paths can be watched");
    }
    let paths = NSArray::from_retained_slice(&paths);
    let context_info = Box::into_raw(Box::new(WatchContext { events, roots })).cast::<std::ffi::c_void>();
    let mut context = FSEventStreamContext {
        version: 0,
        info: context_info,
        retain: None,
        release: None,
        copy_description: None,
    };
    // FSEvents retains the roots while the stream is alive. The callback context stays allocated
    // until the stream is invalidated below.
    let stream = unsafe {
        FSEventStreamCreate(
            std::ptr::null(),
            paths_changed,
            &mut context,
            Retained::as_ptr(&paths).cast(),
            u64::MAX,
            0.5,
            USE_CF_TYPES | if watch_moves { WATCH_ROOT } else { 0 },
        )
    };
    if stream.is_null() {
        // SAFETY: context_info was allocated immediately above and is still uniquely owned.
        drop(unsafe { Box::from_raw(context_info.cast::<WatchContext>()) });
        return Err("could not create the FSEvents stream");
    }
    let queue = unsafe { dispatch_queue_create(c"dupin-path-watch".as_ptr(), std::ptr::null()) };
    if queue.is_null() {
        unsafe {
            FSEventStreamRelease(stream);
            drop(Box::from_raw(context_info.cast::<WatchContext>()));
        }
        return Err("could not create the path watcher queue");
    }
    unsafe { FSEventStreamSetDispatchQueue(stream, queue) };
    let mut watcher = PathWatcher {
        stream,
        queue,
        context: context_info.cast(),
        debounce: None,
        started: false,
    };
    if unsafe { FSEventStreamStart(stream) } == 0 {
        return Err("could not start the FSEvents stream");
    }
    watcher.started = true;
    Ok(watcher)
}

unsafe extern "C" fn paths_changed(
    _stream: FSEventStreamRef,
    info: *mut std::ffi::c_void,
    count: usize,
    paths: *mut std::ffi::c_void,
    flags: *const FSEventFlags,
    _ids: *const FSEventId,
) {
    if count == 0 {
        return;
    }
    // SAFETY: the callback context lives until the stream is stopped and released.
    let context = unsafe { &*info.cast::<WatchContext>() };
    // SAFETY: UseCFTypes supplies an array of strings for the event paths.
    let paths = unsafe { &*paths.cast::<objc2_foundation::NSArray>() };
    // SAFETY: FSEvents supplies one flag for each entry in the event paths array.
    let flags = unsafe { std::slice::from_raw_parts(flags, count) };
    let root_changed = flags.iter().any(|flag| flag & ROOT_CHANGED != 0);
    if root_changed
        || paths.iter().any(|path| {
            path.downcast_ref::<objc2_foundation::NSString>()
                .is_some_and(|path| {
                    let value = path.to_string();
                    let path = Path::new(&value);
                    context
                        .roots
                        .iter()
                        .any(|(root, recursive)| matches_event_path(path, root, *recursive))
                })
        })
    {
        let _ = context.events.send(root_changed);
    }
}

fn matches_event_path(path: &Path, root: &Path, recursive: bool) -> bool {
    path == root || root.starts_with(path) || (recursive && path.starts_with(root))
}

pub fn canonical_root(root: &Path) -> PathBuf {
    let mut existing = root;
    let mut tail = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else { break };
        tail.push(name.to_owned());
        let Some(parent) = existing.parent() else { break };
        existing = parent;
    }
    let mut path = fs::canonicalize(existing).unwrap_or_else(|_| existing.to_owned());
    for name in tail.iter().rev() {
        path.push(name);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::matches_event_path;
    use std::path::Path;

    #[test]
    fn debouncing_keeps_a_root_change_between_ordinary_events() {
        let (events, receiver) = std::sync::mpsc::channel();
        let (fired, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            super::debounce_events(receiver, move |root| fired.send(root).unwrap());
        });
        for root in [false, true, false] {
            events.send(root).unwrap();
        }
        assert!(received.recv_timeout(std::time::Duration::from_secs(2)).unwrap());
        drop(events);
        worker.join().unwrap();
    }

    #[test]
    fn shallow_scope_matches_the_root_and_its_missing_ancestors_only() {
        let root = Path::new("/home/user/.config/dupin");

        assert!(matches_event_path(root, root, false));
        assert!(matches_event_path(Path::new("/home/user/.config"), root, false));
        assert!(matches_event_path(Path::new("/home/user"), root, false));
        assert!(!matches_event_path(
            Path::new("/home/user/.config/other"),
            root,
            false
        ));
        assert!(!matches_event_path(
            Path::new("/home/user/.config/dupin/extra"),
            root,
            false
        ));
    }

    #[test]
    fn recursive_scope_also_matches_descendants() {
        let root = Path::new("/Applications");

        assert!(matches_event_path(
            Path::new("/Applications/Utilities"),
            root,
            true
        ));
        assert!(!matches_event_path(
            Path::new("/ApplicationsElsewhere"),
            root,
            true
        ));
    }
}
