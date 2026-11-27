//! Local control messages are data and never launcher actions.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};
use std::os::unix::{
    fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    net::{UnixListener, UnixStream},
};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
pub const MAX_FRAME: usize = 512 * 1024;
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub command: Command,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Open {
        query: Option<String>,
    },
    Pick {
        values: Vec<Value>,
    },
    ConfigGet {},
    SnapshotsList {},
    SnapshotsOpen {
        id: Option<String>,
    },
    /// The saved content of one snapshot, as the Snapshots window shows it.
    SnapshotsRead {
        id: String,
    },
    SnapshotsClose {},
    ConfigSet {
        base: Box<dupin_core::Config>,
        desired: Box<dupin_core::Config>,
    },
    /// Deletes the stored data of a package `dupin ext remove` removed, which the running
    /// launcher may hold open.
    ExtensionDataDelete {
        package: String,
    },
    /// Turns sync on or off, joins a vault, or reports its status.
    Sync {
        action: SyncAction,
    },
    /// What Settings saved over a read-only config.toml, as TOML, which the running launcher
    /// holds open.
    ConfigOverlay {},
    /// The installed extensions list, which the running launcher holds open.
    ExtensionLockGet {},
    /// Replaces the installed extensions list the running launcher holds open.
    ExtensionLockSet {
        lock: Box<dupin_extension_install::Lock>,
    },
    /// Forgets the grants of a package `dupin ext` removed or installed afresh, which the
    /// running launcher holds open.
    ExtensionGrantsForget {
        package: String,
    },
    /// The scopes the running launcher grants each package, for `dupin ext list`.
    ExtensionGrantsList {
        packages: Vec<String>,
    },
    /// Whether the running launcher sends the usage ping, and if not, why.
    TelemetryStatus {},
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SyncAction {
    Status {},
    /// Runs a full pass now.
    Now {},
    /// Creates a vault in the folder. Without a passphrase one is generated and returned once.
    On {
        folder: Option<String>,
        passphrase: Option<String>,
    },
    Join {
        folder: Option<String>,
        passphrase: Option<String>,
    },
    Off {},
    RemoveDevice {
        device: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSummary {
    pub id: String,
    pub saved_at: u64,
    pub mode: String,
    pub title: String,
    pub query: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotList {
    pub entries: Vec<SnapshotSummary>,
    pub truncated: bool,
    pub problem: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub id: u64,
    pub result: Result<Value, String>,
}
pub struct Call {
    pub request: Request,
    pub cancelled: Arc<AtomicBool>,
    pub reply: mpsc::SyncSender<Reply>,
}
impl Call {
    pub fn finish(self, result: Result<Value, String>) {
        let kind = match &self.request.command {
            Command::Open { .. } => "open",
            Command::Pick { .. } => "pick",
            Command::ConfigGet {} => "config get",
            Command::ConfigSet { .. } => "config set",
            Command::SnapshotsList {} => "snapshots list",
            Command::SnapshotsOpen { .. } => "snapshots open",
            Command::SnapshotsRead { .. } => "snapshots read",
            Command::SnapshotsClose {} => "snapshots close",
            Command::ExtensionDataDelete { .. } => "extension data delete",
            Command::Sync { .. } => "sync",
            Command::ConfigOverlay {} => "config overlay",
            Command::ExtensionLockGet {} => "extension lock get",
            Command::ExtensionLockSet { .. } => "extension lock set",
            Command::ExtensionGrantsForget { .. } => "extension grants forget",
            Command::ExtensionGrantsList { .. } => "extension grants list",
            Command::TelemetryStatus {} => "telemetry status",
        };
        crate::trace::control(kind, result.is_ok());
        let _ = self.reply.try_send(Reply {
            id: self.request.id,
            result,
        });
    }
}
pub fn read_frame<T: serde::de::DeserializeOwned>(input: &mut impl Read) -> io::Result<T> {
    let mut size = [0; 4];
    input.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    if size > MAX_FRAME {
        return Err(io::Error::other("frame too large"));
    }
    let mut bytes = vec![0; size];
    input.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|_| io::Error::other("invalid control JSON"))
}
pub fn write_frame(output: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_FRAME - self.0.len() {
                return Err(io::Error::other("frame too large"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = Bounded(Vec::new());
    serde_json::to_writer(&mut buffer, value)?;
    let bytes = buffer.0;
    output.write_all(&(bytes.len() as u32).to_be_bytes())?;
    output.write_all(&bytes)
}
pub fn validate(request: &Request) -> io::Result<()> {
    match &request.command {
        Command::SnapshotsOpen { id: Some(id) } | Command::SnapshotsRead { id }
            if id.is_empty() || id.len() > 128 =>
        {
            Err(io::Error::other("invalid snapshot id"))
        }
        Command::Open { query: Some(query) } if query.len() > 4096 => {
            Err(io::Error::other("query too large"))
        }
        Command::Pick { values } if values.is_empty() || values.len() > 1000 => {
            Err(io::Error::other("pick requires 1..1000 values"))
        }
        Command::ExtensionDataDelete { package } | Command::ExtensionGrantsForget { package }
            if dupin_broker::PackageId::new(package.as_str()).is_err() =>
        {
            Err(io::Error::other("invalid package ID"))
        }
        Command::ExtensionGrantsList { packages }
            if packages.len() > dupin_extension_install::MAX_PACKAGES
                || packages
                    .iter()
                    .any(|package| dupin_broker::PackageId::new(package.as_str()).is_err()) =>
        {
            Err(io::Error::other(format!(
                "grants list requires at most {} valid package IDs",
                dupin_extension_install::MAX_PACKAGES
            )))
        }
        Command::ExtensionLockSet { lock } => lock
            .validate()
            .map_err(|error| io::Error::other(error.to_string())),
        Command::Sync {
            action: SyncAction::On { folder, passphrase } | SyncAction::Join { folder, passphrase },
        } if folder
            .as_ref()
            .is_some_and(|folder| folder.is_empty() || folder.len() > 4096)
            || passphrase.as_ref().is_some_and(|text| text.len() > 1024) =>
        {
            Err(io::Error::other("invalid sync folder or passphrase"))
        }
        Command::Sync {
            action: SyncAction::RemoveDevice { device },
        } if device.len() != 32 => Err(io::Error::other("invalid device ID")),
        _ => Ok(()),
    }
}
fn owned(path: &std::path::Path, directory: bool) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no preconditions.
    if meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.file_type().is_symlink()
        || (directory && !meta.is_dir())
        || (!directory && !meta.file_type().is_socket())
    {
        return Err(io::Error::other("unsafe control path ownership or permissions"));
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn system_runtime_dir() -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    // SAFETY: a null buffer with length zero queries the required size.
    let size = unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, std::ptr::null_mut(), 0) };
    if size == 0 {
        return Err(io::Error::other("cannot resolve Darwin user temporary directory"));
    }
    let mut bytes = vec![0; size];
    // SAFETY: bytes has exactly size writable bytes; confstr retains no pointer.
    let written = unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, bytes.as_mut_ptr().cast(), size) };
    if written != size || bytes.pop() != Some(0) || bytes.is_empty() {
        return Err(io::Error::other("invalid Darwin user temporary directory"));
    }
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}
#[cfg(not(target_os = "macos"))]
fn system_runtime_dir() -> io::Result<PathBuf> {
    Ok(std::env::temp_dir())
}
pub fn socket_path(create: bool) -> io::Result<PathBuf> {
    let runtime_configured = std::env::var_os("XDG_RUNTIME_DIR").is_some();
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(system_runtime_dir)?;
    if !root.is_absolute() {
        return Err(io::Error::other("runtime directory must be absolute"));
    }
    if runtime_configured {
        owned(&root, true)?;
    }
    // SAFETY: geteuid has no preconditions.
    let dir = root.join(format!("dupin-{}", unsafe { libc::geteuid() }));
    if create {
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e),
        }
    }
    owned(&dir, true)?;
    Ok(dir.join("control.sock"))
}
fn connect() -> io::Result<UnixStream> {
    let path = socket_path(false)?;
    owned(&path, false)?;
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(300)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(stream)
}
/// A command sent to the running launcher whose reply has not been read yet.
pub struct Pending {
    stream: UnixStream,
    id: u64,
}
pub fn send(command: Command) -> Result<Pending, String> {
    let request = Request { id: 1, command };
    validate(&request).map_err(|e| e.to_string())?;
    let mut stream = connect().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => "Dupin is not running".to_owned(),
        _ => e.to_string(),
    })?;
    write_frame(&mut stream, &request).map_err(|e| e.to_string())?;
    Ok(Pending {
        stream,
        id: request.id,
    })
}
impl Pending {
    /// Closing the connection is how the host learns a caller gave up: it cancels the command,
    /// so a picker left on screen goes away with its caller.
    pub fn abort_handle(&self) -> Result<impl FnOnce() + Send + 'static, String> {
        let stream = self.stream.try_clone().map_err(|e| e.to_string())?;
        Ok(move || {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        })
    }
    pub fn wait(mut self) -> Result<Value, String> {
        let reply: Reply = read_frame(&mut self.stream).map_err(|e| e.to_string())?;
        if reply.id != self.id {
            return Err("control correlation mismatch".into());
        }
        reply.result
    }
}
pub fn exchange(command: Command) -> Result<Value, String> {
    send(command)?.wait()
}
pub struct Server {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    path: PathBuf,
    inode: u64,
    _directory_lock: DirectoryLock,
}

