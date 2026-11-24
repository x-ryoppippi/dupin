//! JavaScript extensions on QuickJS-ng. Each call runs in a fresh runtime with a bounded heap,
//! stack, deadline and promise-job budget, and with no APIs beyond the language itself and the
//! host's `dupin` module: no files, network, processes or timers.
//!
//! Dupin itself only reads a package's [`Sources`]; compiling and running them, and erasing the
//! types of TypeScript modules first, is the extension host's work.

mod diagnostic;
mod environment;
mod host;
mod output;
mod package;
mod typescript;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use dupin_plugin_protocol::PluginItem;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::promise::PromiseState;
use rquickjs::{Context, Ctx, Exception, Function, Module, Object, Promise, Runtime, Value, WriteOptions};

pub use diagnostic::Diagnostic;
pub use environment::Environment;
pub use host::{Bridge, MAX_ARGS_BYTES, MAX_CALLS, MAX_OP_BYTES, MAX_PENDING, Reply, Unavailable};
pub use output::ActionEffect;
pub use package::{Sources, is_module, is_source_name, source_files};

const HEAP_BYTES: usize = 16 * 1024 * 1024;
const STACK_BYTES: usize = 512 * 1024;
/// Promise jobs one call may run before it is stopped.
const MAX_JOBS: usize = 10_000;
/// Items one `search` may return.
pub const MAX_ROWS: usize = 50;
/// One large bundle needs more than a call's heap while its parser runs.
pub const COMPILE_HEAP_BYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_ENTRY: &str = "main.js";
const HOST_MODULE: &str = "dupin";
/// The longest name of an action or event handler.
pub(crate) const MAX_HANDLER_BYTES: usize = 64;

/// When a call must stop. Checked between promise jobs and, through QuickJS's interrupt
/// handler, while JavaScript runs.
pub struct Control {
    pub deadline: Instant,
    pub is_cancelled: Box<dyn Fn() -> bool>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    /// The caller cancelled the call; nobody is waiting for an answer.
    Cancelled,
    Failed(Diagnostic),
}

impl From<Diagnostic> for RunError {
    fn from(diagnostic: Diagnostic) -> Self {
        Self::Failed(diagnostic)
    }
}

/// The QuickJS-ng release this build runs, which decides the bytecode format.
pub fn engine_version() -> &'static str {
    // SAFETY: QuickJS returns a pointer to a static, NUL-terminated string.
    unsafe { std::ffi::CStr::from_ptr(rquickjs::qjs::JS_GetVersion()) }
        .to_str()
        .unwrap_or("unknown")
}

/// A package's modules as bytecode. Only the extension host makes or loads one: QuickJS's parser
/// and bytecode loader are reachable from package source and bytes, so they never run in Dupin.
pub struct Package {
    entry: String,
    modules: Arc<HashMap<String, Box<[u8]>>>,
    environment: Option<Environment>,
}

impl std::fmt::Debug for Package {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Package")
            .field("entry", &self.entry)
            .finish_non_exhaustive()
    }
}

impl Package {
    /// Compiles every module of `sources` to bytecode, within the call limits and `control`,
    /// erasing the types of TypeScript modules first. QuickJS links a module's static imports
    /// as it compiles it, so this also catches imports that leave the package or name a missing
    /// file.
    pub fn compile(sources: Sources, control: Control) -> Result<Self, RunError> {
        let entry = sources.entry().to_owned();
        let modules = compile(sources, control)?;
        Ok(Self {
            entry,
            modules: Arc::new(modules),
            environment: None,
        })
    }

    /// Rebuilds a package from what [`Package::modules`] gave out, in a later call.
    ///
    /// # Safety
    ///
    /// QuickJS does not validate bytecode. Every module must be bytes that [`Package::compile`]
    /// wrote, unchanged, in a build whose [`engine_version`] is this one's.
    pub unsafe fn from_compiled(entry: String, modules: HashMap<String, Box<[u8]>>) -> Self {
        Self {
            entry,
            modules: Arc::new(modules),
            environment: None,
        }
    }

    /// The same package, whose calls see the clock and `Math.random` `environment` fixes, from
    /// before its modules load.
    pub fn with_environment(&self, environment: Option<Environment>) -> Self {
        Self {
            entry: self.entry.clone(),
            modules: self.modules.clone(),
            environment,
        }
    }

