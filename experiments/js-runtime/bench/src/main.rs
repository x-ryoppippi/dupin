mod engine;

use std::{
    env, fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use engine::{BUDGET, Code, HEAP_LIMIT, JOB_LIMIT, Report, STACK_LIMIT, Stages, invoke};
use serde_json::{Value as Json, json};

const SEARCH: &str = include_str!("../fixtures/search.js");
const WORD_COUNT: &str = include_str!("../fixtures/word-count.js");
const SEARCH_QUERY: &str = "pr";
const WORD_QUERY: &str = "two words";
const SEARCH_ROWS: usize = 50;
/// Rust's default for spawned threads, as provider workers use.
const WORKER_STACK: usize = 2 * 1024 * 1024;
const WARM_GATE_P95_MS: f64 = 5.0;
const WARM_GATE_P99_MS: f64 = 10.0;
const COLD_GATE_P95_MS: f64 = 20.0;
const OUTER_LIMIT: Duration = Duration::from_secs(2);
const KILL_AFTER: Duration = Duration::from_secs(10);
const DEADLINE_TOLERANCE: Duration = Duration::from_millis(10);
const HEAP_SLACK: u64 = 8 * 1024 * 1024;

macro_rules! cases {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_str!(concat!("../fixtures/containment/", $name, ".js")))),*]
    };
}

const CASES: &[(&str, &str)] = cases![
    "busy-loop",
    "busy-module-body",
    "busy-async-job",
    "heap-object-spike",
    "heap-string-repeat",
    "heap-string-doubling",
    "heap-array-fill",
    "heap-typed-array",
    "stack-recursion",
    "stack-json-parse",
    "stack-json-stringify",
    "promise-chain-storm",
    "promise-fanout-storm",
    "regexp-backtracking",
    "native-string-search",
    "native-array-sort",
];

fn main() {
    let started = Instant::now();
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = match args.as_slice() {
        ["all", "--label", label, "--samples", samples, "--cold-runs", runs] => all(
            label,
            samples.parse().expect("sample count"),
            runs.parse().expect("cold run count"),
        ),
        ["cold-child", mode, path] => cold_child(started, mode, Path::new(path)),
        ["footprint-child", path, cycles] => {
            footprint_child(Path::new(path), cycles.parse().expect("cycle count"))
        }
        ["contain-child", name] => contain_child(name),
        ["contain", names @ ..] if !names.is_empty() => Json::Array(contain(names)),
        _ => {
            eprintln!("usage: dupin-js-runtime-bench all --label <label> --samples <n> --cold-runs <n>");
            eprintln!("       dupin-js-runtime-bench contain <case>...");
            std::process::exit(2);
        }
    };
    println!("{output}");
}

fn on_worker<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    thread::Builder::new()
        .stack_size(WORKER_STACK)
        .spawn(work)
        .expect("spawn worker")
        .join()
        .expect("worker panicked")
}

