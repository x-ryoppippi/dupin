//! Filesystem roots watched with inotify, as `dupin-mac` watches them with FSEvents: changes
//! are debounced, and a root that doesn't exist yet is watched through its nearest existing
//! parent.

use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::fs;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// How deep a recursive watch follows folders below a root.
const MAX_DEPTH: usize = 3;

const CHANGES: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MODIFY
    | libc::IN_CLOSE_WRITE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_ATTRIB
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

/// Watches filesystem roots and calls `changed` after their events settle.
///
/// The watched roots may not exist yet. In that case, the nearest existing parent is watched
/// and events are filtered against the requested roots.
pub struct PathWatcher {
    stop: Option<OwnedFd>,
    reader: Option<std::thread::JoinHandle<()>>,
    debounce: Option<std::thread::JoinHandle<()>>,
}

impl PathWatcher {
    pub(crate) fn start(
        roots: Vec<PathBuf>,
        changed: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        Self::start_with_scope(scoped(roots, true), move |_| changed())
    }

    /// Watches directory entries without forwarding descendant events. The callback receives
    /// `true` when a watched root moved and the watch must be rebuilt.
    pub fn start_shallow(
        roots: Vec<PathBuf>,
        changed: impl Fn(bool) + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        Self::start_with_scope(scoped(roots, false), changed)
    }

    /// Watches shallow directory entries and recursive trees with one inotify instance. The
    /// callback receives `true` when a watched root changes identity.
    pub fn start_scoped(
        shallow: Vec<PathBuf>,
        recursive: Vec<PathBuf>,
        changed: impl Fn(bool) + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        let mut roots = scoped(shallow, false);
        roots.extend(scoped(recursive, true));
        Self::start_with_scope(roots, changed)
    }

    /// Watches direct entries, including in-place edits to their files, which inotify reports
    /// on the folder that holds them.
    pub fn start_entries(
        roots: Vec<PathBuf>,
        changed: impl Fn(bool) + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        Self::start_shallow(roots, changed)
    }

    /// Watches directory entries without forwarding descendant events, and the folders that
    /// hold `files`, so an in-place write to one of them is reported. The callback receives
    /// `true` when a watched root changes identity.
    pub fn start_shallow_with_files(
        roots: Vec<PathBuf>,
        files: Vec<PathBuf>,
        changed: impl Fn(bool) + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        let mut roots = roots;
        for parent in files.iter().filter_map(|file| file.parent()) {
            if !roots.iter().any(|root| root == parent) {
                roots.push(parent.to_owned());
            }
        }
        Self::start_shallow(roots, changed)
    }

    fn start_with_scope(
        roots: Vec<(PathBuf, bool)>,
        changed: impl Fn(bool) + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        let roots: Vec<_> = roots
            .into_iter()
            .map(|(root, recursive)| (canonical_root(&root), recursive))
            .collect();
        let mut watch = Watch::new(roots)?;
        watch.sync();
        if watch.dirs.is_empty() {
            return Err("no paths can be watched");
        }
        let mut fds = [0; 2];
        // SAFETY: `fds` has room for the two descriptors pipe2 writes.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err("could not create the path watcher's stop pipe");
        }
        // SAFETY: pipe2 succeeded, so both descriptors are open and owned by nothing else.
        let (wake, stop) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let (events, received) = mpsc::channel();
        let debounce = std::thread::Builder::new()
            .name("dupin-path-watch-debounce".into())
            .spawn(move || debounce_events(received, changed))
            .map_err(|_| "could not spawn path watcher debounce")?;
        let reader = std::thread::Builder::new()
            .name("dupin-path-watch".into())
            .spawn(move || watch.run(&wake, &events))
            .map_err(|_| "could not spawn the path watcher")?;
        Ok(PathWatcher {
            stop: Some(stop),
            reader: Some(reader),
            debounce: Some(debounce),
        })
    }
}

impl Drop for PathWatcher {
    fn drop(&mut self) {
        // Closing the pipe's write end wakes the reader, which returns and drops the sender the
        // debounce thread waits on.
        drop(self.stop.take());
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
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

/// Each root with whether the folders below it are watched too.
fn scoped(roots: Vec<PathBuf>, recursive: bool) -> Vec<(PathBuf, bool)> {
    roots.into_iter().map(|root| (root, recursive)).collect()
}

struct Watch {
    inotify: OwnedFd,
    roots: Vec<(PathBuf, bool)>,
    /// Each watch descriptor's folder.
    dirs: HashMap<i32, PathBuf>,
}

impl Watch {
    fn new(roots: Vec<(PathBuf, bool)>) -> Result<Self, &'static str> {
        // SAFETY: No pointers are passed.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return Err("could not create the inotify instance");
        }
        Ok(Watch {
            // SAFETY: inotify_init1 succeeded, so `fd` is open and owned by nothing else.
            inotify: unsafe { OwnedFd::from_raw_fd(fd) },
            roots,
            dirs: HashMap::new(),
        })
    }

    /// Watch each root, or its nearest existing parent, and with a recursive scope the folders
    /// below it. Adding a folder already watched returns its descriptor again.
    fn sync(&mut self) {
        for (root, recursive) in self.roots.clone() {
            let mut path = root.as_path();
            while !path.exists() {
                let Some(parent) = path.parent() else { break };
                path = parent;
            }
            self.add(path);
            if recursive && path == root {
                self.add_below(&root, 0);
            }
        }
    }