    pub fn entry(&self) -> &str {
        &self.entry
    }

    /// Each module's name and bytecode.
    pub fn modules(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.modules
            .iter()
            .map(|(name, bytecode)| (name.as_str(), &bytecode[..]))
    }

    /// Links and evaluates the entry module, and checks that it exports `search`.
    pub fn check(&self, control: Control) -> Result<(), RunError> {
        let mut bridge = Unavailable::default();
        let sandbox = Sandbox::new(self, control, &mut bridge, Work::Search)?;
        sandbox.run(|ctx| {
            sandbox.export_function(ctx, "search")?;
            Ok(())
        })
    }

    /// Calls `search(query, { limit })` with no host services and reads its answer.
    pub fn search(&self, query: &str, limit: usize, control: Control) -> Result<Vec<PluginItem>, RunError> {
        self.search_with(query, limit, control, &mut Unavailable::default())
    }

    /// Calls `search(query, { limit })`, carrying its host requests through `bridge`, and reads
    /// its answer. At most `limit` rows are kept.
    pub fn search_with(
        &self,
        query: &str,
        limit: usize,
        control: Control,
        bridge: &mut dyn Bridge,
    ) -> Result<Vec<PluginItem>, RunError> {
        let sandbox = Sandbox::new(self, control, bridge, Work::Search)?;
        sandbox.run(|ctx| {
            let search = sandbox.export_function(ctx, "search")?;
            let options = Object::new(ctx.clone())?;
            options.set("limit", limit.min(MAX_ROWS) as u32)?;
            let options = frozen(ctx, options)?;
            let answer: Value = search
                .call((query, options))
                .map_err(|error| sandbox.thrown(ctx, error, Stage::Call))?;
            let answer = match answer.as_promise() {
                Some(promise) => sandbox.settle(ctx, promise.clone(), Stage::Call)?,
                None => answer,
            };
            let mut reader = output::Reader::new(&self.entry, limit.min(MAX_ROWS));
            reader.rows(answer)
        })
    }

    /// Calls `actions[name](data, ctx)`, which runs because the user chose it, and reads the
    /// effect it returns. `data` is the JSON the item's action carried.
    pub fn action(
        &self,
        name: &str,
        data: &str,
        control: Control,
        bridge: &mut dyn Bridge,
    ) -> Result<Option<ActionEffect>, RunError> {
        let sandbox = Sandbox::new(self, control, bridge, Work::Action)?;
        sandbox.run(|ctx| {
            let handler = sandbox.handler(ctx, "actions", name)?;
            let data = ctx.json_parse(data)?;
            let context = frozen(ctx, Object::new(ctx.clone())?)?;
            let answer: Value = handler
                .call((data, context))
                .map_err(|error| sandbox.thrown(ctx, error, Stage::Call))?;
            let answer = match answer.as_promise() {
                Some(promise) => sandbox.settle(ctx, promise.clone(), Stage::Call)?,
                None => answer,
            };
            output::effect(&self.entry, &answer)
        })
    }

    /// Calls `events[name](event)` for something Dupin watches on the package's behalf, such as
    /// a clipboard change. What it returns is ignored.
    pub fn event(
        &self,
        name: &str,
        event: &str,
        control: Control,
        bridge: &mut dyn Bridge,
    ) -> Result<(), RunError> {
        let sandbox = Sandbox::new(self, control, bridge, Work::Event)?;
        sandbox.run(|ctx| {
            let handler = sandbox.handler(ctx, "events", name)?;
            let event = ctx.json_parse(event)?;
            let answer: Value = handler
                .call((frozen(ctx, event)?,))
                .map_err(|error| sandbox.thrown(ctx, error, Stage::Call))?;
            if let Some(promise) = answer.as_promise() {
                sandbox.settle(ctx, promise.clone(), Stage::Call)?;
            }
            Ok(())
        })
    }
}

/// `Object.freeze(value)`, for what the guest receives and must not change.
fn frozen<'js>(ctx: &Ctx<'js>, value: impl rquickjs::IntoJs<'js>) -> rquickjs::Result<Value<'js>> {
    let freeze: Function = ctx.globals().get::<_, Object>("Object")?.get("freeze")?;
    freeze.call((value,))
}

