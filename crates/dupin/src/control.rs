//! Local control messages are data and never launcher actions.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};
use std::os::unix::{
    fs::{DirBuilderExt, MetadataExt, PermissionsExt},
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
    ConfigSet {
        base: Box<dupin_core::Config>,
        desired: Box<dupin_core::Config>,
    },
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
        Command::Open { query: Some(query) } if query.len() > 4096 => {
            Err(io::Error::other("query too large"))
        }
        Command::Pick { values } if values.is_empty() || values.len() > 1000 => {
            Err(io::Error::other("pick requires 1..1000 values"))
        }
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
    {
        return Err(io::Error::other("unsafe control path ownership or permissions"));
    }
    Ok(())
}
pub fn socket_path(create: bool) -> io::Result<PathBuf> {
    let runtime_configured = std::env::var_os("XDG_RUNTIME_DIR").is_some();
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
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
pub fn connect() -> io::Result<UnixStream> {
    let path = socket_path(false)?;
    owned(&path, false)?;
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(300)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(stream)
}
pub struct Server {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    path: PathBuf,
    inode: u64,
}
impl Server {
    pub fn start(dispatch: impl Fn(Call) + Send + Sync + 'static) -> io::Result<Self> {
        Self::bind(socket_path(true)?, dispatch)
    }
    fn bind(path: PathBuf, dispatch: impl Fn(Call) + Send + Sync + 'static) -> io::Result<Self> {
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let inode = std::fs::symlink_metadata(&path)?.ino();
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            let dispatch = Arc::new(dispatch);
            let active = Arc::new(AtomicUsize::new(0));
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
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
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            stop,
            thread: Some(thread),
            path,
            inode,
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
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
    use super::*;
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
        std::fs::remove_dir(dir).unwrap();
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
        std::fs::remove_dir(dir).unwrap();
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
        std::fs::remove_dir(dir).unwrap();
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
