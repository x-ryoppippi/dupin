//! JavaScript extensions on QuickJS-ng. Each call runs in a fresh runtime with a bounded heap,
//! stack, deadline and promise-job budget, and with no APIs beyond the language itself and the
//! host's `dupin` module: no files, network, processes or timers.
//!
//! A call's runtime is an [`Engine`], which the caller may make before the call and drop after
//! answering, so that neither is part of the call's latency.
//!
//! Dupin itself only reads a package's [`Sources`]; compiling and running them, and erasing the
//! types of TypeScript modules first, is the extension host's work.

mod diagnostic;
mod environment;
mod host;
mod output;
mod package;
pub mod row;
mod typescript;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::promise::PromiseState;
use rquickjs::{Context, Ctx, Exception, Function, Module, Object, Promise, Runtime, Value, WriteOptions};

pub use diagnostic::Diagnostic;
pub use environment::Environment;
pub use host::{Bridge, MAX_ARGS_BYTES, MAX_CALLS, MAX_OP_BYTES, MAX_PENDING, Reply, Unavailable};
pub use output::ActionEffect;
pub use package::{Sources, is_module, is_source_name, source_files};
pub use row::{CataloguePage, Reason, Row};

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

/// When a call must stop. Checked between promise jobs and, through QuickJS's interrupt
/// handler, while JavaScript runs.
pub struct Control {
    pub deadline: Instant,
    pub is_cancelled: Box<dyn Fn() -> bool>,
}

/// A QuickJS runtime and a context with the language's standard library and the host's `dupin`
/// module, for one call.
///
/// Making them is most of what a call costs before any guest code runs, and freeing them most of
/// what it costs after, so the extension host makes the next call's engine while it waits and
/// drops each after answering. Only the host's own module runs in an engine before the call that
/// takes it, and its guest's code is the only guest code that ever runs there. A second call
/// refuses it, so no guest state reaches another call.
pub struct Engine {
    calls: host::Shared,
    invalidation: Rc<host::Invalidation>,
    // Dropped in order: the context's values before the runtime that owns them.
    context: Context,
    runtime: Runtime,
    taken: Cell<bool>,
}