fn compile(sources: Sources, control: Control) -> Result<HashMap<String, Box<[u8]>>, RunError> {
    let entry = sources.entry().to_owned();
    let names: Vec<String> = sources.files().map(|(name, _)| name.to_owned()).collect();
    let mut modules = HashMap::with_capacity(names.len() + 1);
    for (name, source) in sources.files() {
        let source = if typescript::is_typescript(name) {
            typescript::erase(name, source)?
        } else {
            source.to_owned()
        };
        modules.insert(name.to_owned(), source);
    }
    modules.insert(HOST_MODULE.into(), host::SOURCE.into());
    let sources = modules;
    let compiler = Compiler {
        sources: Rc::new(sources),
        compiled: Rc::default(),
        problem: LinkProblem::default(),
    };
    let watchdog = Watchdog::new(control);
    let stopped = |watchdog: &Watchdog| match watchdog.stop.get() {
        Some(Stop::Cancelled) | None => RunError::Cancelled,
        Some(Stop::Deadline) => RunError::Failed(
            Diagnostic::new(
                "deadline",
                "compiling the package didn't finish before its deadline",
            )
            .file(&entry)
            .hint(
                "Bundle the extension into less JavaScript: Dupin stops compiling a package after 2 seconds.",
            ),
        ),
    };
    let runtime = Runtime::new().map_err(|error| internal(&error))?;
    runtime.set_memory_limit(COMPILE_HEAP_BYTES);
    runtime.set_max_stack_size(STACK_BYTES);
    let interrupt = watchdog.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || interrupt.tripped())));
    runtime.set_loader(compiler.clone(), compiler.clone());
    let context = Context::full(&runtime).map_err(|error| internal(&error))?;
    let compiled = context.with(|ctx| {
        for name in &names {
            if watchdog.tripped() {
                return Err(stopped(&watchdog));
            }
            if compiler.compiled.borrow().contains_key(name) {
                continue;
            }
            compiler.declare(&ctx, name).map_err(|error| {
                RunError::Failed(
                    compiler
                        .problem
                        .take()
                        .unwrap_or_else(|| syntax(&ctx, error, name)),
                )
            })?;
        }
        Ok(())
    });
    drop(context);
    // The parser never polls the interrupt handler, so a module that took too long to parse is
    // only caught here.
    if watchdog.tripped() {
        return Err(stopped(&watchdog));
    }
    compiled?;
    Ok(compiler.compiled.take())
}

#[derive(Clone)]
struct Compiler {
    sources: Rc<HashMap<String, String>>,
    compiled: Rc<RefCell<HashMap<String, Box<[u8]>>>>,
    problem: LinkProblem,
}

impl Compiler {
    fn declare<'js>(&self, ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Module<'js, Declared>> {
        let source = self
            .sources
            .get(name)
            .ok_or_else(|| rquickjs::Error::new_loading(name))?;
        let module = Module::declare(ctx.clone(), name, source.as_str())?;
        let bytecode = module.write(WriteOptions::default())?;
        self.compiled
            .borrow_mut()
            .insert(name.to_owned(), bytecode.into_boxed_slice());
        Ok(module)
    }
}

impl Resolver for Compiler {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let sources = self.sources.clone();
        resolve_reporting(&self.problem, &|name| sources.contains_key(name), base, name)
    }
}

impl Loader for Compiler {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        self.declare(ctx, name).map_err(|error| {
            let diagnostic = self.problem.take().unwrap_or_else(|| syntax(ctx, error, name));
            let message = diagnostic.to_string();
            self.problem.report(diagnostic);
            rquickjs::Error::new_loading_message(name, message)
        })
    }
}

fn resolve_reporting(
    problem: &LinkProblem,
    exists: &dyn Fn(&str) -> bool,
    base: &str,
    name: &str,
) -> rquickjs::Result<String> {
    package::resolve(exists, base, name).map_err(|diagnostic| {
        let message = diagnostic.to_string();
        problem.report(diagnostic);
        rquickjs::Error::new_resolving_message(base, name, message)
    })
}

fn internal(error: &rquickjs::Error) -> Diagnostic {
    Diagnostic::new("engine-error", format!("The JavaScript engine failed: {error}"))
}

