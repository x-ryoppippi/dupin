//! Dupin's side: starts a resident helper, on first use, for the searches of each trust domain,
//! and a helper of its own for each compile, action and event, and stops a helper when a call
//! outlives its deadline, ignores a cancellation, outgrows its memory or dies, or when it works
//! or grows while it waits for a call.

use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use std::collections::{HashMap, HashSet};

use dupin_js_runtime::row::{RowAction, RowEffect, Source, is_dictionary_url};
use dupin_js_runtime::{
    Acted, CataloguePage, Diagnostic, Environment, Exports, FilterHit, FilterInput, Handled, PageContext,
    Reason, Row, RunError, Sources, ViewAnswer,
};
use dupin_plugin_protocol::{MAX_RESPONSE_BYTES, is_web_url};
use dupin_spawn::{Child, Command, Stdio};

use crate::wire::{self, Answer, Call, FromHelper, Preferences, ToHelper};

/// How long a call may run past its deadline, or past a cancellation, before the helper is
/// stopped. The helper's own interrupt normally ends it well inside this. Past a cancellation
/// it is the helper's processor time, so a helper that is only waiting for a processor on a busy
/// machine isn't taken for one that ignores the cancel; the deadline still bounds the call.
const GRACE: Duration = Duration::from_millis(150);
/// How often a running call is checked for cancellation, its deadline and memory.
const TICK: Duration = Duration::from_millis(5);
const HANDSHAKE: Duration = Duration::from_secs(5);
/// Calls waiting for the helper while another runs. One query per extension can wait, since a
/// provider's newer query replaces its older one.
const MAX_WAITING: usize = 64;
const IDLE: Duration = Duration::from_secs(120);
/// How often the helpers waiting for a call are checked.
const WATCH: Duration = Duration::from_secs(1);
/// The processor time a helper may use between answering one call and being sent the next:
/// enough to make the next call's engine and drop the last one's. A helper runs nothing else
/// while it waits, so one that spends more is doing work nobody asked for.
const RESTING_CPU: Duration = Duration::from_millis(250);
/// Resident helpers kept at once; the one idle longest is stopped to make room.
const MAX_RESIDENT: usize = 8;
/// The helper's footprint beyond the package a call carries: the engine's 16 MiB heap,
/// allocator overhead past it, the stack and the process itself.
const FOOTPRINT_ALLOWANCE: u64 = 64 * 1024 * 1024;

/// Makes the command that starts a helper; the host connects its standard streams.
pub type Launcher = Box<dyn Fn() -> io::Result<Command> + Send + Sync>;

/// One host request: the run that made it, named uniquely in this process, and its number in
/// that run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HostRequest {
    pub run: u64,
    pub call: u32,
}

/// Dupin's host services as one call sees them. The caller builds one per call, for the package
/// and the reason it runs (typing, a chosen action, a watched event), so the helper can't claim
/// either.
pub trait Services: Send + Sync {
    /// Answers `request`, operation `op` with `args` as the guest sent them, with JSON
    /// `{"ok": value}` or `{"error": {"code", "message", …}}`. Runs on a thread of its own.
    /// `cancelled` is set once the run has ended, after which the reply is dropped.
    fn call(
        &self,
        request: HostRequest,
        op: &str,
        args: &[u8],
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Vec<u8>;
    /// Run `run` has ended, however it ended, with `cancelled` set for each of its requests.
    /// The guest took the replies to the requests numbered in `answered` and to no others, so
    /// whatever another request of the run left for the user to answer is withdrawn.
    fn ended(&self, run: u64, answered: &[u32]) {
        let _ = (run, answered);
    }
    /// Whether one of this call's replies gave the guest `url`, a link not on the web such as
    /// a calendar event's, which its rows may then open.
    fn opens(&self, url: &str) -> bool {
        let _ = url;
        false
    }
}

/// No host services: every request fails as unavailable.
pub struct NoServices;

impl Services for NoServices {
    fn call(
        &self,
        _request: HostRequest,
        _op: &str,
        _args: &[u8],
        _deadline: Instant,
        _cancelled: &AtomicBool,
    ) -> Vec<u8> {
        br#"{"error":{"code":"unavailable","message":"host services are unavailable here"}}"#.to_vec()
    }
}

/// A package's bytecode, as the extension host compiled it. Dupin keeps it as opaque bytes and
/// only ever sends it back to the host, since QuickJS doesn't validate bytecode it loads.
#[derive(Clone)]
pub struct Compiled {
    entry: String,
    modules: Arc<[(String, Box<[u8]>)]>,
    /// What Dupin calls this compile in every run of it, so a helper that holds its bytecode is
    /// sent only this. No other compile in this process has it.
    name: u64,
    /// The resident helper its calls run in, shared only by compiles of the same domain.
    domain: u64,
}

/// Whose code a package's calls may share a resident helper with. A helper answers each call's
/// host requests with the authority of the package the call runs, so code that subverts a
/// helper acts as every package that helper runs later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// Ships with Dupin, sealed by its build: shares a helper with the other bundled packages.
    Bundled,
    /// Any other package: its calls run in a helper no other package's code reaches.
    Own,
}

/// The name of the next compile. Names start above [`BUNDLED`], so no compile's own domain is
/// the bundled packages'.
static NEXT_COMPILE: AtomicU64 = AtomicU64::new(1);