impl Engine {
    pub fn new() -> Result<Self, RunError> {
        let bytecode = host::bytecode().map_err(|error| internal(&error))?;
        let runtime = Runtime::new().map_err(|error| internal(&error))?;
        runtime.set_memory_limit(HEAP_BYTES);
        runtime.set_max_stack_size(STACK_BYTES);
        let context = Context::full(&runtime).map_err(|_| memory_limit())?;
        let calls = host::Shared::default();
        let invalidation = Rc::<host::Invalidation>::default();
        context
            .with(|ctx| {
                host::install(&ctx, &calls, &invalidation)?;
                // SAFETY: `host::bytecode` wrote these bytes in this process from the host's own
                // module source.
                let (_, evaluated) = unsafe { Module::load(ctx.clone(), bytecode) }?.eval()?;
                evaluated.finish::<()>()
            })
            .map_err(|error| internal(&error))?;
        Ok(Self {
            calls,
            invalidation,
            context,
            runtime,
            taken: Cell::new(false),
        })
    }
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("taken", &self.taken.get())
            .finish_non_exhaustive()
    }
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

    /// Links and evaluates the entry module in `engine`, and reports which of `search` and
    /// `items` it exports. A package must export at least one.
    pub fn check(&self, engine: &Engine, control: Control) -> Result<Exports, RunError> {
        let mut bridge = Unavailable::default();
        let sandbox = Sandbox::new(self, engine, control, &mut bridge, Work::Search)?;
        sandbox.run(|ctx| {
            let namespace = sandbox.namespace(ctx)?;
            let exports = Exports {
                search: namespace.get::<_, Value>("search")?.is_function(),
                items: namespace.get::<_, Value>("items")?.is_function(),
            };
            if !exports.search && !exports.items {
                return Err(missing_export(&self.entry).into());
            }
            Ok(exports)
        })
    }

    /// Calls `search(query, { limit })` in an engine of its own, with no host services, and reads
    /// its answer.
    pub fn search(&self, query: &str, limit: usize, control: Control) -> Result<Vec<Row>, RunError> {
        self.search_with(
            &Engine::new()?,
            query,
            limit,
            control,
            &mut Unavailable::default(),
        )
    }

    /// Calls `search(query, { limit })` in `engine`, carrying its host requests through
    /// `bridge`, and reads its answer. At most `limit` rows are kept.
    pub fn search_with(
        &self,
        engine: &Engine,
        query: &str,
        limit: usize,
        control: Control,
        bridge: &mut dyn Bridge,
    ) -> Result<Vec<Row>, RunError> {
        let sandbox = Sandbox::new(self, engine, control, bridge, Work::Search)?;
        sandbox.run(|ctx| {
            let search = sandbox.function(ctx, "search")?;
            let options = Object::new(ctx.clone())?;
            options.set("limit", limit.min(MAX_ROWS) as u32)?;
            let options = frozen(ctx, options)?;
            let answer = sandbox.call(ctx, search, (query, options))?;
            output::Reader::search(&self.entry, limit.min(MAX_ROWS)).rows(answer)
        })
    }

    /// Calls `items({ cursor, reason, preferences })` in `engine`, carrying its host requests
    /// through `bridge`, and reads the part of the catalogue it answers with. The catalogue loads
    /// as the user types, before any row is chosen, so Dupin answers its requests as it does a
    /// search's. `preferences` is a JSON object.
    pub fn items(
        &self,
        engine: &Engine,
        cursor: Option<&str>,
        reason: Reason,
        preferences: &str,
        control: Control,
        bridge: &mut dyn Bridge,
    ) -> Result<CataloguePage, RunError> {
        let sandbox = Sandbox::new(self, engine, control, bridge, Work::Items)?;
        sandbox.run(|ctx| {
            let items = sandbox.function(ctx, "items")?;
            let context = Object::new(ctx.clone())?;
            match cursor {
                Some(cursor) => context.set("cursor", cursor)?,
                None => context.set("cursor", rquickjs::Null)?,
            }
            context.set("reason", reason.as_str())?;
            context.set("preferences", frozen(ctx, ctx.json_parse(preferences)?)?)?;
            let context = frozen(ctx, context)?;
            let answer = sandbox.call(ctx, items, (context,))?;
            output::Reader::items(&self.entry).page(answer)
        })
    }

    /// Calls `actions[name](data, ctx)` in `engine`, which runs because the user chose it, and
    /// reads the effect it returns and whether it invalidated the catalogue. `data` is the JSON
    /// the item's action carried, and `preferences` the JSON object `ctx.preferences` holds.
    pub fn action(
        &self,
        engine: &Engine,
        name: &str,
        data: &str,
        preferences: &str,
        control: Control,
        bridge: &mut dyn Bridge,
    ) -> Result<Acted, RunError> {
        let sandbox = Sandbox::new(self, engine, control, bridge, Work::Action)?;
        sandbox.run(|ctx| {
            let handler = sandbox.handler(ctx, "actions", name)?;
            let data = ctx.json_parse(data)?;
            let context = Object::new(ctx.clone())?;
            context.set("preferences", frozen(ctx, ctx.json_parse(preferences)?)?)?;
            let context = frozen(ctx, context)?;
            let answer = sandbox.call(ctx, handler, (data, context))?;
            Ok(Acted {
                invalidated: sandbox.invalidation.called.get(),
                effect: output::effect(&self.entry, &answer)?,
            })
        })
    }

    /// Calls `events[name](event, ctx)` in `engine` for something Dupin watches on the
    /// package's behalf, such as a clipboard change, and reports whether it invalidated the
    /// catalogue. What it returns is ignored.
    pub fn event(
        &self,
        engine: &Engine,
        name: &str,
        event: &str,
        preferences: &str,
        control: Control,
        bridge: &mut dyn Bridge,
    ) -> Result<Handled, RunError> {
        let sandbox = Sandbox::new(self, engine, control, bridge, Work::Event)?;
        sandbox.run(|ctx| {
            let handler = sandbox.handler(ctx, "events", name)?;
            let event = ctx.json_parse(event)?;
            let context = Object::new(ctx.clone())?;
            context.set("preferences", frozen(ctx, ctx.json_parse(preferences)?)?)?;
            let context = frozen(ctx, context)?;
            sandbox.call(ctx, handler, (frozen(ctx, event)?, context))?;
            Ok(Handled {
                invalidated: sandbox.invalidation.called.get(),
            })
        })
    }
}

