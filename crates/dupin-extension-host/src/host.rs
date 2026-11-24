//! Dupin's side: starts one shared helper, on first use, for searches, and a helper of its own
//! for each compile, action and event, and stops a helper when a call outlives its deadline,
//! ignores a cancellation, outgrows its memory or dies.

use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use std::collections::HashSet;

use dupin_js_runtime::row::Source;
use dupin_js_runtime::{
    Acted, CataloguePage, Diagnostic, Environment, Exports, Handled, Reason, Row, RunError, Sources,
};
use dupin_plugin_protocol::MAX_RESPONSE_BYTES;

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

/// Dupin's host services as one call sees them. The caller builds one per call, for the package
/// and the reason it runs (typing, a chosen action, a watched event), so the helper can't claim
/// either.
pub trait Services: Send + Sync {
    /// Answers request `op` with `args`, both as the guest sent them, with JSON
    /// `{"ok": value}` or `{"error": {"code", "message", …}}`. Runs on a thread of its own.
    /// `cancelled` is set once the call has ended, after which the reply is dropped.
    fn call(&self, op: &str, args: &[u8], deadline: Instant, cancelled: &AtomicBool) -> Vec<u8>;
}

/// No host services: every request fails as unavailable.
pub struct NoServices;

impl Services for NoServices {
    fn call(&self, _op: &str, _args: &[u8], _deadline: Instant, _cancelled: &AtomicBool) -> Vec<u8> {
        br#"{"error":{"code":"unavailable","message":"host services are unavailable here"}}"#.to_vec()
    }
}

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
    /// A package with no bytecode, for tests that route or list extensions without running one.
    #[cfg(feature = "testing")]
    pub fn empty() -> Self {
        Self {
            entry: dupin_js_runtime::DEFAULT_ENTRY.into(),
            modules: Arc::from([]),
        }
    }

    /// The module the package's calls start from, as diagnostics name it.
    pub fn entry(&self) -> &str {
        &self.entry
    }

    fn modules(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.modules
            .iter()
            .map(|(name, bytecode)| (name.as_str(), &bytecode[..]))
    }
}

/// One helper process shared by every JavaScript extension's searches, used by one call at a
/// time, and a helper of its own for each compile, action and event.
pub struct Host {
    shared: Arc<Shared>,
}

struct Shared {
    launch: Launcher,
    environment: Option<Environment>,
    state: Mutex<State>,
    released: Condvar,
    next_call: AtomicU64,
    workers: Workers,
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
    Items,
    Action,
    Event,
}

impl Work {
    fn doing(self) -> &'static str {
        match self {
            Work::Compile => "compiling the package",
            Work::Search => "search",
            Work::Items => "items",
            Work::Action => "the action",
            Work::Event => "the event handler",
        }
    }

    fn whose(self) -> &'static str {
        match self {
            Work::Compile => "the compile's",
            Work::Search => "search's",
            Work::Items => "items'",
            Work::Action => "the action's",
            Work::Event => "the event handler's",
        }
    }
}

impl Host {
    pub fn new(launch: Launcher) -> Self {
        Self::fixed(launch, None)
    }

    /// A host whose guests see the clock and `Math.random` `environment` fixes, as a fixture
    /// replays them; with none, the real ones.
    pub fn fixed(launch: Launcher, environment: Option<Environment>) -> Self {
        Self {
            shared: Arc::new(Shared {
                launch,
                environment,
                state: Mutex::default(),
                released: Condvar::new(),
                next_call: AtomicU64::new(1),
                workers: Workers::default(),
            }),
        }
    }