/// The domain of every bundled package.
const BUNDLED: u64 = 0;

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
        let name = NEXT_COMPILE.fetch_add(1, Ordering::Relaxed);
        Self {
            entry: dupin_js_runtime::DEFAULT_ENTRY.into(),
            modules: Arc::from([]),
            name,
            domain: name,
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

/// A resident helper process for the searches and catalogues of each trust domain, used by one
/// call at a time, and a helper of its own for each compile, action and event.
pub struct Host {
    shared: Arc<Shared>,
}

struct Shared {
    launch: Launcher,
    environment: Option<Environment>,
    clock: Clock,
    /// [`TICK`], unless a test checks that nothing waits for it.
    tick: Duration,
    workers: Arc<Workers>,
    state: Mutex<State>,
    released: Condvar,
    next_call: AtomicU64,
}

#[derive(Default)]
struct State {
    /// The resident helper of each trust domain.
    slots: HashMap<u64, Slot>,
    reaping: bool,
}

#[derive(Default)]
struct Slot {
    helper: Option<Helper>,
    busy: bool,
    waiting: usize,
    idle_since: Option<Instant>,
}

impl Slot {
    fn is_unused(&self) -> bool {
        self.helper.is_none() && !self.busy && self.waiting == 0
    }
}

/// What a call does, for the diagnostics that name it.
#[derive(Clone, Copy)]
enum Work {
    Compile,
    Search,
    Items,
    View,
    Action,
    Event,
    Filter,
}

impl Work {
    fn doing(self) -> &'static str {
        match self {
            Work::Compile => "compiling the package",
            Work::Search => "search",
            Work::Items => "items",
            Work::View => "the view",
            Work::Action => "the action",
            Work::Event => "the event handler",
            Work::Filter => "the filter",
        }
    }

    fn whose(self) -> &'static str {
        match self {
            Work::Compile => "the compile's",
            Work::Search => "search's",
            Work::Items => "items'",
            Work::View => "the view's",
            Work::Action => "the action's",
            Work::Event => "the event handler's",
            Work::Filter => "the filter's",
        }
    }
}

impl Host {
    pub fn new(launch: Launcher) -> Self {
        Self::fixed(launch, None)
    }

    /// A host whose guests see the clock and `Math.random` `environment` fixes, as a fixture
    /// replays them; with none, the real ones.
    pub(crate) fn fixed(launch: Launcher, environment: Option<Environment>) -> Self {
        Self::ticking(launch, environment, TICK)
    }

    /// A host for a fixture's replay, as [`Host::fixed`] but measuring its calls' deadlines in
    /// the processor time its helpers spend on them, from [`Host::now`]: a replay checks what
    /// the guest does in the time the launcher gives it, which a busy machine's wait for a
    /// processor isn't part of.
    pub fn replaying(launch: Launcher, environment: Option<Environment>) -> Self {
        Self::build(
            launch,
            environment,
            TICK,
            Clock::Processor {
                origin: Instant::now(),
                spent: Mutex::default(),
            },
        )
    }

    pub(crate) fn ticking(launch: Launcher, environment: Option<Environment>, tick: Duration) -> Self {
        Self::build(launch, environment, tick, Clock::Wall)
    }

    fn build(launch: Launcher, environment: Option<Environment>, tick: Duration, clock: Clock) -> Self {
        Self {
            shared: Arc::new(Shared {
                launch,
                environment,
                clock,
                tick,
                workers: Arc::default(),
                state: Mutex::default(),
                released: Condvar::new(),
                next_call: AtomicU64::new(1),
            }),
        }
    }

    /// The moment on this host's clock, which every deadline handed to it is measured against:
    /// the wall clock's, or for a [`Host::replaying`] host, a moment its helpers' work advances.
    pub fn now(&self) -> Instant {
        self.shared.clock.now()
    }