/// Which of the exports Dupin calls a package has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Exports {
    pub search: bool,
    pub items: bool,
}

/// What an action handler did that Dupin acts on.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Acted {
    /// It called `items.invalidate()`.
    pub invalidated: bool,
    pub effect: Option<ActionEffect>,
}

/// What an event handler did that Dupin acts on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Handled {
    /// It called `items.invalidate()`.
    pub invalidated: bool,
}

fn missing_export(entry: &str) -> Diagnostic {
    Diagnostic::new("missing-search", "doesn't export a `search` or `items` function")
        .file(entry)
        .hint("Add `export function search(query) { return []; }` or `export function items() { return []; }` to the entry module.")
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
        // Every call that imports `dupin` loads its bytecode, and nothing reads its source back.
        let bytecode = module.write(WriteOptions {
            strip_source: name == HOST_MODULE,
            ..WriteOptions::default()
        })?;
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
    Items,
    Action,
    Event,
}

impl Work {
    fn name(self) -> &'static str {
        match self {
            Work::Search => "search",
            Work::Items => "items",
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
    invalidation: Rc<host::Invalidation>,
    watchdog: Watchdog,
    problem: LinkProblem,
    work: Work,
    calls: host::Shared,
    bridge: RefCell<&'a mut dyn Bridge>,
    context: &'a Context,
}

impl<'a> Sandbox<'a> {
    fn new(
        package: &'a Package,
        engine: &'a Engine,
        control: Control,
        bridge: &'a mut dyn Bridge,
        work: Work,
    ) -> Result<Self, RunError> {
        if engine.taken.replace(true) {
            return Err(RunError::Failed(Diagnostic::new(
                "engine-error",
                "an engine runs one call, and this one already ran",
            )));
        }
        let runtime = &engine.runtime;
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
        let context = &engine.context;
        engine
            .invalidation
            .allowed
            .set(matches!(work, Work::Action | Work::Event));
        if let Some(environment) = &package.environment {
            context
                .with(|ctx| environment::install(&ctx, environment))
                .map_err(|_| memory_limit())?;
        }
        Ok(Self {
            package,
            invalidation: engine.invalidation.clone(),
            watchdog,
            problem,
            work,
            calls: engine.calls.clone(),
            bridge: RefCell::new(bridge),
            context,
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
                    Work::Items => {
                        "Make items return sooner: Dupin stops loading a catalogue 2 seconds after it asks, counting every part."
                    }
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

    /// The entry module's exported function `name`.
    fn function<'js>(&self, ctx: &Ctx<'js>, name: &str) -> Result<Function<'js>, Failure> {
        let entry = &self.package.entry;
        let namespace = self.namespace(ctx)?;
        let function: Value = namespace.get(name)?;
        function.into_function().ok_or_else(|| {
            Failure::Report(match name {
                "items" => Diagnostic::new("missing-items", "doesn't export an `items` function")
                    .file(entry)
                    .hint("Add `export function items() { return []; }` to the entry module."),
                _ => Diagnostic::new("missing-search", "doesn't export a `search` function")
                    .file(entry)
                    .hint("Add `export function search(query) { return []; }` to the entry module."),
            })
        })
    }

    /// Calls `function` and waits for the promise it may return.
    fn call<'js>(
        &self,
        ctx: &Ctx<'js>,
        function: Function<'js>,
        arguments: impl rquickjs::function::IntoArgs<'js>,
    ) -> Result<Value<'js>, Failure> {
        let answer: Value = function
            .call(arguments)
            .map_err(|error| self.thrown(ctx, error, Stage::Call))?;
        match answer.as_promise() {
            Some(promise) => self.settle(ctx, promise.clone(), Stage::Call),
            None => Ok(answer),
        }
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
        if name.is_empty() || name.len() > row::MAX_HANDLER_BYTES {
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
            (Stage::Call, Work::Items) => (
                "items-threw",
                format!("items threw {}", thrown.message),
                "Catch the error inside items, or return an item that explains the problem.",
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
