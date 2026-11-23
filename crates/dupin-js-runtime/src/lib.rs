//! JavaScript extensions on QuickJS-ng. Each call runs in a fresh runtime with a bounded heap,
//! stack, deadline and promise-job budget, and with no APIs beyond the language itself and the
//! host's `dupin` module: no files, network, processes or timers.

mod diagnostic;
mod output;
mod package;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use dupin_plugin_protocol::PluginItem;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::promise::PromiseState;
use rquickjs::{Context, Ctx, Exception, Function, Module, Object, Promise, Runtime, Value, WriteOptions};

pub use diagnostic::Diagnostic;
pub use package::source_files;

const HEAP_BYTES: usize = 16 * 1024 * 1024;
const STACK_BYTES: usize = 512 * 1024;
/// Promise jobs one call may run before it is stopped.
const MAX_JOBS: usize = 10_000;
/// Items one `search` may return.
pub const MAX_ROWS: usize = 50;
/// Compiling is the host's work on files it has already bounded, but one large bundle needs more
/// than a call's heap while its parser runs.
const COMPILE_HEAP_BYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_ENTRY: &str = "main.js";
const HOST_MODULE: &str = "dupin";
const HOST_SOURCE: &str = "export const apiVersion = 1;\n";

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

/// A package's modules, compiled once when it is loaded. A changed file means a new package.
pub struct Package {
    entry: String,
    modules: Arc<HashMap<String, Box<[u8]>>>,
}

impl std::fmt::Debug for Package {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Package")
            .field("entry", &self.entry)
            .finish_non_exhaustive()
    }
}

impl Package {
    /// Reads and compiles every JavaScript file in `root`. `entry` is the module that exports
    /// `search`, relative to `root`.
    pub fn load(root: &Path, entry: &str) -> Result<Self, Diagnostic> {
        let entry = entry.strip_prefix("./").unwrap_or(entry).to_owned();
        let files = source_files(root)?;
        if !files.iter().any(|(name, _)| *name == entry) {
            return Err(Diagnostic::new("missing-entry", "is not a JavaScript file in the package folder")
                .file(&entry)
                .hint(format!(
                    "Create `{DEFAULT_ENTRY}` exporting `search`, or point `dupin.main` in package.json at the module that does."
                )));
        }
        let sources = package::read_sources(&files)?;
        let modules = compile(sources)?;
        Ok(Self {
            entry,
            modules: Arc::new(modules),
        })
    }

    /// Links and evaluates the entry module, and checks that it exports `search`.
    pub fn check(&self, control: Control) -> Result<(), RunError> {
        let sandbox = Sandbox::new(self, control)?;
        sandbox.run(|ctx| {
            sandbox.search_function(ctx)?;
            Ok(())
        })
    }

    /// Calls `search(query, { limit })` and reads its answer. At most `limit` rows are kept.
    pub fn search(&self, query: &str, limit: usize, control: Control) -> Result<Vec<PluginItem>, RunError> {
        let sandbox = Sandbox::new(self, control)?;
        sandbox.run(|ctx| {
            let search = sandbox.search_function(ctx)?;
            let options = Object::new(ctx.clone())?;
            options.set("limit", limit.min(MAX_ROWS) as u32)?;
            let freeze: Function = ctx.globals().get::<_, Object>("Object")?.get("freeze")?;
            let options: Value = freeze.call((options,))?;
            let answer: Value = search
                .call((query, options))
                .map_err(|error| sandbox.thrown(ctx, error, "search-threw"))?;
            let answer = match answer.as_promise() {
                Some(promise) => sandbox.settle(ctx, promise.clone(), "search")?,
                None => answer,
            };
            let mut reader = output::Reader::new(&self.entry, limit.min(MAX_ROWS));
            reader.rows(answer)
        })
    }
}

/// Compiles every module to bytecode. QuickJS links a module's static imports as it compiles
/// it, so this also catches imports that leave the package or name a missing file.
fn compile(sources: Vec<(String, String)>) -> Result<HashMap<String, Box<[u8]>>, Diagnostic> {
    let names: Vec<String> = sources.iter().map(|(name, _)| name.clone()).collect();
    let mut sources: HashMap<String, String> = sources.into_iter().collect();
    sources.insert(HOST_MODULE.into(), HOST_SOURCE.into());
    let compiler = Compiler {
        sources: Rc::new(sources),
        compiled: Rc::default(),
        problem: LinkProblem::default(),
    };
    let runtime = Runtime::new().map_err(|error| internal(&error))?;
    runtime.set_memory_limit(COMPILE_HEAP_BYTES);
    runtime.set_max_stack_size(STACK_BYTES);
    runtime.set_loader(compiler.clone(), compiler.clone());
    let context = Context::full(&runtime).map_err(|error| internal(&error))?;
    context.with(|ctx| {
        for name in &names {
            if compiler.compiled.borrow().contains_key(name) {
                continue;
            }
            compiler.declare(&ctx, name).map_err(|error| {
                compiler
                    .problem
                    .take()
                    .unwrap_or_else(|| syntax(&ctx, error, name))
            })?;
        }
        Ok::<_, Diagnostic>(())
    })?;
    drop(context);
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
    let thrown = describe_thrown(&ctx.catch());
    let mut diagnostic = Diagnostic::new("syntax-error", thrown.message)
        .file(file)
        .hint("Fix the syntax. Dupin runs plain JavaScript modules; build TypeScript or JSX first.");
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
        // SAFETY: `compile` wrote these bytes with this same engine, in this process, from a
        // module it had just parsed; nothing else can put bytes in the map.
        unsafe { Module::load(ctx.clone(), bytecode) }
    }
}