fn syntax(ctx: &Ctx<'_>, error: rquickjs::Error, file: &str) -> Diagnostic {
    if !matches!(error, rquickjs::Error::Exception) {
        return internal(&error).file(file);
    }
    let value = ctx.catch();
    let thrown = describe_thrown(&value);
    if value.is_null() || thrown.message.contains("out of memory") {
        return Diagnostic::new(
            "memory-limit",
            format!(
                "compiling it used more than {} MiB of memory",
                COMPILE_HEAP_BYTES / 1024 / 1024
            ),
        )
        .file(file)
        .hint("Bundle and minify the extension, and leave out code it doesn't run.");
    }
    if thrown.message.contains("stack overflow") || thrown.message.contains("call stack size exceeded") {
        return Diagnostic::new("stack-limit", "nests too deeply to compile")
            .file(file)
            .hint("Flatten deeply nested expressions, arrays or objects.");
    }
    let mut diagnostic = Diagnostic::new("syntax-error", thrown.message)
        .file(file)
        .hint("Fix the syntax. Dupin runs JavaScript modules, and TypeScript once its types are erased; build anything else, such as JSX, with `dupin ext build`.");
    if let Some((_, line, column)) = thrown.location {
        diagnostic = diagnostic.at(line, column);
    }
    diagnostic
}

/// Why the interrupt handler stopped JavaScript.
#[derive(Clone, Copy)]
enum Stop {
    Cancelled,
    Deadline,
}

#[derive(Clone)]
struct Watchdog {
    deadline: Instant,
    is_cancelled: Rc<dyn Fn() -> bool>,
    stop: Rc<Cell<Option<Stop>>>,
}

impl Watchdog {
    fn new(control: Control) -> Self {
        Self {
            deadline: control.deadline,
            is_cancelled: Rc::from(control.is_cancelled),
            stop: Rc::default(),
        }
    }

    fn tripped(&self) -> bool {
        if self.stop.get().is_some() {
            return true;
        }
        let stop = if (self.is_cancelled)() {
            Stop::Cancelled
        } else if Instant::now() >= self.deadline {
            Stop::Deadline
        } else {
            return false;
        };
        self.stop.set(Some(stop));
        true
    }
}

/// The first failure a resolver or loader hit while QuickJS was linking modules, which QuickJS
/// would otherwise reduce to an error message without the file and import at fault.
#[derive(Clone, Default)]
struct LinkProblem(Rc<RefCell<Option<Diagnostic>>>);

impl LinkProblem {
    fn report(&self, diagnostic: Diagnostic) {
        self.0.borrow_mut().get_or_insert(diagnostic);
    }

    fn take(&self) -> Option<Diagnostic> {
        self.0.borrow_mut().take()
    }
}

struct PackageModules {
    modules: Arc<HashMap<String, Box<[u8]>>>,
    problem: LinkProblem,
}

impl Resolver for PackageModules {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let modules = self.modules.clone();
        resolve_reporting(&self.problem, &|name| modules.contains_key(name), base, name)
    }
}

impl Loader for PackageModules {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        let bytecode = self
            .modules
            .get(name)
            .ok_or_else(|| rquickjs::Error::new_loading(name))?;
        // SAFETY: `compile` wrote these bytes from a module it had just parsed, and only
        // `Package::compile` or `Package::from_compiled`, whose contract is the same, fill the
        // map.
        unsafe { Module::load(ctx.clone(), bytecode) }
    }
}

/// What a sandbox runs, for the diagnostics that name it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Work {
    Search,
    Action,
    Event,
}

impl Work {
    fn name(self) -> &'static str {
        match self {
            Work::Search => "search",
            Work::Action => "the action",
            Work::Event => "the event handler",
        }
    }
}

/// Whether a failure came from loading the module or from the call into it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Module,
    Call,
}

struct Sandbox<'a> {
    package: &'a Package,
    watchdog: Watchdog,
    problem: LinkProblem,
    work: Work,
    calls: host::Shared,
    bridge: RefCell<&'a mut dyn Bridge>,
    // Dropped in order: the context's values before the runtime that owns them.
    context: Context,
    _runtime: Runtime,
}