/// The file in the control directory whose lock says a server owns the directory.
const LOCK_FILE: &str = "control.lock";

/// A server's hold on its control directory: a POSIX record lock on its lock file, which no
/// other process can take while this one lives. An `flock` would do the same, but it belongs to
/// the open file, and a child another thread is spawning starts with a copy of this process's
/// descriptors, close-on-exec ones included, until its exec completes: closing ours then left
/// the lock held by that copy, and a launcher started meanwhile found the directory owned. A
/// record lock belongs to this process alone, so it ends when this process lets go of it.
struct DirectoryLock {
    file: Option<std::fs::File>,
    directory: PathBuf,
}

/// The control directories this process's servers hold. Record locks never turn this process
/// away from its own, and closing any descriptor for the lock file releases them, so a second
/// server here is refused before it opens the file.
fn held_here() -> &'static std::sync::Mutex<std::collections::HashSet<PathBuf>> {
    static HELD: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    HELD.get_or_init(Default::default)
}

impl DirectoryLock {
    fn take(directory: &std::path::Path) -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let owned_already = || {
            io::Error::new(
                io::ErrorKind::AddrInUse,
                "control server already owns the directory",
            )
        };
        let mut held = held_here()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.contains(directory) {
            return Err(owned_already());
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join(LOCK_FILE))?;
        let meta = file.metadata()?;
        // SAFETY: geteuid has no preconditions.
        if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(io::Error::other("unsafe control path ownership or permissions"));
        }
        let whole = libc::flock {
            l_start: 0,
            l_len: 0,
            l_pid: 0,
            l_type: libc::F_WRLCK as libc::c_short,
            l_whence: libc::SEEK_SET as libc::c_short,
        };
        // SAFETY: file owns a valid descriptor, and `whole` is the struct F_SETLK reads.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &raw const whole) } != 0 {
            let error = io::Error::last_os_error();
            return Err(match error.raw_os_error() {
                Some(libc::EAGAIN | libc::EACCES) => owned_already(),
                _ => error,
            });
        }
        held.insert(directory.to_owned());
        Ok(Self {
            file: Some(file),
            directory: directory.to_owned(),
        })
    }
}