struct Sandbox<'a> {
    package: &'a Package,
    watchdog: Watchdog,
    problem: LinkProblem,
    // Dropped in order: the context's values before the runtime that owns them.
    context: Context,
    _runtime: Runtime,
}

impl<'a> Sandbox<'a> {
    fn new(package: &'a Package, control: Control) -> Result<Self, RunError> {
        let runtime = Runtime::new().map_err(|error| internal(&error))?;
        runtime.set_memory_limit(HEAP_BYTES);
        runtime.set_max_stack_size(STACK_BYTES);
        let watchdog = Watchdog {
            deadline: control.deadline,
            is_cancelled: Rc::from(control.is_cancelled),
            stop: Rc::default(),
        };
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
        Ok(Self {
            package,
            watchdog,
            problem,
            context,
            _runtime: runtime,
        })
    }

    fn run<T>(&self, call: impl for<'js> FnOnce(&Ctx<'js>) -> Result<T, Failure>) -> Result<T, RunError> {
        if self.watchdog.tripped() {
            return Err(self.stopped());
        }
        let result = self
            .context
            .with(|ctx| call(&ctx).map_err(|failure| self.conclude(&ctx, failure)));
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
            Failure::Js(error) => self.conclude(ctx, self.thrown(ctx, error, "search-threw")),
        }
    }

    fn stopped(&self) -> RunError {
        match self.watchdog.stop.get() {
            Some(Stop::Cancelled) | None => RunError::Cancelled,
            Some(Stop::Deadline) => RunError::Failed(
                Diagnostic::new("deadline", "search didn't finish before its deadline")
                    .file(&self.package.entry)
                    .hint("Make search return sooner: it runs as the user types, and Dupin stops it after 2 seconds."),
            ),
        }
    }

    fn search_function<'js>(&self, ctx: &Ctx<'js>) -> Result<Function<'js>, Failure> {
        let entry = &self.package.entry;
        let loading =
            Module::import(ctx, entry.as_str()).map_err(|error| self.thrown(ctx, error, "module-threw"))?;
        let namespace = self.settle(ctx, loading, "module")?;
        let namespace = namespace
            .into_object()
            .ok_or(Failure::Js(rquickjs::Error::Unknown))?;
        let search: Value = namespace.get("search")?;
        search.into_function().ok_or_else(|| {
            Failure::Report(
                Diagnostic::new("missing-search", "doesn't export a `search` function")
                    .file(entry)
                    .hint("Add `export function search(query) { return []; }` to the entry module."),
            )
        })
    }

    /// Runs promise jobs until `promise` settles, the job budget is spent or nothing is left to
    /// run.
    fn settle<'js>(&self, ctx: &Ctx<'js>, promise: Promise<'js>, what: &str) -> Result<Value<'js>, Failure> {
        let code = if what == "module" {
            "module-threw"
        } else {
            "search-threw"
        };
        let mut jobs = 0;
        loop {
            match promise.state() {
                PromiseState::Resolved => {
                    return promise.result::<Value>().expect("settled").map_err(Failure::Js);
                }
                PromiseState::Rejected => {
                    let error = promise.result::<Value>().expect("settled").expect_err("rejected");
                    return Err(self.thrown(ctx, error, code));
                }
                PromiseState::Pending => {}
            }
            if self.watchdog.tripped() {
                return Err(Failure::Stopped);
            }
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
            if !ctx.execute_pending_job() {
                return Err(Failure::Report(
                    Diagnostic::new(
                        "unsettled",
                        format!("{what} returned a promise that never settles"),
                    )
                    .file(&self.package.entry)
                    .hint("Resolve the promise; extensions have no timers or I/O to wait on yet."),
                ));
            }
            jobs += 1;
        }
    }

    /// Turns a JavaScript exception into a failure, recognizing the limits QuickJS reports as
    /// exceptions.
    fn thrown(&self, ctx: &Ctx<'_>, error: rquickjs::Error, code: &'static str) -> Failure {
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
                    format!("search used more than {} KiB of stack", STACK_BYTES / 1024),
                )
                .file(entry)
                .hint("Replace deep recursion with a loop."),
            );
        }
        let (message, hint) = if code == "module-threw" {
            (
                format!("loading the module threw {}", thrown.message),
                "Fix the error in the module's top-level code.",
            )
        } else {
            (
                format!("search threw {}", thrown.message),
                "Catch the error inside search, or return an item that explains the problem.",
            )
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
