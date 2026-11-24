//! Dupin's side: starts the helper on first use, sends it packages to compile and calls to run,
//! and stops it when a call outlives its deadline, ignores a cancellation, outgrows its memory
//! or dies.

use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use dupin_js_runtime::{Diagnostic, RunError, Sources};
use dupin_plugin_protocol::{self as protocol, PluginItem};

use crate::wire::{self, Answer, Call, FromHelper, ToHelper};

/// How long a call may run past its deadline, or past a cancellation, before the helper is
/// stopped. The helper's own interrupt normally ends it well inside this.
const GRACE: Duration = Duration::from_millis(150);
/// How often a running call is checked for cancellation, its deadline and memory.
const TICK: Duration = Duration::from_millis(5);
const HANDSHAKE: Duration = Duration::from_secs(5);
/// Calls waiting for the helper while another runs. One query per extension can wait, since a
/// provider's newer query replaces its older one.
const MAX_WAITING: usize = 64;
const IDLE: Duration = Duration::from_secs(120);
/// The helper's footprint beyond the package a call carries: the engine's 16 MiB heap,
/// allocator overhead past it, the stack and the process itself.
const FOOTPRINT_ALLOWANCE: u64 = 64 * 1024 * 1024;

/// Makes the command that starts a helper; the host connects its standard streams.
pub type Launcher = Box<dyn Fn() -> io::Result<Command> + Send + Sync>;

/// A package's bytecode, as the extension host compiled it. Dupin keeps it as opaque bytes and
/// only ever sends it back to the host, since QuickJS doesn't validate bytecode it loads.
#[derive(Clone)]
pub struct Compiled {
    entry: String,
    modules: Arc<[(String, Box<[u8]>)]>,
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiled")
            .field("entry", &self.entry)
            .finish_non_exhaustive()
    }
}

impl Compiled {
    fn entry(&self) -> &str {
        &self.entry
    }

    fn modules(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.modules
            .iter()
            .map(|(name, bytecode)| (name.as_str(), &bytecode[..]))
    }
}

/// One helper process shared by every JavaScript extension, used by one call at a time.
pub struct Host {
    shared: Arc<Shared>,
}

struct Shared {
    launch: Launcher,
    state: Mutex<State>,
    released: Condvar,
    next_call: AtomicU64,
}

#[derive(Default)]
struct State {
    helper: Option<Helper>,
    busy: bool,
    waiting: usize,
    idle_since: Option<Instant>,
    reaping: bool,
}

/// What a call does, for the diagnostics that name it.
#[derive(Clone, Copy)]
enum Work {
    Compile,
    Search,
}

impl Work {
    fn doing(self) -> &'static str {
        match self {
            Work::Compile => "compiling the package",
            Work::Search => "search",
        }
    }

    fn whose(self) -> &'static str {
        match self {
            Work::Compile => "the compile's",
            Work::Search => "search's",
        }
    }
}

impl Host {
    pub fn new(launch: Launcher) -> Self {
        Self {
            shared: Arc::new(Shared {
                launch,
                state: Mutex::default(),
                released: Condvar::new(),
                next_call: AtomicU64::new(1),
            }),
        }
    }