fn search_source() -> String {
    let adjectives = [
        "amber", "brisk", "cobalt", "dusty", "eager", "frosty", "gentle", "hollow", "ivory", "jolly", "keen",
        "lunar", "mellow", "nimble", "olive", "proud", "quiet", "rapid", "silver", "tidal",
    ];
    let nouns = [
        "anchor", "beacon", "canyon", "delta", "ember", "falcon", "garden", "harbor", "island", "jungle",
        "kettle", "lantern", "meadow", "nebula", "orchard", "prairie", "quarry", "river", "summit",
        "thicket",
    ];
    let data: Vec<Json> = adjectives
        .iter()
        .flat_map(|adjective| {
            nouns.iter().map(move |noun| {
                let id = format!("{adjective}-{noun}");
                json!({
                    "id": id,
                    "title": format!("{} {}", capitalize(adjective), capitalize(noun)),
                    "subtitle": format!("~/Projects/{id}"),
                    "path": format!("/Users/example/Projects/{id}"),
                })
            })
        })
        .collect();
    assert_eq!(data.len(), 400);
    format!("const DATA = {};\n{SEARCH}", Json::Array(data))
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn check(report: &Report, rows: usize) {
    match &report.result {
        Ok(found) if found.len() == rows => {}
        Ok(found) => panic!("expected {rows} rows, got {}", found.len()),
        Err(failure) => panic!("invocation failed: {}", failure.detail()),
    }
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn summary(mut samples: Vec<f64>) -> Json {
    samples.sort_by(f64::total_cmp);
    let rank = |p: f64| samples[((p / 100.0 * samples.len() as f64).ceil() as usize).max(1) - 1];
    json!({
        "count": samples.len(),
        "min_ms": samples[0],
        "p50_ms": rank(50.0),
        "p95_ms": rank(95.0),
        "p99_ms": rank(99.0),
        "max_ms": samples[samples.len() - 1],
        "mean_ms": samples.iter().sum::<f64>() / samples.len() as f64,
    })
}

fn stage_summary(stages: &[Stages]) -> Json {
    let pick = |f: fn(&Stages) -> Duration| summary(stages.iter().map(|s| millis(f(s))).collect());
    json!({
        "total": pick(|s| s.total),
        "setup": pick(|s| s.setup),
        "load": pick(|s| s.load),
        "call": pick(|s| s.call),
        "validate": pick(|s| s.validate),
        "teardown": pick(|s| s.teardown),
    })
}

fn warm(bytecode: Vec<u8>, query: &'static str, rows: usize, samples: usize) -> Json {
    on_worker(move || {
        for _ in 0..50 {
            check(&invoke(&Code::Bytecode(&bytecode), query), rows);
        }
        let stages: Vec<Stages> = (0..samples)
            .map(|_| {
                let report = invoke(&Code::Bytecode(&bytecode), query);
                check(&report, rows);
                report.stages
            })
            .collect();
        json!({
            "query": query,
            "rows": rows,
            "bytecode_bytes": bytecode.len(),
            "warmup_calls": 50,
            "stages": stage_summary(&stages),
        })
    })
}

fn footprint() -> (u64, u64) {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let status = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V4,
            (&raw mut info).cast::<libc::rusage_info_t>(),
        )
    };
    assert_eq!(status, 0, "proc_pid_rusage failed");
    (info.ri_phys_footprint, info.ri_lifetime_max_phys_footprint)
}