impl<'a> Sandbox<'a> {
    fn new(
        package: &'a Package,
        control: Control,
        bridge: &'a mut dyn Bridge,
        work: Work,
    ) -> Result<Self, RunError> {
        let runtime = Runtime::new().map_err(|error| internal(&error))?;
        runtime.set_memory_limit(HEAP_BYTES);
        runtime.set_max_stack_size(STACK_BYTES);
        let watchdog = Watchdog::new(control);
        let interrupt = watchdog.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || interrupt.tripped())));
        let problem = LinkProblem::default();
        runtime.set_loader(
            PackageModules {
                modules: package.modules.clone(),
                problem: problem.clone(),
            },
            PackageModules {
                modules: package.modules.clone(),
                problem: problem.clone(),
            },
        );
        let context = Context::full(&runtime).map_err(|_| memory_limit())?;
        let calls = host::Shared::default();
        context
            .with(|ctx| {
                host::install(&ctx, &calls)?;
                match &package.environment {
                    Some(environment) => environment::install(&ctx, environment),
                    None => Ok(()),
                }
            })
            .map_err(|_| memory_limit())?;
        Ok(Self {
            package,
            watchdog,
            problem,
            work,
            calls,
            bridge: RefCell::new(bridge),
            context,
            _runtime: runtime,
        })
    }

    fn run<T>(&self, call: impl for<'js> FnOnce(&Ctx<'js>) -> Result<T, Failure>) -> Result<T, RunError> {
        if self.watchdog.tripped() {
            return Err(self.stopped());
        }
        let result = self.context.with(|ctx| {
            let result = call(&ctx).map_err(|failure| self.conclude(&ctx, failure));
            self.calls.borrow_mut().clear();
            result
        });
        // A stop wins over whatever the guest made of it, such as a caught "interrupted" error.
        if self.watchdog.stop.get().is_some() {
            return Err(self.stopped());
        }
        result
    }

    fn conclude(&self, ctx: &Ctx<'_>, failure: Failure) -> RunError {
        match failure {
            Failure::Report(diagnostic) => RunError::Failed(diagnostic),
            Failure::Stopped => self.stopped(),
            Failure::Js(error) => self.conclude(ctx, self.thrown(ctx, error, Stage::Call)),
        }
    }

    fn stopped(&self) -> RunError {
        match self.watchdog.stop.get() {
            Some(Stop::Cancelled) | None => RunError::Cancelled,
            Some(Stop::Deadline) => RunError::Failed(
                Diagnostic::new(
                    "deadline",
                    format!("{} didn't finish before its deadline", self.work.name()),
                )
                .file(&self.package.entry)
                .hint(match self.work {
                    Work::Search => "Make search return sooner: it runs as the user types, and Dupin stops it after 2 seconds.",
                    Work::Action | Work::Event => "Do less in one call: Dupin stops actions and event handlers at their deadline.",
                }),
            ),
        }
    }

    fn namespace<'js>(&self, ctx: &Ctx<'js>) -> Result<Object<'js>, Failure> {
        let loading = Module::import(ctx, self.package.entry.as_str())
            .map_err(|error| self.thrown(ctx, error, Stage::Module))?;
        let namespace = self.settle(ctx, loading, Stage::Module)?;
        namespace
            .into_object()
            .ok_or(Failure::Js(rquickjs::Error::Unknown))
    }

    fn export_function<'js>(&self, ctx: &Ctx<'js>, name: &str) -> Result<Function<'js>, Failure> {
        let value: Value = self.namespace(ctx)?.get(name)?;
        value.into_function().ok_or_else(|| {
            Failure::Report(
                Diagnostic::new("missing-search", format!("doesn't export a `{name}` function"))
                    .file(&self.package.entry)
                    .hint("Add `export function search(query) { return []; }` to the entry module."),
            )
        })
    }

    /// `map[name]` from the entry module's exported `map`, such as `actions.save`.
    fn handler<'js>(&self, ctx: &Ctx<'js>, map: &str, name: &str) -> Result<Function<'js>, Failure> {
        let missing = || {
            Failure::Report(
                Diagnostic::new(
                    "missing-handler",
                    format!("doesn't export a `{map}.{name}` function"),
                )
                .file(&self.package.entry)
                .field(format!("{map}.{name}"))
                .hint(format!(
                    "Export `const {map} = {{ {name}(…) {{ … }} }}` from the entry module."
                )),
            )
        };
        if name.is_empty() || name.len() > MAX_HANDLER_BYTES {
            return Err(missing());
        }
        let handlers: Value = self.namespace(ctx)?.get(map)?;
        let Some(handlers) = handlers.into_object().filter(|value| !value.is_function()) else {
            return Err(missing());
        };
        // Only the map's own properties, so `toString` or `__proto__` can't name a handler.
        let has_own: Function = ctx.globals().get::<_, Object>("Object")?.get("hasOwn")?;
        if !has_own.call::<_, bool>((handlers.clone(), name))? {
            return Err(missing());
        }
        let handler: Value = handlers.get(name)?;
        handler.into_function().ok_or_else(missing)
    }

    /// Sends the requests the guest queued since the last turn of the promise loop.
    fn flush(&self, ctx: &Ctx<'_>) -> Result<(), Failure> {
        let outbox = self.calls.borrow_mut().take_outbox();
        let mut bridge = self.bridge.borrow_mut();
        for (call, op, args) in outbox {
            if let Err(diagnostic) = bridge.send(call, &op, &args) {
                drop(bridge);
                let reply = Reply {
                    call,
                    json: host::failure("unavailable", &diagnostic.message),
                };
                self.deliver(ctx, reply)?;
                bridge = self.bridge.borrow_mut();
            }
        }
        Ok(())
    }

    fn deliver(&self, ctx: &Ctx<'_>, reply: Reply) -> Result<(), Failure> {
        let Some(resolve) = self.calls.borrow_mut().take(reply.call) else {
            return Ok(());
        };
        let resolve = resolve.restore(ctx)?;
        resolve.call::<_, ()>((reply.json,))?;
        Ok(())
    }

    /// Runs promise jobs until `promise` settles, the job budget is spent or nothing is left to
    /// run.
    fn settle<'js>(
        &self,
        ctx: &Ctx<'js>,
        promise: Promise<'js>,
        stage: Stage,
    ) -> Result<Value<'js>, Failure> {
        let what = match stage {
            Stage::Module => "module",
            Stage::Call => self.work.name(),
        };
        let mut jobs = 0;
        loop {
            match promise.state() {
                PromiseState::Resolved => {
                    return promise.result::<Value>().expect("settled").map_err(Failure::Js);
                }
                PromiseState::Rejected => {
                    let error = promise.result::<Value>().expect("settled").expect_err("rejected");
                    return Err(self.thrown(ctx, error, stage));
                }
                PromiseState::Pending => {}
            }
            if self.watchdog.tripped() {
                return Err(Failure::Stopped);
            }
            self.flush(ctx)?;
            if jobs == MAX_JOBS {
                return Err(Failure::Report(
                    Diagnostic::new(
                        "job-limit",
                        format!("{what} ran more than {MAX_JOBS} promise jobs without settling"),
                    )
                    .file(&self.package.entry)
                    .hint("Await fewer promises, or compute the answer without chaining promises in a loop."),
                ));
            }
            if ctx.execute_pending_job() {
                jobs += 1;
                continue;
            }
            if self.calls.borrow().waiting() {
                let reply = self.bridge.borrow_mut().receive(&|| self.watchdog.tripped());
                match reply {
                    Some(reply) => self.deliver(ctx, reply)?,
                    None => return Err(Failure::Stopped),
                }
                continue;
            }
            return Err(Failure::Report(
                Diagnostic::new(
                    "unsettled",
                    format!("{what} returned a promise that never settles"),
                )
                .file(&self.package.entry)
                .hint(
                    "Resolve the promise; extensions have no timers to wait on, only Dupin's own services.",
                ),
            ));
        }
    }

    /// Turns a JavaScript exception into a failure, recognizing the limits QuickJS reports as
    /// exceptions.
    fn thrown(&self, ctx: &Ctx<'_>, error: rquickjs::Error, stage: Stage) -> Failure {
        if self.watchdog.stop.get().is_some() {
            return Failure::Stopped;
        }
        if let Some(problem) = self.problem.take() {
            return Failure::Report(problem);
        }
        if !matches!(error, rquickjs::Error::Exception) {
            return match error {
                rquickjs::Error::Allocation => Failure::Report(memory_limit().file(&self.package.entry)),
                error => Failure::Report(internal(&error).file(&self.package.entry)),
            };
        }
        let value = ctx.catch();
        let thrown = describe_thrown(&value);
        let entry = &self.package.entry;
        // QuickJS throws `null` when it has no memory left to build the "out of memory" error.
        if value.is_null() || thrown.message.contains("out of memory") {
            return Failure::Report(memory_limit().file(entry));
        }
        if thrown.message.contains("stack overflow") || thrown.message.contains("call stack size exceeded") {
            return Failure::Report(
                Diagnostic::new(
                    "stack-limit",
                    format!(
                        "{} used more than {} KiB of stack",
                        self.work.name(),
                        STACK_BYTES / 1024
                    ),
                )
                .file(entry)
                .hint("Replace deep recursion with a loop."),
            );
        }
        let (code, message, hint) = match (stage, self.work) {
            (Stage::Module, _) => (
                "module-threw",
                format!("loading the module threw {}", thrown.message),
                "Fix the error in the module's top-level code.",
            ),
            (Stage::Call, Work::Search) => (
                "search-threw",
                format!("search threw {}", thrown.message),
                "Catch the error inside search, or return an item that explains the problem.",
            ),
            (Stage::Call, Work::Action) => (
                "action-threw",
                format!("the action threw {}", thrown.message),
                "Catch the error inside the action, or return `{ type: \"toast\", text }` to explain it.",
            ),
            (Stage::Call, Work::Event) => (
                "event-threw",
                format!("the event handler threw {}", thrown.message),
                "Catch the error inside the handler.",
            ),
        };
        let mut diagnostic = Diagnostic::new(code, message).hint(hint);
        diagnostic = match thrown.location {
            Some((file, line, column)) => diagnostic.file(file).at(line, column),
            None => diagnostic.file(entry),
        };
        Failure::Report(diagnostic)
    }
}