    /// Compiles every module of `sources` in the helper, which reports syntax errors and imports
    /// that leave the package or name a missing file.
    pub fn compile(
        &self,
        sources: &Sources,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Compiled, RunError> {
        let (modules, tail) = wire::spans(sources.files().map(|(name, source)| (name, source.as_bytes())));
        let bytes: u64 = tail.iter().map(|part| part.len() as u64).sum();
        // The compiler's own heap, then the source, its copies and the bytecode written from it.
        let allowance = dupin_js_runtime::COMPILE_HEAP_BYTES as u64 + 4 * bytes;
        let request = Request {
            entry: sources.entry(),
            modules,
            tail,
            call: Call::Compile,
            work: Work::Compile,
            allowance,
        };
        let (answer, tail) = self.call(request, deadline, is_cancelled)?;
        let Answer::Compiled { modules } = answer else {
            return Err(mismatch());
        };
        let modules = wire::split(modules, &tail).map_err(|error| {
            RunError::Failed(host_error(format!(
                "the extension host returned a package Dupin can't read: {error}"
            )))
        })?;
        Ok(Compiled {
            entry: sources.entry().into(),
            modules: modules
                .into_iter()
                .map(|(name, bytecode)| (name, Box::from(bytecode)))
                .collect(),
        })
    }

    /// Links and evaluates `package`'s entry and checks that it exports `search`.
    pub fn check(
        &self,
        package: &Compiled,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<(), RunError> {
        match self.call(Request::run(package, Call::Check), deadline, is_cancelled)? {
            (Answer::Checked, _) => Ok(()),
            _ => Err(mismatch()),
        }
    }

    /// Calls `search(query, { limit })` in `package`.
    pub fn search(
        &self,
        package: &Compiled,
        query: &str,
        limit: usize,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<PluginItem>, RunError> {
        let limit = limit.min(dupin_js_runtime::MAX_ROWS) as u32;
        let call = Call::Search {
            query: query.into(),
            limit,
        };
        match self.call(Request::run(package, call), deadline, is_cancelled)? {
            (Answer::Rows { items }, _) => {
                validate(&items, limit)
                    .map_err(|diagnostic| RunError::Failed(diagnostic.file(package.entry())))?;
                Ok(items)
            }
            _ => Err(mismatch()),
        }
    }

    /// The running helper's process ID, for tests and measurements.
    pub fn helper_pid(&self) -> Option<u32> {
        self.shared.lock().helper.as_ref().map(|helper| helper.child.id())
    }

    fn call(
        &self,
        request: Request<'_>,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<(Answer, Vec<u8>), RunError> {
        let mut lease = self.lease(&request, deadline, is_cancelled)?;
        if is_cancelled() {
            return Err(RunError::Cancelled);
        }
        let id = self.shared.next_call.fetch_add(1, Ordering::Relaxed);
        if lease.helper.as_mut().is_some_and(|helper| !helper.is_idle()) {
            lease.helper = None;
        }
        let helper = match lease.helper.as_mut() {
            Some(helper) => helper,
            None => lease.helper.insert(Helper::start(&self.shared.launch)?),
        };
        match helper.greet(&request, deadline, is_cancelled) {
            Greeting::Ready => {}
            // A helper still starting is kept, so the next call doesn't pay for another start.
            Greeting::Waiting(error) => return Err(error),
            Greeting::Failed(error) => {
                lease.helper = None;
                return Err(error);
            }
        }
        match helper.run(id, request, deadline, is_cancelled) {
            Ok((Answer::Cancelled, _)) => Err(RunError::Cancelled),
            Ok((Answer::Failed { diagnostic }, _)) => Err(RunError::Failed(diagnostic)),
            Ok(answer) => Ok(answer),
            Err(stop) => {
                lease.helper = None;
                Err(stop)
            }
        }
    }

    /// Waits for the helper to be free, unless the caller gives up first.
    fn lease(
        &self,
        request: &Request<'_>,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Lease<'_>, RunError> {
        let mut state = self.shared.lock();
        if state.busy && state.waiting >= MAX_WAITING {
            return Err(RunError::Failed(
                Diagnostic::new("host-busy", "too many extension calls are waiting")
                    .file(request.entry)
                    .hint("Try again; other extensions are still answering."),
            ));
        }
        state.waiting += 1;
        while state.busy {
            if is_cancelled() {
                state.waiting -= 1;
                return Err(RunError::Cancelled);
            }
            if Instant::now() >= deadline {
                state.waiting -= 1;
                return Err(RunError::Failed(deadline_passed(request.work, request.entry)));
            }
            state = self
                .shared
                .released
                .wait_timeout(state, TICK)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        state.waiting -= 1;
        state.busy = true;
        Ok(Lease {
            helper: state.helper.take(),
            shared: &self.shared,
        })
    }
}

/// One call to the helper, with the package it carries.
struct Request<'a> {
    entry: &'a str,
    modules: Vec<wire::ModuleSpan>,
    tail: Vec<&'a [u8]>,
    call: Call,
    work: Work,
    /// Bytes the helper may grow past [`FOOTPRINT_ALLOWANCE`] for this call.
    allowance: u64,
}

impl<'a> Request<'a> {
    fn run(package: &'a Compiled, call: Call) -> Self {
        let (modules, tail) = wire::spans(package.modules());
        let bytes: u64 = tail.iter().map(|part| part.len() as u64).sum();
        Self {
            entry: package.entry(),
            modules,
            tail,
            call,
            work: Work::Search,
            // The bytecode as read from the socket and as the package the helper rebuilds.
            allowance: 2 * bytes,
        }
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The helper, held by one call; it goes back to the host when the call ends.
struct Lease<'a> {
    helper: Option<Helper>,
    shared: &'a Arc<Shared>,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.helper = self.helper.take();
        state.busy = false;
        state.idle_since = Some(Instant::now());
        if state.helper.is_some() && !state.reaping {
            state.reaping = spawn_reaper(Arc::downgrade(self.shared));
        }
        drop(state);
        self.shared.released.notify_one();
    }
}

/// Stops a helper nobody has used for [`IDLE`], so an unused extension costs no process.
fn spawn_reaper(shared: Weak<Shared>) -> bool {
    std::thread::Builder::new()
        .name("extension-host-reaper".into())
        .spawn(move || {
            loop {
                std::thread::sleep(IDLE / 4);
                let Some(shared) = shared.upgrade() else { return };
                let mut state = shared.lock();
                let idle = state.idle_since.is_some_and(|since| since.elapsed() >= IDLE);
                if !state.busy && idle {
                    state.helper = None;
                }
                if state.helper.is_none() && !state.busy {
                    state.reaping = false;
                    return;
                }
            }
        })
        .is_ok()
}

struct Helper {
    child: Child,
    socket: UnixStream,
    /// Bounded, so a helper that writes without being asked stalls on its own socket instead
    /// of growing Dupin's memory.
    replies: mpsc::Receiver<(FromHelper, Vec<u8>)>,
    started: Instant,
    greeted: bool,
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Helper {
    fn start(launch: &Launcher) -> Result<Self, RunError> {
        let unavailable = |error: io::Error| {
            RunError::Failed(
                Diagnostic::new(
                    "host-unavailable",
                    format!("couldn't start the extension host: {error}"),
                )
                .hint("Reinstall Dupin if this keeps happening."),
            )
        };
        let (ours, theirs) = UnixStream::pair().map_err(unavailable)?;
        let mut command = launch().map_err(unavailable)?;
        command
            .stdin(Stdio::from(OwnedFd::from(theirs)))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn().map_err(unavailable)?;
        let (replies, received) = mpsc::sync_channel(1);
        let mut reader = ours.try_clone().map_err(unavailable)?;
        ours.set_write_timeout(Some(HANDSHAKE)).map_err(unavailable)?;
        std::thread::Builder::new()
            .name("extension-host-replies".into())
            .spawn(move || {
                while let Ok(Some(message)) = wire::read::<FromHelper>(&mut reader) {
                    if replies.send(message).is_err() {
                        return;
                    }
                }
            })
            .map_err(unavailable)?;
        Ok(Self {
            child,
            socket: ours,
            replies: received,
            started: Instant::now(),
            greeted: false,
        })
    }

    /// Waits for the helper's hello, but no longer than the call may wait.
    fn greet(
        &mut self,
        request: &Request<'_>,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Greeting {
        let refused = |reason: String| {
            RunError::Failed(
                Diagnostic::new(
                    "sandbox-unavailable",
                    format!("the extension host can't run extensions safely: {reason}"),
                )
                .hint("Dupin runs extensions only inside its sandbox."),
            )
        };
        let didnt_start =
            || Greeting::Failed(RunError::Failed(host_error("the extension host didn't start")));
        while !self.greeted {
            match self.replies.recv_timeout(TICK) {
                Ok((FromHelper::Hello { version, engine }, _))
                    if version == wire::VERSION && engine == dupin_js_runtime::engine_version() =>
                {
                    self.greeted = true;
                }
                Ok((FromHelper::Hello { .. }, _)) => {
                    return Greeting::Failed(RunError::Failed(
                        host_error("the extension host is from another Dupin build")
                            .hint("Quit and reopen Dupin after an update."),
                    ));
                }
                Ok((FromHelper::Refused { reason }, _)) => return Greeting::Failed(refused(reason)),
                Ok((FromHelper::Done { .. }, _)) | Err(RecvTimeoutError::Disconnected) => {
                    return didnt_start();
                }
                Err(RecvTimeoutError::Timeout) => {
                    let now = Instant::now();
                    if now >= self.started + HANDSHAKE {
                        return didnt_start();
                    }
                    if is_cancelled() {
                        return Greeting::Waiting(RunError::Cancelled);
                    }
                    if now >= deadline {
                        return Greeting::Waiting(RunError::Failed(
                            Diagnostic::new(
                                "host-starting",
                                format!(
                                    "the extension host was still starting when {} deadline passed",
                                    request.work.whose()
                                ),
                            )
                            .file(request.entry)
                            .hint("Try again; Dupin keeps starting the extension host."),
                        ));
                    }
                }
            }
        }
        Greeting::Ready
    }

    /// Whether the helper is alive and, once greeted, has sent nothing since its last answer.
    /// It speaks only to answer a call, so anything queued between calls, such as a reply
    /// forged for a later call, means it can't be trusted with another.
    fn is_idle(&mut self) -> bool {
        let quiet = !self.greeted || matches!(self.replies.try_recv(), Err(mpsc::TryRecvError::Empty));
        quiet && matches!(self.child.try_wait(), Ok(None))
    }

    fn send(&mut self, message: &ToHelper, tail: &[&[u8]]) -> Result<(), RunError> {
        wire::write(&mut self.socket, message, tail).map_err(|error| {
            RunError::Failed(if error.kind() == io::ErrorKind::InvalidInput {
                Diagnostic::new(
                    "package-too-large",
                    "the package is larger than the extension host accepts",
                )
                .hint("Bundle and minify the extension, and leave out code it doesn't run.")
            } else {
                crashed()
            })
        })
    }

    fn run(
        &mut self,
        id: u64,
        request: Request<'_>,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<(Answer, Vec<u8>), RunError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let message = ToHelper::Run {
            id,
            deadline_ms: remaining.as_millis() as u64,
            entry: request.entry.into(),
            modules: request.modules,
            call: request.call,
        };
        self.send(&message, &request.tail)?;
        let footprint_limit = FOOTPRINT_ALLOWANCE + request.allowance;
        let mut cancel_by = None;
        loop {
            match self.replies.recv_timeout(TICK) {
                Ok((FromHelper::Done { id: answered, answer }, tail)) if answered == id => {
                    return Ok((answer, tail));
                }
                // Every call given up on stops the helper, so no other reply is ever due.
                Ok(_) => {
                    return Err(RunError::Failed(host_error(
                        "the extension host answered out of turn",
                    )));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(RunError::Failed(crashed().file(request.entry)));
                }
            }
            let now = Instant::now();
            if cancel_by.is_none() && is_cancelled() {
                let _ = self.send(&ToHelper::Cancel { id }, &[]);
                cancel_by = Some(now + GRACE);
            }
            if cancel_by.is_some_and(|by| now >= by) {
                return Err(RunError::Cancelled);
            }
            if now >= deadline + GRACE {
                return Err(RunError::Failed(deadline_passed(request.work, request.entry)));
            }
            if footprint(self.child.id()).is_some_and(|bytes| bytes > footprint_limit) {
                return Err(RunError::Failed(
                    Diagnostic::new(
                        "memory-limit",
                        format!(
                            "{} used more memory than the extension host allows",
                            request.work.doing()
                        ),
                    )
                    .file(request.entry)
                    .hint("Build smaller strings and arrays, and return only the items Dupin shows."),
                ));
            }
        }
    }
}

enum Greeting {
    Ready,
    /// The call gave up while the helper was still starting.
    Waiting(RunError),
    Failed(RunError),
}

/// The helper's physical footprint, the figure macOS itself charges a process for.
#[cfg(target_os = "macos")]
fn footprint(pid: u32) -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: `usage` is large enough for the RUSAGE_INFO_V2 flavor the call writes.
    let status =
        unsafe { libc::proc_pid_rusage(pid as i32, libc::RUSAGE_INFO_V2, usage.as_mut_ptr().cast()) };
    // SAFETY: a zero status means the call filled `usage`.
    (status == 0).then(|| unsafe { usage.assume_init() }.ri_phys_footprint)
}

#[cfg(not(target_os = "macos"))]
fn footprint(_pid: u32) -> Option<u64> {
    None
}

/// The helper is another process, so its rows are checked again here, against the protocol
/// limits a Wasm guest's answer meets.
fn validate(items: &[PluginItem], limit: u32) -> Result<(), Diagnostic> {
    let request = protocol::Request {
        version: protocol::VERSION,
        request_id: "1".into(),
        generation: "1".into(),
        command: "extension".into(),
        query: String::new(),
        limit,
        state: None,
        view: None,
        context: None,
    };
    let response = protocol::Response {
        version: protocol::VERSION,
        request_id: "1".into(),
        generation: "1".into(),
        outcome: protocol::Outcome::Results {
            items: items.to_vec(),
            state: None,
        },
    };
    response.encode(&request).map(drop).map_err(|error| {
        host_error(format!(
            "the extension host returned rows Dupin can't show: {error}"
        ))
    })
}

fn host_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new("host-error", message)
}

fn mismatch() -> RunError {
    host_error("the extension host gave an answer that doesn't fit the call").into()
}

fn crashed() -> Diagnostic {
    Diagnostic::new("host-crashed", "the extension host stopped while answering")
        .hint("Dupin starts a new one for the next query. If this keeps happening, simplify the extension.")
}

fn deadline_passed(work: Work, entry: &str) -> Diagnostic {
    let hint = match work {
        Work::Compile => {
            "Bundle the extension into less JavaScript: Dupin stops compiling a package after 2 seconds."
        }
        Work::Search => {
            "Make search return sooner: it runs as the user types, and Dupin stops it after 2 seconds."
        }
    };
    Diagnostic::new(
        "deadline",
        format!("{} didn't finish before its deadline", work.doing()),
    )
    .file(entry)
    .hint(hint)
}