fn run_child(args: &[&str]) -> (Json, Duration, bool) {
    let started = Instant::now();
    let mut child = Command::new(env::current_exe().expect("current exe"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn child");
    loop {
        if let Some(status) = child.try_wait().expect("wait for child") {
            let elapsed = started.elapsed();
            let mut stdout = String::new();
            child
                .stdout
                .take()
                .expect("child stdout")
                .read_to_string(&mut stdout)
                .expect("read child stdout");
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .expect("child stderr")
                .read_to_string(&mut stderr)
                .expect("read child stderr");
            let value = serde_json::from_str(stdout.trim()).unwrap_or_else(|_| {
                json!({
                    "exit_status": status.to_string(),
                    "stdout": stdout,
                    "stderr": stderr,
                })
            });
            return (value, elapsed, false);
        }
        if started.elapsed() >= KILL_AFTER {
            child.kill().expect("kill child");
            child.wait().expect("reap child");
            return (json!(null), started.elapsed(), true);
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn cold_child(started: Instant, mode: &str, path: &Path) -> Json {
    let bytes = fs::read(path).expect("read cached module");
    let report = on_worker({
        let mode = mode.to_owned();
        move || match mode.as_str() {
            "bytecode" => invoke(&Code::Bytecode(&bytes), SEARCH_QUERY),
            "source" => invoke(
                &Code::Source(std::str::from_utf8(&bytes).expect("utf-8 source")),
                SEARCH_QUERY,
            ),
            _ => panic!("unknown cold mode {mode}"),
        }
    });
    check(&report, SEARCH_ROWS);
    json!({
        "first_result_ms": millis(started.elapsed()),
        "invocation_ms": millis(report.stages.total),
    })
}

fn cold(dir: &Path, mode: &str, runs: usize) -> Json {
    let path = dir.join(if mode == "bytecode" {
        "search.qjsbc"
    } else {
        "search.js"
    });
    let path = path.to_str().expect("utf-8 path");
    let mut first = Vec::new();
    let mut invocation = Vec::new();
    let mut process = Vec::new();
    for _ in 0..runs {
        let (value, elapsed, killed) = run_child(&["cold-child", mode, path]);
        assert!(!killed, "cold child did not exit");
        first.push(value["first_result_ms"].as_f64().expect("cold child result"));
        invocation.push(value["invocation_ms"].as_f64().expect("cold child result"));
        process.push(millis(elapsed));
    }
    json!({
        "mode": mode,
        "first_result_in_process": summary(first),
        "first_invocation": summary(invocation),
        "process_spawn_to_exit": summary(process),
    })
}

fn footprint_child(path: &Path, cycles: usize) -> Json {
    let bytecode = fs::read(path).expect("read cached module");
    let idle = footprint().0;
    on_worker(move || {
        let live = {
            let runtime = rquickjs::Runtime::new().expect("runtime");
            runtime.set_memory_limit(HEAP_LIMIT);
            runtime.set_max_stack_size(STACK_LIMIT);
            let context = rquickjs::Context::full(&runtime).expect("context");
            let empty = (footprint().0, runtime.memory_usage().malloc_size);
            context.with(|ctx| {
                let module = unsafe { rquickjs::Module::load(ctx.clone(), &bytecode) }.expect("load module");
                let (module, promise) = module.eval().expect("evaluate module");
                promise.finish::<()>().expect("settle module");
                let search: rquickjs::Function = module.get("search").expect("search export");
                let rows: rquickjs::Array = search.call((SEARCH_QUERY,)).expect("call search");
                assert_eq!(rows.len(), SEARCH_ROWS);
            });
            let usage = runtime.memory_usage();
            json!({
                "empty_context_footprint_bytes": empty.0,
                "empty_context_malloc_bytes": empty.1,
                "loaded_context_footprint_bytes": footprint().0,
                "loaded_context_malloc_bytes": usage.malloc_size,
                "loaded_context_memory_used_bytes": usage.memory_used_size,
            })
        };
        let after_drop = footprint().0;
        let mut checkpoints = Vec::new();
        for cycle in 1..=cycles {
            check(&invoke(&Code::Bytecode(&bytecode), SEARCH_QUERY), SEARCH_ROWS);
            if cycle == 1 || cycle == 10 || cycle % 100 == 0 {
                checkpoints.push(json!({"cycles": cycle, "footprint_bytes": footprint().0}));
            }
        }
        let (current, peak) = footprint();
        json!({
            "idle_footprint_bytes": idle,
            "live": live,
            "after_first_drop_footprint_bytes": after_drop,
            "create_drop_cycles": cycles,
            "checkpoints": checkpoints,
            "final_footprint_bytes": current,
            "lifetime_peak_footprint_bytes": peak,
            "peak_over_idle_bytes": peak.saturating_sub(idle),
        })
    })
}

fn contain_child(name: &str) -> Json {
    let source = CASES
        .iter()
        .find(|(case, _)| *case == name)
        .map(|(_, source)| *source)
        .expect("known containment case");
    let idle = footprint().0;
    let report = on_worker(move || invoke(&Code::Source(source), "query"));
    let (current, peak) = footprint();
    let (outcome, detail, rows) = match &report.result {
        Ok(rows) => (
            "returned",
            rows.first().map(|row| row.title.clone()).unwrap_or_default(),
            rows.len(),
        ),
        Err(failure) => (failure.kind(), failure.detail(), 0),
    };
    json!({
        "outcome": outcome,
        "detail": detail.chars().take(300).collect::<String>(),
        "rows": rows,
        "interrupted": report.interrupted,
        "jobs": report.jobs,
        "elapsed_ms": millis(report.stages.total - report.stages.teardown),
        "teardown_ms": millis(report.stages.teardown),
        "malloc_bytes_at_end": report.malloc_size,
        "memory_used_bytes_at_end": report.memory_used_size,
        "idle_footprint_bytes": idle,
        "final_footprint_bytes": current,
        "peak_over_idle_bytes": peak.saturating_sub(idle),
    })
}

fn contain(names: &[&str]) -> Vec<Json> {
    names
        .iter()
        .map(|name| {
            let (value, wall, killed) = run_child(&["contain-child", name]);
            let elapsed = value["elapsed_ms"].as_f64();
            let peak = value["peak_over_idle_bytes"].as_u64();
            let in_time = elapsed.is_some_and(|ms| ms <= millis(BUDGET + DEADLINE_TOLERANCE)) && !killed;
            let heap_held = peak.is_some_and(|bytes| bytes <= HEAP_LIMIT as u64 + HEAP_SLACK);
            let crashed = value["outcome"].as_str().is_none();
            json!({
                "case": name,
                "result": value,
                "process_wall_ms": millis(wall),
                "killed_after_ms": killed.then(|| millis(KILL_AFTER)),
                "terminated_within_outer_limit": !killed && wall <= OUTER_LIMIT,
                "stopped_within_budget": in_time,
                "heap_held": heap_held,
                "pass": !crashed && in_time && heap_held,
            })
        })
        .collect()
}

fn all(label: &str, samples: usize, runs: usize) -> Json {
    let dir = env::temp_dir().join(format!("dupin-js-runtime-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    let search = search_source();
    let search_bytecode = engine::compile(&search).expect("compile search fixture");
    let word_bytecode = engine::compile(WORD_COUNT).expect("compile word-count fixture");
    fs::write(dir.join("search.js"), &search).expect("write source");
    fs::write(dir.join("search.qjsbc"), &search_bytecode).expect("write bytecode");

    eprintln!("[{label}] warm calls…");
    let search_warm = warm(search_bytecode, SEARCH_QUERY, SEARCH_ROWS, samples);
    let word_warm = warm(word_bytecode, WORD_QUERY, 1, samples);
    eprintln!("[{label}] cold processes…");
    let cold_bytecode = cold(&dir, "bytecode", runs);
    let cold_source = cold(&dir, "source", runs);
    eprintln!("[{label}] footprint…");
    let bytecode_path = dir.join("search.qjsbc");
    let (footprint, _, killed) = run_child(&[
        "footprint-child",
        bytecode_path.to_str().expect("utf-8 path"),
        "1000",
    ]);
    assert!(!killed, "footprint child did not exit");
    eprintln!("[{label}] containment…");
    let names: Vec<&str> = CASES.iter().map(|(name, _)| *name).collect();
    let containment = contain(&names);
    fs::remove_dir_all(&dir).ok();

    let warm_p95 = search_warm["stages"]["total"]["p95_ms"].as_f64().unwrap();
    let warm_p99 = search_warm["stages"]["total"]["p99_ms"].as_f64().unwrap();
    let cold_p95 = |value: &Json| value["first_result_in_process"]["p95_ms"].as_f64().unwrap();
    let failed_cases: Vec<&str> = containment
        .iter()
        .filter(|case| !case["pass"].as_bool().unwrap_or(false))
        .map(|case| case["case"].as_str().unwrap())
        .collect();
    json!({
        "label": label,
        "limits": {
            "heap_bytes": HEAP_LIMIT,
            "stack_bytes": STACK_LIMIT,
            "execution_budget_ms": BUDGET.as_millis(),
            "deadline_tolerance_ms": DEADLINE_TOLERANCE.as_millis(),
            "job_limit": JOB_LIMIT,
            "outer_limit_ms": OUTER_LIMIT.as_millis(),
            "kill_after_ms": KILL_AFTER.as_millis(),
            "heap_slack_bytes": HEAP_SLACK,
            "worker_stack_bytes": WORKER_STACK,
        },
        "warm": {"search_400": search_warm, "word_count": word_warm},
        "cold": {"bytecode": cold_bytecode, "source": cold_source},
        "footprint": footprint,
        "containment": containment,
        "gates": {
            "warm_p95": {"limit_ms": WARM_GATE_P95_MS, "measured_ms": warm_p95, "pass": warm_p95 <= WARM_GATE_P95_MS},
            "warm_p99": {"limit_ms": WARM_GATE_P99_MS, "measured_ms": warm_p99, "pass": warm_p99 <= WARM_GATE_P99_MS},
            "cold_bytecode_p95": {"limit_ms": COLD_GATE_P95_MS, "measured_ms": cold_p95(&cold_bytecode), "pass": cold_p95(&cold_bytecode) <= COLD_GATE_P95_MS},
            "cold_source_p95": {"limit_ms": COLD_GATE_P95_MS, "measured_ms": cold_p95(&cold_source), "pass": cold_p95(&cold_source) <= COLD_GATE_P95_MS},
            "containment": {"failed_cases": failed_cases, "pass": failed_cases.is_empty()},
        },
    })
}