impl Drop for DirectoryLock {
    fn drop(&mut self) {
        // Closed before another server here may open the file, whose close would end this lock.
        drop(self.file.take());
        held_here()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.directory);
    }
}
impl Server {
    pub fn start(dispatch: impl Fn(Call) + Send + Sync + 'static) -> io::Result<Self> {
        Self::bind(socket_path(true)?, dispatch)
    }
    fn bind(path: PathBuf, dispatch: impl Fn(Call) + Send + Sync + 'static) -> io::Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("missing control directory"))?;
        owned(parent, true)?;
        // A full accept queue can also return ECONNREFUSED. Hold the lock until
        // shutdown and unlink finish so another starter cannot mistake us for a stale socket.
        let directory = DirectoryLock::take(parent)?;
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                owned(&path, false)?;
                match UnixStream::connect(&path) {
                    Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                        std::fs::remove_file(&path)?;
                        UnixListener::bind(&path)?
                    }
                    Ok(_) => return Err(error),
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let inode = std::fs::symlink_metadata(&path)?.ino();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            let dispatch = Arc::new(dispatch);
            let active = Arc::new(AtomicUsize::new(0));
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if stopping.load(Ordering::Relaxed) {
                            break;
                        }
                        if active.load(Ordering::Relaxed) >= 8 {
                            let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
                            let _ = stream.set_write_timeout(Some(Duration::from_millis(50)));
                            if let Ok(request) = read_frame::<Request>(&mut stream) {
                                let _ = write_frame(
                                    &mut stream,
                                    &Reply {
                                        id: request.id,
                                        result: Err("Busy: connection limit".into()),
                                    },
                                );
                            }
                            continue;
                        }
                        active.fetch_add(1, Ordering::Relaxed);
                        let (active, dispatch) = (active.clone(), dispatch.clone());
                        std::thread::spawn(move || {
                            let _ = serve(&mut stream, |call| dispatch(call));
                            active.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            stop,
            thread: Some(thread),
            path,
            inode,
            _directory_lock: directory,
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = UnixStream::connect(&self.path);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if std::fs::symlink_metadata(&self.path).is_ok_and(|m| m.ino() == self.inode) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
struct DeadlineRead<'a> {
    stream: &'a mut UnixStream,
    deadline: Instant,
}
impl Read for DeadlineRead<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "control read deadline"));
        }
        self.stream.set_read_timeout(Some(remaining))?;
        self.stream.read(bytes)
    }
}
fn serve(stream: &mut UnixStream, dispatch: impl Fn(Call)) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let request: Request = read_frame(&mut DeadlineRead {
        stream,
        deadline: Instant::now() + Duration::from_secs(5),
    })?;
    validate(&request)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    dispatch(Call {
        request,
        cancelled: cancelled.clone(),
        reply: sender,
    });
    let deadline = Instant::now() + Duration::from_secs(300);
    let result = loop {
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(reply) => break write_frame(stream, &reply),
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err(io::Error::other("control host stopped")),
            Err(_) => (),
        }
        use std::os::fd::AsRawFd;
        let mut byte = 0u8;
        // SAFETY: byte is writable for one byte; recv does not retain the pointer.
        let n = unsafe {
            libc::recv(
                stream.as_raw_fd(),
                (&mut byte as *mut u8).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        if n == 0 || Instant::now() >= deadline {
            break Err(io::Error::other("control request cancelled"));
        }
    };
    cancelled.store(true, Ordering::Relaxed);
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn snapshot_messages_are_bounded_and_strict() {
        use super::*;
        for command in [
            Command::SnapshotsList {},
            Command::SnapshotsOpen {
                id: Some("日本語".into()),
            },
            Command::SnapshotsRead {
                id: "日本語".into()
            },
            Command::SnapshotsClose {},
        ] {
            let request = Request { id: 42, command };
            let mut bytes = Vec::new();
            write_frame(&mut bytes, &request).unwrap();
            let decoded: Request = read_frame(&mut bytes.as_slice()).unwrap();
            assert_eq!(
                serde_json::to_value(decoded).unwrap(),
                serde_json::to_value(request).unwrap()
            );
        }
        assert!(
            serde_json::from_value::<Command>(
                serde_json::json!({"kind":"snapshots_open", "id":null, "save":true})
            )
            .is_err()
        );
        for id in [String::new(), "x".repeat(129)] {
            assert!(
                validate(&Request {
                    id: 1,
                    command: Command::SnapshotsOpen { id: Some(id.clone()) }
                })
                .is_err()
            );
            assert!(
                validate(&Request {
                    id: 1,
                    command: Command::SnapshotsRead { id }
                })
                .is_err()
            );
        }
    }
    use super::*;
    #[test]
    fn extension_messages_name_valid_packages_and_stay_bounded() {
        let check = |command| validate(&Request { id: 1, command });
        assert!(
            check(Command::ExtensionGrantsForget {
                package: "notes".into()
            })
            .is_ok()
        );
        assert!(
            check(Command::ExtensionGrantsForget {
                package: String::new()
            })
            .is_err()
        );
        assert!(
            check(Command::ExtensionGrantsList {
                packages: vec!["notes".into(), "bundled:clipboard".into()]
            })
            .is_ok()
        );
        assert!(
            check(Command::ExtensionGrantsList {
                packages: vec!["x".into(); dupin_extension_install::MAX_PACKAGES + 1]
            })
            .is_err()
        );
        assert!(
            check(Command::ExtensionGrantsList {
                packages: vec!["a\nb".into()]
            })
            .is_err()
        );
        let mut lock = dupin_extension_install::Lock::new();
        assert!(
            check(Command::ExtensionLockSet {
                lock: Box::new(lock.clone())
            })
            .is_ok()
        );
        let entry = serde_json::from_value(serde_json::json!({
            "source": "/x", "content_hash": "blake3:00", "installed_at": "2026-01-01T00:00:00Z",
            "version": "1", "keyword": "x"
        }))
        .unwrap();
        lock.packages.insert("../escape".into(), entry);
        assert!(check(Command::ExtensionLockSet { lock: Box::new(lock) }).is_err());
        let newer = serde_json::from_value(serde_json::json!({ "version": 2 })).unwrap();
        assert!(
            check(Command::ExtensionLockSet {
                lock: Box::new(newer)
            })
            .is_err()
        );
    }
    /// Removes a test's control directory, with the lock file a server there left.
    fn remove_control_dir(dir: &std::path::Path) {
        match std::fs::remove_file(dir.join(LOCK_FILE)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("{error}"),
        }
        std::fs::remove_dir(dir).unwrap();
    }
    fn subprocess(mode: &str, dir: &std::path::Path, tmp: &str) {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "control::tests::disposable_process", "--nocapture"])
            .env("DUPIN_CONTROL_TEST", mode)
            .env("DUPIN_CONTROL_TEST_DIR", dir)
            .env_remove("XDG_RUNTIME_DIR")
            .env("TMPDIR", tmp)
            .spawn()
            .unwrap();
        // Waited for however long a busy machine takes to start it; one that hung would hang the
        // test for the harness to report.
        let status = child.wait().unwrap();
        assert!(status.success(), "child failed: {status}");
    }

    #[test]
    fn disposable_process() {
        let Ok(mode) = std::env::var("DUPIN_CONTROL_TEST") else {
            return;
        };
        let dir = PathBuf::from(std::env::var_os("DUPIN_CONTROL_TEST_DIR").unwrap());
        let path = dir.join("control.sock");
        match mode.as_str() {
            "exit" => {
                let _server = Server::bind(path, |_| panic!("unexpected dispatch")).unwrap();
                // SAFETY: this disposable subprocess models destructor-free Rune shutdown.
                unsafe { libc::_exit(0) }
            }
            "shutdown" => {
                let server = Server::bind(path.clone(), |_| panic!("shutdown dispatched work")).unwrap();
                std::thread::sleep(Duration::from_millis(50));
                drop(server);
                assert!(!path.exists());
            }
            "refused" => {
                let refused = Server::bind(path, |_| panic!("refused dispatched work")).err();
                assert_eq!(refused.map(|error| error.kind()), Some(io::ErrorKind::AddrInUse));
            }
            "bind" => drop(Server::bind(path, |_| panic!("bind dispatched work")).unwrap()),
            "path" => {
                use std::os::unix::ffi::OsStrExt;
                std::fs::write(
                    dir.join("resolved"),
                    system_runtime_dir().unwrap().as_os_str().as_bytes(),
                )
                .unwrap();
            }
            _ => panic!("unknown subprocess mode"),
        }
    }
    #[test]
    fn abrupt_exit_recovers_and_idle_shutdown_joins() {
        let dir = std::env::temp_dir().join(format!("dupin-control-exit-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        subprocess("exit", &dir, "/unused-first");
        let path = dir.join("control.sock");
        assert!(path.exists());
        let server = Server::bind(path.clone(), |_| {}).unwrap();
        assert!(Server::bind(path.clone(), |_| {}).is_err());
        drop(server);
        subprocess("shutdown", &dir, "/unused-second");
        remove_control_dir(&dir);
    }
    #[test]
    fn another_launcher_can_bind_while_a_child_started_before_the_server_dropped_has_not_exec_ed() {
        use std::os::unix::process::CommandExt;
        let dir = std::env::temp_dir().join(format!("dupin-control-spawning-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let path = dir.join("control.sock");
        let server = Server::bind(path.clone(), |_| {}).unwrap();
        let pipe = || {
            let mut fds = [0; 2];
            // SAFETY: fds is the two-descriptor array pipe fills.
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            (fds[0], fds[1])
        };
        let (forked_read, forked_write) = pipe();
        let (go_read, go_write) = pipe();
        // A child between fork and exec holds a copy of every descriptor this process has, as a
        // child being spawned does until its exec completes; this one stays there until told.
        let child = std::thread::spawn(move || {
            let mut command = std::process::Command::new("/usr/bin/true");
            // SAFETY: the closure makes only async-signal-safe calls on descriptors it was given.
            unsafe {
                command.pre_exec(move || {
                    libc::write(forked_write, [1u8].as_ptr().cast(), 1);
                    let mut byte = 0u8;
                    libc::read(go_read, (&raw mut byte).cast(), 1);
                    Ok(())
                });
            }
            command.status().unwrap()
        });
        /// Lets the child go on to its exec, even when an assertion below fails: a child left
        /// waiting would keep the test run's output open.
        struct Release(i32);
        impl Drop for Release {
            fn drop(&mut self) {
                // SAFETY: self.0 is the test's end of the pipe the child waits on.
                unsafe { libc::write(self.0, [1u8].as_ptr().cast(), 1) };
            }
        }
        let release = Release(go_write);
        let mut byte = 0u8;
        // SAFETY: byte is the one byte read writes.
        assert_eq!(unsafe { libc::read(forked_read, (&raw mut byte).cast(), 1) }, 1);
        drop(server);
        subprocess("bind", &dir, "/unused-spawning");
        drop(release);
        assert!(child.join().unwrap().success());
        for fd in [forked_read, forked_write, go_read, go_write] {
            // SAFETY: each is a pipe end this test opened and no longer uses.
            unsafe { libc::close(fd) };
        }
        remove_control_dir(&dir);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn darwin_runtime_path_ignores_shell_tmpdir() {
        let dir = std::env::temp_dir().join(format!("dupin-control-root-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        subprocess("path", &dir, "/unused-first");
        let first = std::fs::read(dir.join("resolved")).unwrap();
        subprocess("path", &dir, "/unused-second");
        assert_eq!(first, std::fs::read(dir.join("resolved")).unwrap());
        assert!(first.starts_with(b"/"));
        std::fs::remove_file(dir.join("resolved")).unwrap();
        remove_control_dir(&dir);
    }
    #[test]
    fn recovery_does_not_remove_regular_files_or_unsafe_sockets() {
        let dir = std::env::temp_dir().join(format!("dupin-control-refuse-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let path = dir.join("control.sock");
        std::fs::write(&path, b"keep").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(Server::bind(path.clone(), |_| {}).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        std::fs::remove_file(&path).unwrap();
        drop(UnixListener::bind(&path).unwrap());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(Server::bind(path.clone(), |_| {}).is_err());
        assert!(path.exists());
        std::fs::remove_file(path).unwrap();
        remove_control_dir(&dir);
    }
    #[test]
    fn simultaneous_stale_restarts_leave_one_live_listener() {
        let dir = std::env::temp_dir().join(format!("dupin-control-race-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        // Left by a server that exited, as a stale socket is: one this process bound would stay
        // listening in any child another test forks until that child execs.
        subprocess("exit", &dir, "/unused-race");
        let path = dir.join("control.sock");
        let barrier = Arc::new(std::sync::Barrier::new(4));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let (path, barrier) = (path.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    Server::bind(path, |call| call.finish(Ok(Value::Null)))
                })
            })
            .collect();
        let mut servers = Vec::new();
        for worker in workers {
            match worker.join().unwrap() {
                Ok(server) => servers.push(server),
                Err(error) => assert_eq!(error.kind(), io::ErrorKind::AddrInUse, "{error}"),
            }
        }
        assert_eq!(servers.len(), 1);
        // A record lock never refuses the process holding it, so another process checks it.
        subprocess("refused", &dir, "/unused-race");

        let mut client = UnixStream::connect(&path).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write_frame(
            &mut client,
            &Request {
                id: 17,
                command: Command::ConfigGet {},
            },
        )
        .unwrap();
        let reply: Reply = read_frame(&mut client).unwrap();
        assert_eq!(reply.id, 17);
        assert_eq!(reply.result.unwrap(), Value::Null);
        drop(client);
        drop(servers);
        remove_control_dir(&dir);
    }
    #[test]
    fn frames_are_strict_and_bounded() {
        for bytes in [
            b"{}".as_slice(),
            b"{",
            br#"{"id":1,"command":{"kind":"config_get","extra":true}}"#,
        ] {
            let mut framed = (bytes.len() as u32).to_be_bytes().to_vec();
            framed.extend(bytes);
            assert!(read_frame::<Request>(&mut framed.as_slice()).is_err());
        }
        assert!(read_frame::<Request>(&mut ((MAX_FRAME + 1) as u32).to_be_bytes().as_slice()).is_err());
    }
    #[test]
    fn rejects_unsafe_paths_without_replacing_them() {
        let dir = std::env::temp_dir().join(format!("dupin-control-test-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        owned(&dir, true).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(owned(&link, true).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(owned(&dir, true).is_err());
        std::fs::remove_file(link).unwrap();
        remove_control_dir(&dir);
    }
    #[test]
    fn disconnect_cancels_pending_work() {
        let (mut client, mut host) = UnixStream::pair().unwrap();
        let (sent, received) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || serve(&mut host, |call| sent.send(call).unwrap()));
        write_frame(
            &mut client,
            &Request {
                id: 7,
                command: Command::ConfigGet {},
            },
        )
        .unwrap();
        let call = received.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(client);
        assert!(worker.join().unwrap().is_err());
        assert!(call.cancelled.load(Ordering::Relaxed));
    }
    #[test]
    fn duplicate_instances_do_not_steal_and_drop_cleans_up() {
        let dir = std::env::temp_dir().join(format!("dupin-control-bind-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let path = dir.join("control.sock");
        let server = Server::bind(path.clone(), |call| call.finish(Ok(serde_json::Value::Null))).unwrap();
        owned(&path, false).unwrap();
        assert!(Server::bind(path.clone(), |_| {}).is_err());
        let mut client = UnixStream::connect(&path).unwrap();
        write_frame(
            &mut client,
            &Request {
                id: 99,
                command: Command::ConfigGet {},
            },
        )
        .unwrap();
        assert_eq!(read_frame::<Reply>(&mut client).unwrap().id, 99);
        drop(server);
        assert!(!path.exists());
        remove_control_dir(&dir);
    }
    #[test]
    fn concurrent_connections_are_bounded_and_busy_is_correlated() {
        let dir = std::env::temp_dir().join(format!("dupin-control-busy-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let path = dir.join("control.sock");
        let (sender, receiver) = mpsc::channel();
        let server = Server::bind(path.clone(), move |call| sender.send(call).unwrap()).unwrap();
        let mut clients = Vec::new();
        let mut calls = Vec::new();
        for id in 0..8 {
            let mut client = UnixStream::connect(&path).unwrap();
            write_frame(
                &mut client,
                &Request {
                    id,
                    command: Command::ConfigGet {},
                },
            )
            .unwrap();
            calls.push(receiver.recv_timeout(Duration::from_secs(2)).unwrap());
            clients.push(client);
        }
        let mut extra = UnixStream::connect(&path).unwrap();
        extra.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        write_frame(
            &mut extra,
            &Request {
                id: 100,
                command: Command::ConfigGet {},
            },
        )
        .unwrap();
        let busy: Reply = read_frame(&mut extra).unwrap();
        assert_eq!(busy.id, 100);
        assert!(busy.result.unwrap_err().starts_with("Busy"));
        for call in calls.into_iter().rev() {
            call.finish(Ok(serde_json::Value::Null));
        }
        for (id, client) in clients.iter_mut().enumerate() {
            assert_eq!(read_frame::<Reply>(client).unwrap().id, id as u64);
        }
        drop(server);
        remove_control_dir(&dir);
    }
    #[test]
    fn fake_host_correlates() {
        let (mut client, mut host) = UnixStream::pair().unwrap();
        let worker = std::thread::spawn(move || {
            serve(&mut host, |call| call.finish(Ok(serde_json::json!("日本語")))).unwrap()
        });
        write_frame(
            &mut client,
            &Request {
                id: 42,
                command: Command::ConfigGet {},
            },
        )
        .unwrap();
        let reply: Reply = read_frame(&mut client).unwrap();
        assert_eq!(reply.id, 42);
        assert_eq!(reply.result.unwrap(), "日本語");
        worker.join().unwrap();
    }
}