fn memory_limit() -> Diagnostic {
    Diagnostic::new(
        "memory-limit",
        format!("search used more than {} MiB of memory", HEAP_BYTES / 1024 / 1024),
    )
    .hint("Build smaller strings and arrays, and return only the items Dupin shows.")
}

/// Why a call inside a sandbox failed, before the sandbox decides what to report.
pub(crate) enum Failure {
    Js(rquickjs::Error),
    Report(Diagnostic),
    /// The watchdog stopped JavaScript; the sandbox reports why.
    Stopped,
}

impl From<rquickjs::Error> for Failure {
    fn from(error: rquickjs::Error) -> Self {
        Self::Js(error)
    }
}

impl From<Diagnostic> for Failure {
    fn from(diagnostic: Diagnostic) -> Self {
        Self::Report(diagnostic)
    }
}

struct Thrown {
    message: String,
    location: Option<(String, u32, u32)>,
}

fn describe_thrown(value: &Value<'_>) -> Thrown {
    if let Some(exception) = value.as_exception() {
        return describe_exception(exception);
    }
    let message = if let Some(text) = value.as_string().and_then(|text| text.to_string().ok()) {
        format!("{text:?}")
    } else {
        output::describe(value).to_owned()
    };
    Thrown {
        message,
        location: None,
    }
}

fn describe_exception(exception: &Exception<'_>) -> Thrown {
    let name: Option<String> = exception.get("name").ok();
    let message = exception.message().unwrap_or_default();
    let message = match name {
        Some(name) if !name.is_empty() => format!("{name}: {message}"),
        _ => message,
    };
    Thrown {
        message,
        location: exception.stack().as_deref().and_then(location),
    }
}

/// The innermost `file:line:column` in a QuickJS stack, such as `    at search (main.js:3:9)`.
fn location(stack: &str) -> Option<(String, u32, u32)> {
    stack.lines().find_map(|line| {
        let frame = line.trim().strip_prefix("at ")?;
        let place = match frame.rfind('(') {
            Some(open) => frame[open + 1..].strip_suffix(')')?,
            None => frame,
        };
        let mut parts = place.rsplitn(3, ':');
        let column = parts.next()?.parse().ok()?;
        let line = parts.next()?.parse().ok()?;
        let file = parts.next()?;
        (!file.is_empty() && file != "native").then(|| (file.to_owned(), line, column))
    })
}

#[cfg(test)]
mod tests;
