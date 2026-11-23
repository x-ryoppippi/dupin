//! Dupin's side: starts the helper on first use, sends it packages and calls, and stops it
//! when a call outlives its deadline, ignores a cancellation, outgrows its memory or dies.

use std::collections::VecDeque;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use dupin_js_runtime::{Diagnostic, Package, RunError};
use dupin_plugin_protocol::{self as protocol, PluginItem};

use crate::wire::{self, Answer, Call, FromHelper, ModuleSpan, ToHelper};

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
/// Packages, and bytes of their bytecode, kept in the helper between calls.
const MAX_PACKAGES: usize = 64;
const MAX_PACKAGE_BYTES: usize = 32 * 1024 * 1024;
/// The helper's footprint beyond the bytecode it holds: the engine's 16 MiB heap, allocator
/// overhead past it, the stack and the process itself.
const FOOTPRINT_ALLOWANCE: u64 = 64 * 1024 * 1024;

/// Makes the command that starts a helper; the host connects its standard streams.
pub type Launcher = Box<dyn Fn() -> io::Result<Command> + Send + Sync>;

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

    /// Links and evaluates `package`'s entry and checks that it exports `search`.
    pub fn check(
        &self,
        package: &Package,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<(), RunError> {
        match self.call(package, Call::Check, deadline, is_cancelled)? {
            Answer::Checked => Ok(()),
            _ => Err(host_error("the extension host gave an answer that doesn't fit the call").into()),
        }
    }

    /// Calls `search(query, { limit })` in `package`.
    pub fn search(
        &self,
        package: &Package,
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
        match self.call(package, call, deadline, is_cancelled)? {
            Answer::Rows { items } => {
                validate(&items, limit)
                    .map_err(|diagnostic| RunError::Failed(diagnostic.file(package.entry())))?;
                Ok(items)
            }
            _ => Err(host_error("the extension host gave an answer that doesn't fit the call").into()),
        }
    }

    /// The running helper's process ID, for tests and measurements.
    pub fn helper_pid(&self) -> Option<u32> {
        self.shared.lock().helper.as_ref().map(|helper| helper.child.id())
    }

    fn call(
        &self,
        package: &Package,
        call: Call,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Answer, RunError> {
        let mut lease = self.lease(package, deadline, is_cancelled)?;
        if is_cancelled() {
            return Err(RunError::Cancelled);
        }
        let id = self.shared.next_call.fetch_add(1, Ordering::Relaxed);
        if lease
            .helper
            .as_mut()
            .is_some_and(|helper| !matches!(helper.child.try_wait(), Ok(None)))
        {
            lease.helper = None;
        }
        let helper = match lease.helper.as_mut() {
            Some(helper) => helper,
            None => lease.helper.insert(Helper::start(&self.shared.launch)?),
        };
        let outcome = helper
            .load(package)
            .and_then(|()| helper.run(id, package, call, deadline, is_cancelled));
        match outcome {
            Ok(Answer::Cancelled) => Err(RunError::Cancelled),
            Ok(Answer::Failed { diagnostic }) => Err(RunError::Failed(diagnostic)),
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
        package: &Package,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Lease<'_>, RunError> {
        let mut state = self.shared.lock();
        if state.busy && state.waiting >= MAX_WAITING {
            return Err(RunError::Failed(
                Diagnostic::new("host-busy", "too many extension calls are waiting")
                    .file(package.entry())
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
                return Err(RunError::Failed(deadline_passed(package)));
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
    replies: mpsc::Receiver<FromHelper>,
    /// Packages the helper holds, least recently used first, with their bytecode size.
    loaded: VecDeque<(u64, usize)>,
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
        let (replies, received) = mpsc::channel();
        let mut reader = ours.try_clone().map_err(unavailable)?;
        ours.set_write_timeout(Some(HANDSHAKE)).map_err(unavailable)?;
        std::thread::Builder::new()
            .name("extension-host-replies".into())
            .spawn(move || {
                while let Ok(Some((message, _))) = wire::read::<FromHelper>(&mut reader) {
                    if replies.send(message).is_err() {
                        return;
                    }
                }
            })
            .map_err(unavailable)?;
        let helper = Self {
            child,
            socket: ours,
            replies: received,
            loaded: VecDeque::new(),
        };
        let refused = |reason: String| {
            RunError::Failed(
                Diagnostic::new(
                    "sandbox-unavailable",
                    format!("the extension host can't run extensions safely: {reason}"),
                )
                .hint("Dupin runs extensions only inside its sandbox."),
            )
        };
        match helper.replies.recv_timeout(HANDSHAKE) {
            Ok(FromHelper::Hello { version, engine })
                if version == wire::VERSION && engine == dupin_js_runtime::engine_version() =>
            {
                Ok(helper)
            }
            Ok(FromHelper::Hello { .. }) => Err(RunError::Failed(
                host_error("the extension host is from another Dupin build")
                    .hint("Quit and reopen Dupin after an update."),
            )),
            Ok(FromHelper::Refused { reason }) => Err(refused(reason)),
            Ok(FromHelper::Done { .. }) | Err(_) => {
                Err(RunError::Failed(host_error("the extension host didn't start")))
            }
        }
    }

    fn send(&mut self, message: &ToHelper, tail: &[&[u8]]) -> Result<(), RunError> {
        wire::write(&mut self.socket, message, tail).map_err(|error| {
            RunError::Failed(if error.kind() == io::ErrorKind::InvalidInput {
                Diagnostic::new(
                    "package-too-large",
                    "the compiled package is larger than the extension host accepts",
                )
                .hint("Bundle and minify the extension, and leave out code it doesn't run.")
            } else {
                crashed()
            })
        })
    }

    /// Sends `package` unless the helper holds it, making room by dropping the least recently
    /// used packages.
    fn load(&mut self, package: &Package) -> Result<(), RunError> {
        if let Some(position) = self.loaded.iter().position(|(id, _)| *id == package.id()) {
            let entry = self.loaded.remove(position).expect("present");
            self.loaded.push_back(entry);
            return Ok(());
        }
        let mut modules = Vec::new();
        let mut tail = Vec::new();
        for (name, bytecode) in package.modules() {
            modules.push(ModuleSpan {
                name: name.into(),
                len: bytecode.len() as u32,
            });
            tail.push(bytecode);
        }
        let bytes: usize = tail.iter().map(|part| part.len()).sum();
        while let Some(&(oldest, _)) = self.loaded.front() {
            let held: usize = self.loaded.iter().map(|(_, size)| size).sum();
            if self.loaded.len() < MAX_PACKAGES && held + bytes <= MAX_PACKAGE_BYTES {
                break;
            }
            self.send(&ToHelper::Unload { package: oldest }, &[])?;
            self.loaded.pop_front();
        }
        let message = ToHelper::Load {
            package: package.id(),
            entry: package.entry().into(),
            modules,
        };
        self.send(&message, &tail)?;
        self.loaded.push_back((package.id(), bytes));
        Ok(())
    }

    fn run(
        &mut self,
        id: u64,
        package: &Package,
        call: Call,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Answer, RunError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let message = ToHelper::Run {
            id,
            package: package.id(),
            deadline_ms: remaining.as_millis() as u64,
            call,
        };
        self.send(&message, &[])?;
        let footprint_limit =
            FOOTPRINT_ALLOWANCE + self.loaded.iter().map(|(_, size)| *size as u64).sum::<u64>();
        let mut cancel_by = None;
        loop {
            match self.replies.recv_timeout(TICK) {
                Ok(FromHelper::Done { id: answered, answer }) if answered == id => return Ok(answer),
                // A reply to a call that was already given up on.
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(RunError::Failed(crashed().file(package.entry())));
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
                return Err(RunError::Failed(deadline_passed(package)));
            }
            if footprint(self.child.id()).is_some_and(|bytes| bytes > footprint_limit) {
                return Err(RunError::Failed(
                    Diagnostic::new(
                        "memory-limit",
                        "search used more memory than the extension host allows",
                    )
                    .file(package.entry())
                    .hint("Build smaller strings and arrays, and return only the items Dupin shows."),
                ));
            }
        }
    }
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

fn crashed() -> Diagnostic {
    Diagnostic::new("host-crashed", "the extension host stopped while answering")
        .hint("Dupin starts a new one for the next query. If this keeps happening, simplify the extension.")
}

fn deadline_passed(package: &Package) -> Diagnostic {
    Diagnostic::new("deadline", "search didn't finish before its deadline")
        .file(package.entry())
        .hint("Make search return sooner: it runs as the user types, and Dupin stops it after 2 seconds.")
}