    /// Compiles every module of `sources` in a helper started for this compile alone, which
    /// reports syntax errors and imports that leave the package or name a missing file.
    /// `limit` is how long compiling may take once that helper is ready.
    ///
    /// Every later helper loads the bytecode unvalidated, so no other package's code may have
    /// reached the helper that writes it. Not waiting for the shared helper also keeps a busy
    /// extension from failing another's load.
    pub fn compile(
        &self,
        sources: &Sources,
        limit: Duration,
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
        let (answer, tail) = self.alone(
            request,
            |helper| helper.started + HANDSHAKE,
            || Instant::now() + limit,
            is_cancelled,
            &no_services(),
        )?;
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

    /// Links and evaluates `package`'s entry and reports which of `search` and `items` it
    /// exports.
    pub fn check(
        &self,
        package: &Compiled,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Exports, RunError> {
        let request = Request::run(package, Call::Check, Work::Search);
        match self.call(request, deadline, is_cancelled, &no_services())? {
            (Answer::Checked { exports }, _) => Ok(exports),
            _ => Err(mismatch()),
        }
    }

    /// Calls `items({ cursor, reason })` in `package` for the next part of its catalogue,
    /// answering its host requests with `services`.
    pub fn items(
        &self,
        package: &Compiled,
        cursor: Option<&str>,
        reason: Reason,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<CataloguePage, RunError> {
        let call = Call::Items {
            cursor: cursor.map(str::to_owned),
            reason,
        };
        let request = Request::run(package, call, Work::Items);
        match self.call(request, deadline, is_cancelled, services)? {
            (Answer::Catalogue { page }, _) => {
                page.validate().map_err(|problem| unshowable(problem, package))?;
                Ok(page)
            }
            _ => Err(mismatch()),
        }
    }

    /// Calls `search(query, { limit })` in `package`, answering its host requests with
    /// `services`.
    pub fn search(
        &self,
        package: &Compiled,
        query: &str,
        limit: usize,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<Vec<Row>, RunError> {
        let limit = limit.min(dupin_js_runtime::MAX_ROWS) as u32;
        let call = Call::Search {
            query: query.into(),
            limit,
        };
        match self.call(
            Request::run(package, call, Work::Search),
            deadline,
            is_cancelled,
            services,
        )? {
            (Answer::Rows { items }, _) => {
                validate(&items, limit as usize).map_err(|problem| unshowable(problem, package))?;
                Ok(items)
            }
            _ => Err(mismatch()),
        }
    }

    /// Calls `actions[name](data)` in `package`, which the user chose, answering its host
    /// requests with `services`. An action may wait on the network or a program for far longer
    /// than a search may wait for the shared helper, so it runs in a helper of its own.
    pub fn action(
        &self,
        package: &Compiled,
        name: &str,
        data: &serde_json::Value,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<Acted, RunError> {
        let call = Call::Action {
            name: name.into(),
            data: data.clone(),
        };
        match self.alone(
            Request::run(package, call, Work::Action),
            |_| deadline,
            || deadline,
            is_cancelled,
            services,
        )? {
            (Answer::Acted { acted }, _) => Ok(acted),
            _ => Err(mismatch()),
        }
    }

    /// Calls `events[name](event)` in `package`, answering its host requests with `services`,
    /// in a helper of its own so that background work never holds up typing.
    pub fn event(
        &self,
        package: &Compiled,
        name: &str,
        event: &serde_json::Value,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<Handled, RunError> {
        let call = Call::Event {
            name: name.into(),
            event: event.clone(),
        };
        match self.alone(
            Request::run(package, call, Work::Event),
            |_| deadline,
            || deadline,
            is_cancelled,
            services,
        )? {
            (Answer::Handled { handled }, _) => Ok(handled),
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
        services: &Arc<dyn Services>,
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
        let answer = helper.run(id, request, &self.shared, deadline, is_cancelled, services);
        if answer.is_err() {
            lease.helper = None;
        }
        settle(answer)
    }

    /// Runs `request` in a helper started for it alone and stopped when it ends, which neither
    /// waits for nor holds the shared helper. `greet_by` is read from the new helper as it
    /// starts and `deadline` once it is ready.
    fn alone(
        &self,
        request: Request<'_>,
        greet_by: impl FnOnce(&Helper) -> Instant,
        deadline: impl FnOnce() -> Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<(Answer, Vec<u8>), RunError> {
        let mut helper = Helper::start(&self.shared.launch)?;
        match helper.greet(&request, greet_by(&helper), is_cancelled) {
            Greeting::Ready => {}
            Greeting::Waiting(error) | Greeting::Failed(error) => return Err(error),
        }
        let id = self.shared.next_call.fetch_add(1, Ordering::Relaxed);
        let deadline = deadline();
        settle(helper.run(id, request, &self.shared, deadline, is_cancelled, services))
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
    fn run(package: &'a Compiled, call: Call, work: Work) -> Self {
        let (modules, tail) = wire::spans(package.modules());
        let bytes: u64 = tail.iter().map(|part| part.len() as u64).sum();
        Self {
            entry: package.entry(),
            modules,
            tail,
            call,
            work,
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
    events: mpsc::Receiver<Event>,
    /// Where a call's host requests report back, so the call wakes the moment one finishes.
    finished: mpsc::SyncSender<Event>,
    started: Instant,
    greeted: bool,
}

/// What a running call waits for: the helper's next message, or one of its host requests
/// finishing.
enum Event {
    Helper(FromHelper, Vec<u8>),
    Finished {
        run: u64,
        call: u32,
        reply: Vec<u8>,
    },
    /// The helper's socket closed: it stopped or broke the wire format.
    Closed,
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
        let (events, received) = mpsc::sync_channel(1);
        let finished = events.clone();
        let mut reader = ours.try_clone().map_err(unavailable)?;
        ours.set_write_timeout(Some(HANDSHAKE)).map_err(unavailable)?;
        std::thread::Builder::new()
            .name("extension-host-replies".into())
            .spawn(move || {
                while let Ok(Some((message, tail))) = wire::read::<FromHelper>(&mut reader) {
                    if events.send(Event::Helper(message, tail)).is_err() {
                        return;
                    }
                }
                let _ = events.send(Event::Closed);
            })
            .map_err(unavailable)?;
        Ok(Self {
            child,
            socket: ours,
            events: received,
            finished,
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
            match self.events.recv_timeout(TICK) {
                Ok(Event::Helper(FromHelper::Hello { version, engine }, _))
                    if version == wire::VERSION && engine == dupin_js_runtime::engine_version() =>
                {
                    self.greeted = true;
                }
                Ok(Event::Helper(FromHelper::Hello { .. }, _)) => {
                    return Greeting::Failed(RunError::Failed(
                        host_error("the extension host is from another Dupin build")
                            .hint("Quit and reopen Dupin after an update."),
                    ));
                }
                Ok(Event::Helper(FromHelper::Refused { reason }, _)) => {
                    return Greeting::Failed(refused(reason));
                }
                Ok(Event::Helper(FromHelper::Done { .. } | FromHelper::Host { .. }, _) | Event::Closed)
                | Err(RecvTimeoutError::Disconnected) => {
                    return didnt_start();
                }
                // No call has run on this helper yet, so no request of one can have finished.
                Ok(Event::Finished { .. }) => {}
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
        let quiet = !self.greeted
            || loop {
                match self.events.try_recv() {
                    // A request of an ended call, whose reply nothing waits for.
                    Ok(Event::Finished { .. }) => {}
                    Ok(Event::Helper(..) | Event::Closed) | Err(mpsc::TryRecvError::Disconnected) => {
                        break false;
                    }
                    Err(mpsc::TryRecvError::Empty) => break true,
                }
            };
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
        shared: &Shared,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<(Answer, Vec<u8>), RunError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let message = ToHelper::Run {
            id,
            deadline_ms: remaining.as_millis() as u64,
            entry: request.entry.into(),
            modules: request.modules,
            call: request.call,
            environment: shared.environment,
        };
        self.send(&message, &request.tail)?;
        let footprint_limit = FOOTPRINT_ALLOWANCE + request.allowance;
        let mut cancel_by = None;
        let mut requests = Requests::new(
            services.clone(),
            deadline,
            id,
            self.finished.clone(),
            &shared.workers,
        );
        loop {
            match self.events.recv_timeout(TICK) {
                Ok(Event::Helper(FromHelper::Done { id: answered, answer }, tail)) if answered == id => {
                    return Ok((answer, tail));
                }
                Ok(Event::Helper(FromHelper::Host { id: asking, call, op }, args)) if asking == id => {
                    let unstarted = requests.start(call, op, args).map_err(|problem| {
                        RunError::Failed(host_error(format!(
                            "the extension host broke the rules for host requests: {problem}"
                        )))
                    })?;
                    if let Some(reply) = unstarted {
                        requests.waiting -= 1;
                        self.send(&ToHelper::Reply { id, call }, &[&reply])?;
                    }
                }
                // A request of an earlier call that ended before it finished is dropped.
                Ok(Event::Finished { run, call, reply }) => {
                    if run == id {
                        requests.waiting -= 1;
                        self.send(&ToHelper::Reply { id, call }, &[&reply])?;
                    }
                }
                // Every call given up on stops the helper, so no other reply is ever due.
                Ok(Event::Helper(..)) => {
                    return Err(RunError::Failed(host_error(
                        "the extension host answered out of turn",
                    )));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Ok(Event::Closed) | Err(RecvTimeoutError::Disconnected) => {
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

/// The host requests of one run: each runs on a thread of its own and reports to the run's
/// events as it finishes. A reply that comes after the run ends is dropped.
struct Requests<'a> {
    services: Arc<dyn Services>,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    run: u64,
    finished: mpsc::SyncSender<Event>,
    workers: &'a Workers,
    last: u32,
    waiting: usize,
}

impl<'a> Requests<'a> {
    fn new(
        services: Arc<dyn Services>,
        deadline: Instant,
        run: u64,
        finished: mpsc::SyncSender<Event>,
        workers: &'a Workers,
    ) -> Self {
        Self {
            services,
            deadline,
            cancelled: Arc::default(),
            run,
            finished,
            workers,
            last: 0,
            waiting: 0,
        }
    }

    /// Starts request `call`, or answers it at once when it can't start. The guest runtime keeps
    /// to these bounds itself, so a helper that breaks one has been subverted and is stopped.
    fn start(&mut self, call: u32, op: String, args: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        if call != self.last + 1 || call > dupin_js_runtime::MAX_CALLS {
            return Err("a request out of sequence");
        }
        if self.waiting >= dupin_js_runtime::MAX_PENDING {
            return Err("too many requests waiting");
        }
        if op.len() > dupin_js_runtime::MAX_OP_BYTES || args.len() > dupin_js_runtime::MAX_ARGS_BYTES {
            return Err("a request too large");
        }
        self.last = call;
        self.waiting += 1;
        let services = self.services.clone();
        let cancelled = self.cancelled.clone();
        let finished = self.finished.clone();
        let (deadline, run) = (self.deadline, self.run);
        let started = self.workers.run(Box::new(move || {
            let reply = services.call(&op, &args, deadline, &cancelled);
            if !cancelled.load(Ordering::Relaxed) {
                let _ = finished.send(Event::Finished { run, call, reply });
            }
        }));
        Ok(started.is_err().then(|| {
            br#"{"error":{"code":"unavailable","message":"Dupin couldn't start the request"}}"#.to_vec()
        }))
    }
}

impl Drop for Requests<'_> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

type Job = Box<dyn FnOnce() + Send>;

/// Threads that run host requests, each kept for [`IDLE`] after its last, so a request a guest
/// makes while the user types doesn't wait for a thread to start.
#[derive(Default)]
struct Workers(Arc<Mutex<Vec<mpsc::Sender<Job>>>>);

impl Workers {
    fn run(&self, mut job: Job) -> io::Result<()> {
        loop {
            let idle = self.0.lock().unwrap_or_else(PoisonError::into_inner).pop();
            let Some(worker) = idle else { break };
            // A worker that has stopped hands the job back.
            match worker.send(job) {
                Ok(()) => return Ok(()),
                Err(mpsc::SendError(back)) => job = back,
            }
        }
        let (worker, jobs) = mpsc::channel::<Job>();
        let pool = Arc::downgrade(&self.0);
        std::thread::Builder::new()
            .name("extension-host-request".into())
            .spawn(move || {
                loop {
                    job();
                    let Some(pool) = pool.upgrade() else { return };
                    pool.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(worker.clone());
                    drop(pool);
                    match jobs.recv_timeout(IDLE) {
                        Ok(next) => job = next,
                        Err(_) => return,
                    }
                }
            })
            .map(drop)
    }
}

fn no_services() -> Arc<dyn Services> {
    Arc::new(NoServices)
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

/// The helper is another process, so its rows are checked again here, against the limits the
/// reader in the helper applied.
fn validate(items: &[Row], limit: usize) -> Result<(), String> {
    if items.len() > limit {
        return Err(format!("{} rows where {limit} were asked for", items.len()));
    }
    let mut ids = HashSet::new();
    let mut bytes = 0;
    for item in items {
        item.validate(Source::Search)?;
        if !ids.insert(item.id.as_str()) {
            return Err(format!("`{}` is the id of more than one row", item.id));
        }
        bytes += item.text_bytes();
    }
    if bytes > MAX_RESPONSE_BYTES {
        return Err("more text than an answer may carry".into());
    }
    Ok(())
}

fn unshowable(problem: String, package: &Compiled) -> RunError {
    RunError::Failed(
        host_error(format!(
            "the extension host returned rows Dupin can't show: {problem}"
        ))
        .file(package.entry()),
    )
}

/// A helper's answer as the call's result: its own failure or cancellation becomes the error.
fn settle(answer: Result<(Answer, Vec<u8>), RunError>) -> Result<(Answer, Vec<u8>), RunError> {
    match answer? {
        (Answer::Cancelled, _) => Err(RunError::Cancelled),
        (Answer::Failed { diagnostic }, _) => Err(RunError::Failed(diagnostic)),
        answer => Ok(answer),
    }
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
        Work::Items => {
            "Make items return sooner: Dupin stops loading a catalogue 2 seconds after it asks, counting every part."
        }
        Work::Action | Work::Event => {
            "Do less in one call: Dupin stops actions and event handlers at their deadline."
        }
    };
    Diagnostic::new(
        "deadline",
        format!("{} didn't finish before its deadline", work.doing()),
    )
    .file(entry)
    .hint(hint)
}
