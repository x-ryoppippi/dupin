use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use rquickjs::{
    CatchResultExt, Context, Ctx, Function, Module, Object, Runtime, Value, WriteOptions,
    promise::PromiseState,
};

pub const HEAP_LIMIT: usize = 16 * 1024 * 1024;
pub const STACK_LIMIT: usize = 512 * 1024;
pub const BUDGET: Duration = Duration::from_millis(100);
pub const JOB_LIMIT: usize = 10_000;
const ROW_LIMIT: usize = 200;
const FIELD_LIMIT: usize = 1024;

pub enum Code<'a> {
    Bytecode(&'a [u8]),
    Source(&'a str),
}

pub struct Row {
    pub title: String,
}

#[derive(Debug)]
pub enum Failure {
    Engine(String),
    Thrown(String),
    Deadline,
    JobLimit,
    NeverSettled,
    Invalid(String),
}

impl Failure {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Engine(_) => "engine",
            Self::Thrown(_) => "thrown",
            Self::Deadline => "deadline",
            Self::JobLimit => "job-limit",
            Self::NeverSettled => "never-settled",
            Self::Invalid(_) => "invalid",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Engine(text) | Self::Thrown(text) | Self::Invalid(text) => text.clone(),
            Self::Deadline => format!("pending after the {} ms budget", BUDGET.as_millis()),
            Self::JobLimit => format!("more than {JOB_LIMIT} promise jobs"),
            Self::NeverSettled => "job queue empty before the result settled".to_owned(),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Stages {
    pub setup: Duration,
    pub load: Duration,
    pub call: Duration,
    pub validate: Duration,
    pub teardown: Duration,
    pub total: Duration,
}

pub struct Report {
    pub result: Result<Vec<Row>, Failure>,
    pub stages: Stages,
    pub interrupted: bool,
    pub jobs: usize,
    pub malloc_size: i64,
    pub memory_used_size: i64,
}

/// One invocation as the design describes it: a fresh runtime and context with
/// heap, stack, deadline and job limits, dropped before the rows are returned.
pub fn invoke(code: &Code, query: &str) -> Report {
    let start = Instant::now();
    let deadline = start + BUDGET;
    let mut stages = Stages::default();
    let interrupted = Rc::new(Cell::new(false));
    let mut jobs = 0;

    let runtime = match Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => return failed(Failure::Engine(error.to_string()), start),
    };
    runtime.set_memory_limit(HEAP_LIMIT);
    runtime.set_max_stack_size(STACK_LIMIT);
    let flag = Rc::clone(&interrupted);
    runtime.set_interrupt_handler(Some(Box::new(move || {
        let expired = Instant::now() >= deadline;
        if expired {
            flag.set(true);
        }
        expired
    })));
    let context = match Context::full(&runtime) {
        Ok(context) => context,
        Err(error) => return failed(Failure::Engine(error.to_string()), start),
    };
    stages.setup = start.elapsed();

    let result = context.with(|ctx| {
        let started = Instant::now();
        let declared = match code {
            // The bytecode was written by this process from trusted source.
            Code::Bytecode(bytes) => unsafe { Module::load(ctx.clone(), bytes) },
            Code::Source(source) => Module::declare(ctx.clone(), "main.js", *source),
        }
        .catch(&ctx)
        .map_err(|error| Failure::Thrown(error.to_string()))?;
        let (module, evaluated) = declared
            .eval()
            .catch(&ctx)
            .map_err(|error| Failure::Thrown(error.to_string()))?;
        settle(&ctx, evaluated.into_value(), deadline, &mut jobs)?;
        stages.load = started.elapsed();

        let started = Instant::now();
        let search: Function = module
            .get("search")
            .catch(&ctx)
            .map_err(|error| Failure::Invalid(format!("missing search export: {error}")))?;
        let value: Value = search
            .call((query,))
            .catch(&ctx)
            .map_err(|error| Failure::Thrown(error.to_string()))?;
        let value = settle(&ctx, value, deadline, &mut jobs)?;
        stages.call = started.elapsed();

        let started = Instant::now();
        let rows = validate(&value);
        stages.validate = started.elapsed();
        rows
    });

    let usage = runtime.memory_usage();
    let started = Instant::now();
    drop(context);
    drop(runtime);
    stages.teardown = started.elapsed();
    stages.total = start.elapsed();
    Report {
        result,
        stages,
        interrupted: interrupted.get(),
        jobs,
        malloc_size: usage.malloc_size,
        memory_used_size: usage.memory_used_size,
    }
}

fn failed(failure: Failure, start: Instant) -> Report {
    Report {
        result: Err(failure),
        stages: Stages {
            total: start.elapsed(),
            ..Stages::default()
        },
        interrupted: false,
        jobs: 0,
        malloc_size: 0,
        memory_used_size: 0,
    }
}

fn settle<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    deadline: Instant,
    jobs: &mut usize,
) -> Result<Value<'js>, Failure> {
    let Some(promise) = value.as_promise().cloned() else {
        return Ok(value);
    };
    loop {
        match promise.state() {
            PromiseState::Resolved | PromiseState::Rejected => {
                return promise
                    .result::<Value>()
                    .ok_or(Failure::NeverSettled)?
                    .catch(ctx)
                    .map_err(|error| Failure::Thrown(error.to_string()));
            }
            PromiseState::Pending => {
                if *jobs >= JOB_LIMIT {
                    return Err(Failure::JobLimit);
                }
                if Instant::now() >= deadline {
                    return Err(Failure::Deadline);
                }
                if !ctx.execute_pending_job() {
                    return Err(Failure::NeverSettled);
                }
                *jobs += 1;
            }
        }
    }
}

fn validate(value: &Value) -> Result<Vec<Row>, Failure> {
    let array = value
        .as_array()
        .ok_or_else(|| Failure::Invalid("search must return an array".to_owned()))?;
    if array.len() > ROW_LIMIT {
        return Err(Failure::Invalid(format!(
            "{} rows exceed the {ROW_LIMIT}-row limit",
            array.len()
        )));
    }
    array
        .iter::<Object>()
        .map(|item| {
            let item = item.map_err(|error| Failure::Invalid(format!("row: {error}")))?;
            let title: String = item
                .get("title")
                .map_err(|error| Failure::Invalid(format!("title: {error}")))?;
            for field in ["id", "subtitle", "text"] {
                let text: Option<String> = item
                    .get(field)
                    .map_err(|error| Failure::Invalid(format!("{field}: {error}")))?;
                if text.is_some_and(|text| text.len() > FIELD_LIMIT) {
                    return Err(Failure::Invalid(format!("{field} is too long")));
                }
            }
            if title.len() > FIELD_LIMIT {
                return Err(Failure::Invalid("title is too long".to_owned()));
            }
            Ok(Row { title })
        })
        .collect()
}

/// Compiles source to the bytecode a host cache would keep.
pub fn compile(source: &str) -> Result<Vec<u8>, String> {
    let runtime = Runtime::new().map_err(|error| error.to_string())?;
    let context = Context::full(&runtime).map_err(|error| error.to_string())?;
    context.with(|ctx| {
        let module = Module::declare(ctx.clone(), "main.js", source)
            .catch(&ctx)
            .map_err(|error| error.to_string())?;
        module
            .write(WriteOptions::default())
            .catch(&ctx)
            .map_err(|error| error.to_string())
    })
}