    /// Compiles every module of `sources` in a helper started for this compile alone, which
    /// reports syntax errors and imports that leave the package or name a missing file.
    /// `limit` is how long compiling may take once that helper is ready. `trust` decides which
    /// other packages' calls may share a resident helper with this package's.
    ///
    /// Every later helper loads the bytecode unvalidated, so no other package's code may have
    /// reached the helper that writes it, and what a subverted compile wrote reaches only
    /// helpers of its own domain. Not waiting for a resident helper also keeps a busy extension
    /// from failing another's load.
    pub fn compile(
        &self,
        sources: &Sources,
        trust: Trust,
        limit: Duration,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Compiled, RunError> {
        let (modules, tail) = wire::spans(sources.files().map(|(name, source)| (name, source.as_bytes())));
        let bytes: u64 = tail.iter().map(|part| part.len() as u64).sum();
        // The compiler's own heap, then the source, its copies and the bytecode written from it.
        let allowance = dupin_js_runtime::COMPILE_HEAP_BYTES as u64 + 4 * bytes;
        let request = Request {
            entry: sources.entry(),
            package: None,
            modules,
            tail,
            call: Call::Compile,
            work: Work::Compile,
            allowance,
        };
        let (answer, tail) = self.alone(request, None, |now| now + limit, is_cancelled, &no_services())?;
        let Answer::Compiled { modules } = answer else {
            return Err(mismatch());
        };
        let modules = wire::split(modules, &tail).map_err(|error| {
            RunError::Failed(host_error(format!(
                "the extension host returned a package Dupin can't read: {error}"
            )))
        })?;
        let name = NEXT_COMPILE.fetch_add(1, Ordering::Relaxed);
        Ok(Compiled {
            entry: sources.entry().into(),
            modules: modules
                .into_iter()
                .map(|(name, bytecode)| (name, Box::from(bytecode)))
                .collect(),
            name,
            domain: match trust {
                Trust::Bundled => BUNDLED,
                Trust::Own => name,
            },
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
        match self.call(package.domain, request, deadline, is_cancelled, &no_services())? {
            (Answer::Checked { exports }, _) => Ok(exports),
            _ => Err(mismatch()),
        }
    }

    /// Calls `items({ cursor, reason, preferences })` in `package` for the next part of its
    /// catalogue, answering its host requests with `services`.
    #[allow(clippy::too_many_arguments)]
    pub fn items(
        &self,
        package: &Compiled,
        cursor: Option<&str>,
        reason: Reason,
        preferences: &Preferences,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<CataloguePage, RunError> {
        let call = Call::Items {
            cursor: cursor.map(str::to_owned),
            reason,
            preferences: preferences.clone(),
        };
        let request = Request::run(package, call, Work::Items);
        catalogue_page(
            self.call(package.domain, request, deadline, is_cancelled, services)?,
            package,
        )
    }

    /// As [`Host::items`], in a helper of its own, as Dupin refreshing a catalogue nobody is
    /// waiting for does: however long the package's programs take, it never holds up the
    /// searches of the resident helper.
    #[allow(clippy::too_many_arguments)]
    pub fn items_alone(
        &self,
        package: &Compiled,
        cursor: Option<&str>,
        reason: Reason,
        preferences: &Preferences,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<CataloguePage, RunError> {
        let call = Call::Items {
            cursor: cursor.map(str::to_owned),
            reason,
            preferences: preferences.clone(),
        };
        let request = Request::run(package, call, Work::Items);
        catalogue_page(
            self.alone(request, Some(deadline), |_| deadline, is_cancelled, services)?,
            package,
        )
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
            package.domain,
            Request::run(package, call, Work::Search),
            deadline,
            is_cancelled,
            services,
        )? {
            (Answer::Rows { items }, _) => {
                validate(&items, limit as usize).map_err(|problem| unshowable(problem, package))?;
                issued(
                    items.iter().flat_map(|item| &item.actions),
                    services.as_ref(),
                    package,
                )?;
                Ok(items)
            }
            _ => Err(mismatch()),
        }
    }

    /// Calls `views[name]({ query, state, preferences })` in `package` for a page the user
    /// opened, answering its host requests with `services`. A page is searched as the keyword
    /// is, so it runs in the resident helper as a search does.
    #[allow(clippy::too_many_arguments)]
    pub fn view(
        &self,
        package: &Compiled,
        name: &str,
        page: &PageContext,
        preferences: &serde_json::Value,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<ViewAnswer, RunError> {
        let call = Call::View {
            name: name.into(),
            page: Box::new(page.clone()),
            preferences: Preferences::encode(preferences),
        };
        match self.call(
            package.domain,
            Request::run(package, call, Work::View),
            deadline,
            is_cancelled,
            services,
        )? {
            (Answer::View { view }, _) => {
                view.validate().map_err(|problem| unshowable(problem, package))?;
                let (rows, actions): (&[Row], &[RowAction]) = match &view {
                    ViewAnswer::List { rows, .. } => (rows, &[]),
                    ViewAnswer::Detail { actions, .. } => (&[], actions),
                    ViewAnswer::Form(_) => (&[], &[]),
                };
                let offered = rows.iter().flat_map(|row| &row.actions).chain(actions);
                issued(offered, services.as_ref(), package)?;
                Ok(view)
            }
            _ => Err(mismatch()),
        }
    }

    /// Calls `actions[name](data, { query, state, preferences })` in `package`, which the user
    /// chose on `page`, answering its host requests with `services`. An action may wait on the
    /// network or a program for far longer than a search may wait for the shared helper, so it
    /// runs in a helper of its own.
    #[allow(clippy::too_many_arguments)]
    pub fn action(
        &self,
        package: &Compiled,
        name: &str,
        data: &serde_json::Value,
        page: &PageContext,
        preferences: &serde_json::Value,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<Acted, RunError> {
        let call = Call::Action {
            name: name.into(),
            data: data.clone(),
            page: Box::new(page.clone()),
            preferences: Preferences::encode(preferences),
        };
        match self.alone(
            Request::run(package, call, Work::Action),
            Some(deadline),
            |_| deadline,
            is_cancelled,
            services,
        )? {
            (Answer::Acted { acted }, _) => {
                if let Some(effect) = &acted.effect {
                    effect.validate().map_err(|problem| {
                        RunError::Failed(
                            host_error(format!(
                                "the extension host returned an effect Dupin can't carry out: {problem}"
                            ))
                            .file(package.entry()),
                        )
                    })?;
                }
                Ok(acted)
            }
            _ => Err(mismatch()),
        }
    }

    /// Calls `events[name](event, { preferences })` in `package`, answering its host requests
    /// with `services`, in a helper of its own so that background work never holds up typing.
    #[allow(clippy::too_many_arguments)]
    pub fn event(
        &self,
        package: &Compiled,
        name: &str,
        event: &serde_json::Value,
        preferences: &serde_json::Value,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<Handled, RunError> {
        let call = Call::Event {
            name: name.into(),
            event: event.clone(),
            preferences: Preferences::encode(preferences),
        };
        match self.alone(
            Request::run(package, call, Work::Event),
            Some(deadline),
            |_| deadline,
            is_cancelled,
            services,
        )? {
            (Answer::Handled { handled }, _) => Ok(handled),
            _ => Err(mismatch()),
        }
    }

    /// Calls `filters[name](input)` in `package` for one batch of a source it is composed onto.
    /// It runs as the user types, so in the resident helper as a search does, and with
    /// `services` answering nothing: a filter has no external effects or persistent state.
    #[allow(clippy::too_many_arguments)]
    pub fn filter(
        &self,
        package: &Compiled,
        name: &str,
        input: &FilterInput,
        preferences: &serde_json::Value,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<Vec<FilterHit>, RunError> {
        let call = Call::Filter {
            name: name.into(),
            input: Box::new(input.clone()),
            preferences: Preferences::encode(preferences),
        };
        match self.call(
            package.domain,
            Request::run(package, call, Work::Filter),
            deadline,
            is_cancelled,
            services,
        )? {
            (Answer::Filtered { hits }, _) => Ok(hits),
            _ => Err(mismatch()),
        }
    }

    /// The process ID of the resident helper that runs `package`'s calls, for tests and
    /// measurements.
    pub fn helper_pid(&self, package: &Compiled) -> Option<u32> {
        let state = self.shared.lock();
        let helper = state.slots.get(&package.domain)?.helper.as_ref()?;
        Some(helper.child.id())
    }

    /// The process ID of the resident helper that runs `package`'s calls and the processor time
    /// it has used, for tests and measurements that mean the guest's work, not the wall clock.
    pub fn helper_processor_time(&self, package: &Compiled) -> Option<(u32, Duration)> {
        let pid = self.helper_pid(package)?;
        Some((pid, processor_time(pid)?))
    }

    fn call(
        &self,
        domain: u64,
        request: Request<'_>,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<(Answer, Vec<u8>), RunError> {
        let mut lease = self.lease(domain, &request, deadline, is_cancelled)?;
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
        let meter = Meter::start(&self.shared.clock, helper, Some(deadline));
        match helper.greet(&request, &meter, self.shared.tick, is_cancelled) {
            Greeting::Ready => {}
            // A helper still starting is kept, so the next call doesn't pay for another start.
            Greeting::Waiting(error) => return Err(error),
            Greeting::Failed(error) => {
                drop(meter);
                lease.helper = None;
                return Err(error);
            }
        }
        let answer = helper.run(id, request, &self.shared, &meter, is_cancelled, services);
        drop(meter);
        if answer.is_err() {
            lease.helper = None;
        }
        settle(answer)
    }

    /// Runs `request` in a helper started for it alone and stopped when it ends, which neither
    /// waits for nor holds a resident helper. The helper must be ready by `greet_by`, or within
    /// the handshake without one, and `deadline` is read, given the clock's moment, once it is.
    fn alone(
        &self,
        request: Request<'_>,
        greet_by: Option<Instant>,
        deadline: impl FnOnce(Instant) -> Instant,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<(Answer, Vec<u8>), RunError> {
        let mut helper = Helper::start(&self.shared.launch)?;
        let mut meter = Meter::start(&self.shared.clock, &helper, greet_by);
        match helper.greet(&request, &meter, self.shared.tick, is_cancelled) {
            Greeting::Ready => {}
            Greeting::Waiting(error) | Greeting::Failed(error) => return Err(error),
        }
        let id = self.shared.next_call.fetch_add(1, Ordering::Relaxed);
        meter.deadline = Some(deadline(meter.now()));
        let answer = helper.run(id, request, &self.shared, &meter, is_cancelled, services);
        drop(meter);
        settle(answer)
    }

    /// Waits for `domain`'s helper to be free, unless the caller gives up first.
    fn lease(
        &self,
        domain: u64,
        request: &Request<'_>,
        deadline: Instant,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Lease<'_>, RunError> {
        let mut state = self.shared.lock();
        let slot = state.slots.entry(domain).or_default();
        if slot.busy && slot.waiting >= MAX_WAITING {
            return Err(RunError::Failed(
                Diagnostic::new("host-busy", "too many extension calls are waiting")
                    .file(request.entry)
                    .hint("Try again; other extensions are still answering."),
            ));
        }
        slot.waiting += 1;
        loop {
            let slot = state.slots.entry(domain).or_default();
            if !slot.busy {
                slot.waiting -= 1;
                slot.busy = true;
                return Ok(Lease {
                    helper: slot.helper.take(),
                    domain,
                    shared: &self.shared,
                });
            }
            if is_cancelled() {
                slot.waiting -= 1;
                return Err(RunError::Cancelled);
            }
            if self.shared.clock.now() >= deadline {
                slot.waiting -= 1;
                return Err(RunError::Failed(deadline_passed(request.work, request.entry)));
            }
            state = self
                .shared
                .released
                .wait_timeout(state, self.shared.tick)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }
}

/// One call to the helper, with the package it carries.
struct Request<'a> {
    entry: &'a str,
    /// The compile it runs, which the helper may hold; none for a compile.
    package: Option<u64>,
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
            package: Some(package.name),
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

/// A domain's helper, held by one call; it goes back to the host when the call ends.
struct Lease<'a> {
    helper: Option<Helper>,
    domain: u64,
    shared: &'a Arc<Shared>,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        let slot = state.slots.entry(self.domain).or_default();
        slot.helper = self.helper.take().map(Helper::rest);
        slot.busy = false;
        slot.idle_since = Some(Instant::now());
        let resident = state.slots.values().filter(|slot| slot.helper.is_some()).count();
        if resident > MAX_RESIDENT {
            let oldest = state
                .slots
                .iter_mut()
                .filter(|(domain, slot)| **domain != self.domain && slot.helper.is_some())
                .min_by_key(|(_, slot)| slot.idle_since);
            if let Some((_, slot)) = oldest {
                slot.helper = None;
            }
        }
        state.slots.retain(|_, slot| !slot.is_unused());
        if !state.slots.is_empty() && !state.reaping {
            state.reaping = spawn_reaper(Arc::downgrade(self.shared));
        }
        drop(state);
        self.shared.released.notify_all();
    }
}

/// Stops a helper nobody has used for [`IDLE`], so an unused extension costs no process, and
/// one that is no longer idle while it waits for a call.
fn spawn_reaper(shared: Weak<Shared>) -> bool {
    std::thread::Builder::new()
        .name("extension-host-reaper".into())
        .spawn(move || {
            loop {
                std::thread::sleep(WATCH);
                let Some(shared) = shared.upgrade() else { return };
                let mut state = shared.lock();
                for slot in state.slots.values_mut() {
                    let idle = slot.idle_since.is_some_and(|since| since.elapsed() >= IDLE);
                    if idle || slot.helper.as_mut().is_some_and(|helper| !helper.is_idle()) {
                        slot.helper = None;
                    }
                }
                state.slots.retain(|_, slot| !slot.is_unused());
                if state.slots.is_empty() {
                    state.reaping = false;
                    return;
                }
            }
        })
        .is_ok()
}

struct Helper {
    child: Child,
    /// Shared with the threads that answer host requests, which reply as soon as they finish.
    socket: Arc<Mutex<UnixStream>>,
    /// Bounded, so a helper that writes without being asked stalls on its own socket instead
    /// of growing Dupin's memory.
    replies: mpsc::Receiver<(FromHelper, Vec<u8>)>,
    started: Instant,
    greeted: bool,
    /// What this helper holds of the compiles it was sent: it holds them by the same rules.
    held: wire::Held<()>,
    /// The footprint its last call was allowed, which it keeps to while it waits for the next.
    footprint_limit: u64,
    /// Its processor time when it answered its last call.
    resting_since: Option<Duration>,
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
        let mut reader = io::BufReader::new(ours.try_clone().map_err(unavailable)?);
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
            socket: Arc::new(Mutex::new(ours)),
            replies: received,
            started: Instant::now(),
            greeted: false,
            held: wire::Held::default(),
            footprint_limit: FOOTPRINT_ALLOWANCE + wire::MAX_HELD_BYTES as u64,
            resting_since: None,
        })
    }

    /// Waits for the helper's hello, but no longer than the call may wait, if it has a deadline.
    fn greet(
        &mut self,
        request: &Request<'_>,
        meter: &Meter<'_>,
        tick: Duration,
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
            match self.replies.recv_timeout(tick) {
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
                Ok((FromHelper::Done { .. } | FromHelper::Host { .. }, _))
                | Err(RecvTimeoutError::Disconnected) => {
                    return didnt_start();
                }
                Err(RecvTimeoutError::Timeout) => {
                    if meter.since(self.started) >= HANDSHAKE {
                        return didnt_start();
                    }
                    if is_cancelled() {
                        return Greeting::Waiting(RunError::Cancelled);
                    }
                    if meter.passed(Duration::ZERO) {
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

    /// Marks the helper as waiting for its next call from now.
    fn rest(mut self) -> Self {
        self.resting_since = processor_time(self.child.id());
        self
    }

    /// Whether the helper is alive and, once greeted, has sent nothing since its last answer,
    /// used no more than [`RESTING_CPU`] and kept to its last call's footprint. It speaks and
    /// works only to answer a call, so anything sent or done between calls, such as a reply
    /// forged for a later call or threads it started itself, means it can't be trusted with
    /// another.
    fn is_idle(&mut self) -> bool {
        let quiet = !self.greeted || matches!(self.replies.try_recv(), Err(mpsc::TryRecvError::Empty));
        let pid = self.child.id();
        let rested = self.resting_since.is_none_or(|since| {
            processor_time(pid).is_none_or(|now| now.saturating_sub(since) <= RESTING_CPU)
        });
        let small = footprint(pid).is_none_or(|bytes| bytes <= self.footprint_limit);
        quiet && rested && small && matches!(self.child.try_wait(), Ok(None))
    }

    fn send(&mut self, message: &ToHelper, tail: &[&[u8]]) -> Result<(), RunError> {
        write(&self.socket, message, tail).map_err(|error| {
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

    /// Runs `request`, sending its package's bytecode unless this helper holds it.
    fn run(
        &mut self,
        id: u64,
        request: Request<'_>,
        shared: &Shared,
        meter: &Meter<'_>,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<(Answer, Vec<u8>), RunError> {
        let mut answer = self.exchange(id, &request, shared, meter, is_cancelled, services)?;
        if matches!(answer.0, Answer::Missing) && request.package.is_some_and(|name| self.held.forget(name)) {
            answer = self.exchange(id, &request, shared, meter, is_cancelled, services)?;
        }
        if matches!(answer.0, Answer::Missing) {
            return Err(mismatch());
        }
        Ok(answer)
    }

    fn exchange(
        &mut self,
        id: u64,
        request: &Request<'_>,
        shared: &Shared,
        meter: &Meter<'_>,
        is_cancelled: &dyn Fn() -> bool,
        services: &Arc<dyn Services>,
    ) -> Result<(Answer, Vec<u8>), RunError> {
        let held = request.package.is_some_and(|name| self.held.run(name).is_some());
        if let Some(name) = request.package.filter(|_| !held) {
            let bytes = request.tail.iter().map(|part| part.len()).sum();
            self.held.hold(name, (), bytes);
        }
        let remaining = meter.remaining();
        let (modules, tail): (_, &[&[u8]]) = if held {
            (None, &[])
        } else {
            (Some(request.modules.clone()), &request.tail)
        };
        let message = ToHelper::Run {
            id,
            deadline_ms: remaining.as_millis() as u64,
            processor_time: matches!(shared.clock, Clock::Processor { .. }),
            entry: request.entry.into(),
            package: request.package,
            modules,
            call: request.call.clone(),
            environment: shared.environment,
        };
        self.send(&message, tail)?;
        let footprint_limit = FOOTPRINT_ALLOWANCE + wire::MAX_HELD_BYTES as u64 + request.allowance;
        self.footprint_limit = footprint_limit;
        let mut cancelled = None;
        let mut requests = Requests::new(services.clone(), id, self.socket.clone(), shared.workers.clone());
        loop {
            match self.replies.recv_timeout(shared.tick) {
                Ok((
                    FromHelper::Done {
                        id: done,
                        answer,
                        answered,
                    },
                    tail,
                )) if done == id => {
                    requests.answered = answered;
                    return Ok((answer, tail));
                }
                Ok((FromHelper::Host { id: asking, call, op }, args)) if asking == id => {
                    requests.start(call, op, args, meter.due()).map_err(|problem| {
                        RunError::Failed(host_error(format!(
                            "the extension host broke the rules for host requests: {problem}"
                        )))
                    })?;
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
            if cancelled.is_none() && is_cancelled() {
                let _ = self.send(&ToHelper::Cancel { id }, &[]);
                cancelled = Some(Since::now(self.child.id(), now));
            }
            if cancelled.is_some_and(|since| since.elapsed(self.child.id(), now) >= GRACE) {
                return Err(RunError::Cancelled);
            }
            if meter.passed(GRACE) {
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

/// Names each run for its services, uniquely across every host in the process.
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);

/// The host requests of one run: each runs on a worker thread of its own, which writes its
/// reply to the helper the moment it has one, so a guest awaiting it never waits for the run's
/// next check. A reply that comes after the run ends is dropped, and when the run ends its
/// services learn which replies the guest took.
struct Requests {
    services: Arc<dyn Services>,
    cancelled: Arc<AtomicBool>,
    /// The helper's id for the run.
    run: u64,
    /// The services' name for the run.
    token: u64,
    socket: Arc<Mutex<UnixStream>>,
    workers: Arc<Workers>,
    last: u32,
    waiting: Arc<AtomicUsize>,
    /// The requests whose replies were written to the helper.
    sent: Arc<Mutex<HashSet<u32>>>,
    /// The requests whose replies the helper says the guest took, once it has answered.
    answered: Vec<u32>,
}

impl Requests {
    fn new(
        services: Arc<dyn Services>,
        run: u64,
        socket: Arc<Mutex<UnixStream>>,
        workers: Arc<Workers>,
    ) -> Self {
        Self {
            services,
            cancelled: Arc::default(),
            run,
            token: NEXT_RUN.fetch_add(1, Ordering::Relaxed),
            socket,
            workers,
            last: 0,
            waiting: Arc::default(),
            sent: Arc::default(),
            answered: Vec::new(),
        }
    }

    /// Starts request `call`, due as `due` says. The guest runtime keeps to these bounds itself,
    /// so a helper that breaks one has been subverted and is stopped.
    fn start(&mut self, call: u32, op: String, args: Vec<u8>, due: Due) -> Result<(), &'static str> {
        if call != self.last + 1 || call > dupin_js_runtime::MAX_CALLS {
            return Err("a request out of sequence");
        }
        if self.waiting.load(Ordering::Acquire) >= dupin_js_runtime::MAX_PENDING {
            return Err("too many requests waiting");
        }
        if op.len() > dupin_js_runtime::MAX_OP_BYTES || args.len() > dupin_js_runtime::MAX_ARGS_BYTES {
            return Err("a request too large");
        }
        self.last = call;
        self.waiting.fetch_add(1, Ordering::AcqRel);
        let services = self.services.clone();
        let cancelled = self.cancelled.clone();
        let reply = self.reply(call);
        let request = HostRequest {
            run: self.token,
            call,
        };
        let started = self.workers.run(Box::new(move || {
            reply.send(&services.call(request, &op, &args, due.at(), &cancelled));
        }));
        if started.is_err() {
            self.reply(call)
                .send(br#"{"error":{"code":"unavailable","message":"Dupin couldn't start the request"}}"#);
        }
        Ok(())
    }

    fn reply(&self, call: u32) -> Reply {
        Reply {
            run: self.run,
            call,
            socket: self.socket.clone(),
            cancelled: self.cancelled.clone(),
            waiting: self.waiting.clone(),
            sent: self.sent.clone(),
        }
    }
}

type Job = Box<dyn FnOnce() + Send>;

/// Idle workers kept for the next requests.
const MAX_IDLE_WORKERS: usize = 4;

/// The threads host requests run on, kept between requests: starting a thread takes about as
/// long as answering a request, and a service may keep per-thread state that outlives one,
/// such as EventKit's store and the date formatters the calendar builds.
#[derive(Default)]
struct Workers {
    idle: Mutex<Vec<mpsc::Sender<Job>>>,
}

impl Workers {
    /// Hands `job` to an idle worker, or to a new one when every worker is busy.
    fn run(self: &Arc<Self>, job: Job) -> io::Result<()> {
        let mut job = job;
        loop {
            let idle = self
                .idle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop();
            let Some(worker) = idle else { break };
            match worker.send(job) {
                Ok(()) => return Ok(()),
                Err(mpsc::SendError(returned)) => job = returned,
            }
        }
        let workers = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("extension-host-request".into())
            .spawn(move || {
                let mut next = Some(job);
                while let Some(job) = next.take() {
                    job();
                    // Only the idle list holds the sender, so the worker ends with the host.
                    let (worker, jobs) = mpsc::channel::<Job>();
                    let Some(workers) = workers.upgrade() else { return };
                    {
                        let mut idle = workers
                            .idle
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if idle.len() >= MAX_IDLE_WORKERS {
                            return;
                        }
                        idle.push(worker);
                    }
                    drop(workers);
                    next = jobs.recv().ok();
                }
            })
            .map(drop)
    }
}

/// Where one host request's answer goes.
struct Reply {
    run: u64,
    call: u32,
    socket: Arc<Mutex<UnixStream>>,
    cancelled: Arc<AtomicBool>,
    waiting: Arc<AtomicUsize>,
    sent: Arc<Mutex<HashSet<u32>>>,
}

impl Reply {
    /// A helper that can't take the reply has died, which the run notices as its replies
    /// channel closing.
    fn send(self, json: &[u8]) {
        if !self.cancelled.load(Ordering::Relaxed) {
            // Noted before it is written, so any reply the guest can have taken is noted.
            self.sent
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(self.call);
            let _ = write(
                &self.socket,
                &ToHelper::Reply {
                    id: self.run,
                    call: self.call,
                },
                &[json],
            );
        }
        self.waiting.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Writes one message whole, so concurrent replies and the run's own messages never interleave.
fn write(socket: &Mutex<UnixStream>, message: &ToHelper, tail: &[&[u8]]) -> io::Result<()> {
    let mut socket = socket.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    wire::write(&mut *socket, message, tail)
}

impl Drop for Requests {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        // A helper may claim to have taken only replies it was sent.
        let answered: Vec<u32> = {
            let sent = self.sent.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            self.answered
                .iter()
                .copied()
                .filter(|call| sent.contains(call))
                .collect()
        };
        self.services.ended(self.token, &answered);
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

/// When a host request must be answered by, on the wall clock its services keep to.
#[derive(Clone, Copy)]
enum Due {
    At(Instant),
    /// This long after the request starts.
    Left(Duration),
}

impl Due {
    fn at(self) -> Instant {
        match self {
            Due::At(at) => at,
            Due::Left(left) => Instant::now() + left,
        }
    }
}

/// What a host measures its calls' deadlines on.
enum Clock {
    /// The wall clock: the launcher answers a person, who waits however busy the Mac is.
    Wall,
    /// The processor time its helpers spent on its calls, as a moment after `origin`.
    Processor { origin: Instant, spent: Mutex<Duration> },
}

impl Clock {
    fn now(&self) -> Instant {
        match self {
            Clock::Wall => Instant::now(),
            Clock::Processor { origin, spent } => {
                *origin + *spent.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            }
        }
    }
}

/// A call's deadline, if it has one yet, read on its host's clock while one helper works on it.
/// On a processor clock the moment moves with the helper's processor time, and what the helper
/// spent is added to the clock once the reading ends.
struct Meter<'a> {
    clock: &'a Clock,
    deadline: Option<Instant>,
    pid: u32,
    from: Instant,
    /// The helper's processor time when the reading began, and when it was last read; the last
    /// stands once the helper has exited.
    used: Duration,
    last: std::cell::Cell<Duration>,
}

impl<'a> Meter<'a> {
    fn start(clock: &'a Clock, helper: &Helper, deadline: Option<Instant>) -> Self {
        let pid = helper.child.id();
        let used = processor_time(pid).unwrap_or_default();
        Self {
            clock,
            deadline,
            pid,
            from: clock.now(),
            used,
            last: std::cell::Cell::new(used),
        }
    }

    fn now(&self) -> Instant {
        match self.clock {
            Clock::Wall => Instant::now(),
            Clock::Processor { .. } => self.from + self.read().saturating_sub(self.used),
        }
    }

    /// When a host request asked for now is due. Services keep to the wall clock: on it the
    /// call's deadline itself, and on a processor clock the time the call has left, from when
    /// the request starts on its worker, since waiting for one takes none of the guest's time.
    fn due(&self) -> Due {
        match (self.clock, self.deadline) {
            (Clock::Wall, Some(deadline)) => Due::At(deadline),
            _ => Due::Left(self.remaining()),
        }
    }

    /// Whether the deadline passed `grace` ago.
    fn passed(&self, grace: Duration) -> bool {
        self.deadline
            .is_some_and(|deadline| self.now() >= deadline + grace)
    }

    fn remaining(&self) -> Duration {
        self.deadline.map_or(Duration::ZERO, |deadline| {
            deadline.saturating_duration_since(self.now())
        })
    }

    /// How long ago the helper started at `started` on the wall clock, measured on this clock.
    fn since(&self, started: Instant) -> Duration {
        match self.clock {
            Clock::Wall => started.elapsed(),
            Clock::Processor { .. } => self.read(),
        }
    }

    fn read(&self) -> Duration {
        if let Some(time) = processor_time(self.pid) {
            self.last.set(time.max(self.last.get()));
        }
        self.last.get()
    }
}

impl Drop for Meter<'_> {
    fn drop(&mut self) {
        if let Clock::Processor { spent, .. } = self.clock {
            let used = self.read().saturating_sub(self.used);
            *spent.lock().unwrap_or_else(std::sync::PoisonError::into_inner) += used;
        }
    }
}

/// A moment in a helper's processor time, or on the wall clock where that can't be read.
#[derive(Clone, Copy)]
enum Since {
    Processor(Duration),
    Wall(Instant),
}

impl Since {
    fn now(pid: u32, now: Instant) -> Self {
        processor_time(pid).map_or(Self::Wall(now), Self::Processor)
    }

    fn elapsed(self, pid: u32, now: Instant) -> Duration {
        match self {
            // A helper whose time can no longer be read has exited.
            Self::Processor(since) => {
                processor_time(pid).map_or(Duration::MAX, |time| time.saturating_sub(since))
            }
            Self::Wall(since) => now.saturating_duration_since(since),
        }
    }
}

/// The processor time the helper has used, user and system, across all its threads.
#[cfg(target_os = "macos")]
pub(crate) fn processor_time(pid: u32) -> Option<Duration> {
    #[repr(C)]
    struct Timebase {
        numerator: u32,
        denominator: u32,
    }
    unsafe extern "C" {
        fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
    }
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: `usage` is large enough for the RUSAGE_INFO_V2 flavor the call writes.
    let status =
        unsafe { libc::proc_pid_rusage(pid as i32, libc::RUSAGE_INFO_V2, usage.as_mut_ptr().cast()) };
    if status != 0 {
        return None;
    }
    // SAFETY: a zero status means the call filled `usage`.
    let usage = unsafe { usage.assume_init() };
    let mut timebase = Timebase {
        numerator: 0,
        denominator: 0,
    };
    // SAFETY: `timebase` is the struct the call fills.
    if unsafe { mach_timebase_info(&mut timebase) } != 0 || timebase.denominator == 0 {
        return None;
    }
    // Mach absolute time units, which are nanoseconds only on Intel.
    let ticks = u128::from(usage.ri_user_time) + u128::from(usage.ri_system_time);
    let nanos = ticks * u128::from(timebase.numerator) / u128::from(timebase.denominator);
    Some(Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX)))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn processor_time(_pid: u32) -> Option<Duration> {
    None
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

/// Fails unless every link that `items` open, other than a web URL or a Dictionary word, came
/// from one of the call's own replies, so only a package that read an event can show it in
/// Calendar, and only that one.
fn issued<'a>(
    actions: impl IntoIterator<Item = &'a RowAction>,
    services: &dyn Services,
    package: &Compiled,
) -> Result<(), RunError> {
    let stray = actions.into_iter().any(|action| match &action.effect {
        RowEffect::OpenUrl { url } => !is_web_url(url) && !is_dictionary_url(url) && !services.opens(url),
        _ => false,
    });
    if !stray {
        return Ok(());
    }
    Err(RunError::Failed(
        Diagnostic::new(
            "invalid-url",
            "an `open` action names a calendar link that no `calendar.events` reply in this search gave",
        )
        .file(package.entry())
        .hint("Open the `link` of an event `calendar.events` returned while answering this query."),
    ))
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

fn catalogue_page(answer: (Answer, Vec<u8>), package: &Compiled) -> Result<CataloguePage, RunError> {
    match answer {
        (Answer::Catalogue { page }, _) => {
            page.validate().map_err(|problem| unshowable(problem, package))?;
            Ok(page)
        }
        _ => Err(mismatch()),
    }
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
        Work::View => "Make the view return sooner: Dupin stops it after 2 seconds, as it does a search.",
        Work::Action | Work::Event => {
            "Do less in one call: Dupin stops actions and event handlers at their deadline."
        }
        Work::Filter => {
            "Make the filter return sooner: it runs as the user types, and Dupin shows the source's own matches once it overruns."
        }
    };
    Diagnostic::new(
        "deadline",
        format!("{} didn't finish before its deadline", work.doing()),
    )
    .file(entry)
    .hint(hint)
}