    fn add_below(&mut self, dir: &Path, depth: usize) {
        if depth + 1 >= MAX_DEPTH {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.add(&path);
                self.add_below(&path, depth + 1);
            }
        }
    }

    fn add(&mut self, dir: &Path) {
        let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else {
            return;
        };
        // SAFETY: `c` is NUL-terminated and outlives the call.
        let wd = unsafe { libc::inotify_add_watch(self.inotify.as_raw_fd(), c.as_ptr(), CHANGES) };
        if wd >= 0 {
            self.dirs.insert(wd, dir.to_owned());
        }
    }

    fn run(mut self, wake: &OwnedFd, events: &mpsc::Sender<bool>) {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let mut fds = [
                libc::pollfd {
                    fd: self.inotify.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: wake.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // SAFETY: Both descriptors stay open for this call; poll only writes `revents`.
            if unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) } < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return;
            }
            if fds[1].revents != 0 {
                return;
            }
            let mut relevant = false;
            let mut root_changed = false;
            loop {
                // SAFETY: `buffer` is valid for its length; the descriptor is non-blocking.
                let read =
                    unsafe { libc::read(self.inotify.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
                let Ok(read) = usize::try_from(read) else { break };
                if read == 0 {
                    break;
                }
                let mut offset = 0;
                while offset + std::mem::size_of::<libc::inotify_event>() <= read {
                    // SAFETY: The kernel writes whole events; the header lies inside `read`.
                    let event = unsafe {
                        std::ptr::read_unaligned(buffer[offset..].as_ptr().cast::<libc::inotify_event>())
                    };
                    let header = std::mem::size_of::<libc::inotify_event>();
                    let name_bytes = &buffer[offset + header..offset + header + event.len as usize];
                    let name_end = name_bytes
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(name_bytes.len());
                    let name = OsStr::from_bytes(&name_bytes[..name_end]);
                    offset += header + event.len as usize;
                    if event.mask & libc::IN_Q_OVERFLOW != 0 {
                        relevant = true;
                        root_changed = true;
                        continue;
                    }
                    if event.mask & libc::IN_IGNORED != 0 {
                        self.dirs.remove(&event.wd);
                        continue;
                    }
                    let Some(dir) = self.dirs.get(&event.wd).cloned() else {
                        continue;
                    };
                    let entry = (!name.is_empty()).then(|| dir.join(name));
                    let (matched, root) = classify(&self.roots, &dir, entry.as_deref(), event.mask);
                    relevant |= matched;
                    root_changed |= root;
                }
            }
            if relevant {
                self.sync();
                if events.send(root_changed).is_err() {
                    return;
                }
            }
        }
    }
}

/// Whether an event in folder `dir` (about `entry` in it, or `dir` itself) concerns a root,
/// and whether it moved, removed or created a root or one of its parents.
fn classify(roots: &[(PathBuf, bool)], dir: &Path, entry: Option<&Path>, mask: u32) -> (bool, bool) {
    let self_event = mask & (libc::IN_DELETE_SELF | libc::IN_MOVE_SELF) != 0;
    let structural =
        mask & (libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO) != 0;
    let mut matched = false;
    let mut root_changed = false;
    for (root, recursive) in roots {
        if !matches_event_path(dir, root, *recursive) {
            continue;
        }
        matched = true;
        if self_event && root.starts_with(dir) {
            root_changed = true;
        }
        if structural && entry.is_some_and(|entry| root.starts_with(entry)) {
            root_changed = true;
        }
    }
    (matched, root_changed)
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
    use super::*;

    fn temp() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "dupin-linux-watch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        canonical_root(&dir)
    }

    #[test]
    fn debouncing_keeps_a_root_change_between_ordinary_events() {
        let (events, receiver) = mpsc::channel();
        let (fired, received) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            debounce_events(receiver, move |root| fired.send(root).unwrap());
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
    fn creating_a_missing_root_is_a_root_change() {
        let root = PathBuf::from("/home/user/.config/dupin");
        let roots = [(root.clone(), false)];
        let parent = Path::new("/home/user/.config");
        let created = (classify(&roots, parent, Some(&root), libc::IN_CREATE)).1;
        assert!(created);
        let sibling = Path::new("/home/user/.config/other");
        assert_eq!(
            classify(&roots, parent, Some(sibling), libc::IN_CREATE),
            (true, false)
        );
        let edit = Path::new("/home/user/.config/dupin/config.toml");
        assert_eq!(
            classify(&roots, &root, Some(edit), libc::IN_CLOSE_WRITE),
            (true, false)
        );
    }

    #[test]
    fn a_file_written_in_a_shallow_root_is_reported() {
        let dir = temp();
        let (fired, received) = mpsc::channel();
        let watcher =
            PathWatcher::start_shallow(vec![dir.clone()], move |root| fired.send(root).unwrap()).unwrap();
        fs::write(dir.join("config.toml"), "a = 1\n").unwrap();
        assert!(!received.recv_timeout(std::time::Duration::from_secs(5)).unwrap());
        drop(watcher);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_root_created_later_is_reported_as_a_root_change() {
        let dir = temp();
        let root = dir.join("dupin");
        let (fired, received) = mpsc::channel();
        let watcher =
            PathWatcher::start_shallow(vec![root.clone()], move |root| fired.send(root).unwrap()).unwrap();
        fs::create_dir(&root).unwrap();
        assert!(received.recv_timeout(std::time::Duration::from_secs(5)).unwrap());
        drop(watcher);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dropping_the_watcher_stops_its_threads() {
        let dir = temp();
        let watcher = PathWatcher::start_shallow(vec![dir.clone()], |_| {}).unwrap();
        drop(watcher);
        fs::remove_dir_all(dir).unwrap();
    }
}
