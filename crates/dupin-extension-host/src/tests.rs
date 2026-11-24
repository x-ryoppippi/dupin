use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use dupin_js_runtime::{DEFAULT_ENTRY, Diagnostic, RunError, Sources};
use dupin_plugin_protocol::View;

use super::*;

const HELPER_ENV: &str = "DUPIN_EXTENSION_HOST_TEST";

/// Not a test: the helper process these tests start is this test binary running only this
/// function, which serves when the variable is set.
#[test]
fn helper_entry() {
    if std::env::var_os(HELPER_ENV).is_some() {
        serve();
    }
}

fn host() -> Host {
    Host::new(Box::new(|| helper_command("tests::helper_entry")))
}

/// This test binary running only `entry`. The zone is one without daylight saving time that is
/// unlikely to be the machine's, so a guest that sees UTC instead is caught.
fn helper_command(entry: &str) -> std::io::Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", entry, "--nocapture", "--test-threads=1"])
        .env_clear()
        .env(HELPER_ENV, "1")
        .env("TZ", "Asia/Tokyo")
        .current_dir("/");
    Ok(command)
}

/// A host whose helper takes `delay` to start.
fn slow_host(delay: &'static str) -> Host {
    Host::new(Box::new(move || {
        let helper = helper_command("tests::helper_entry")?;
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(format!("/bin/sleep {delay}; exec \"$0\" \"$@\""))
            .arg(helper.get_program())
            .args(helper.get_args())
            .env_clear()
            .envs(helper.get_envs().filter_map(|(key, value)| Some((key, value?))))
            .current_dir("/");
        Ok(command)
    }))
}

fn sources(files: &[(&str, &str)]) -> (tempfile::TempDir, Sources) {
    sources_from(DEFAULT_ENTRY, files)
}

/// `files` in a package folder whose entry is `entry`, read as Dupin reads them.
fn sources_from(entry: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, Sources) {
    let folder = tempfile::tempdir().unwrap();
    for (name, source) in files {
        let path = folder.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let sources = Sources::read(folder.path(), entry).unwrap();
    (folder, sources)
}

fn compile(host: &Host, files: &[(&str, &str)]) -> Result<Compiled, RunError> {
    let (_folder, sources) = sources(files);
    host.compile(&sources, Duration::from_secs(5), &|| false)
}

/// `source` as `main.js`, compiled.
fn package(source: &str) -> Compiled {
    compile(&host(), &[(DEFAULT_ENTRY, source)]).unwrap()
}

/// A host that counts the helpers it starts.
fn counting_host() -> (Host, Arc<AtomicUsize>) {
    let started = Arc::new(AtomicUsize::new(0));
    let count = started.clone();
    let host = Host::new(Box::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
        helper_command("tests::helper_entry")
    }));
    (host, started)
}

fn search(
    host: &Host,
    package: &Compiled,
    query: &str,
    within: Duration,
) -> Result<Vec<dupin_plugin_protocol::PluginItem>, RunError> {
    host.search(package, query, 10, Instant::now() + within, &|| false)
}

fn titles(rows: &[dupin_plugin_protocol::PluginItem]) -> Vec<String> {
    rows.iter()
        .map(|row| match &row.view {
            View::Text { title, .. } => title.clone(),
            other => panic!("unexpected view {other:?}"),
        })
        .collect()
}

fn failed<T: std::fmt::Debug>(result: Result<T, RunError>) -> Diagnostic {
    match result {
        Err(RunError::Failed(diagnostic)) => diagnostic,
        other => panic!("expected a failure, got {other:?}"),
    }
}

fn pid_is_sandboxed(pid: u32) -> bool {
    unsafe extern "C" {
        fn sandbox_check(pid: libc::pid_t, operation: *const std::ffi::c_char, kind: i32, ...) -> i32;
    }
    // SAFETY: a null operation asks only whether `pid` is sandboxed, and takes no other
    // arguments.
    unsafe { sandbox_check(pid as libc::pid_t, std::ptr::null(), 0) == 1 }
}

const ECHO: &str = "export function search(query) { return [{ title: `echo ${query}` }]; }";

#[test]
fn queries_run_in_one_resident_helper() {
    skip_inside_a_sandbox!();
    let host = host();
    let echo = compile(&host, &[(DEFAULT_ENTRY, ECHO)]).unwrap();
    let other = compile(
        &host,
        &[(DEFAULT_ENTRY, "export const search = () => [{ title: 'other' }];")],
    )
    .unwrap();
    assert_eq!(host.helper_pid(), None, "the helper starts on the first query");
    assert_eq!(
        titles(&search(&host, &echo, "one", Duration::from_secs(5)).unwrap()),
        ["echo one"]
    );
    let pid = host.helper_pid().unwrap();
    assert_ne!(pid, std::process::id());
    assert!(pid_is_sandboxed(pid));
    assert!(!pid_is_sandboxed(std::process::id()));
    assert_eq!(
        titles(&search(&host, &other, "", Duration::from_secs(5)).unwrap()),
        ["other"]
    );
    assert_eq!(
        titles(&search(&host, &echo, "two", Duration::from_secs(5)).unwrap()),
        ["echo two"]
    );
    host.check(&echo, Instant::now() + Duration::from_secs(5), &|| false)
        .unwrap();
    assert_eq!(host.helper_pid(), Some(pid));

    let broken = compile(
        &host,
        &[(
            DEFAULT_ENTRY,
            "export function search() { throw new TypeError('nope'); }",
        )],
    )
    .unwrap();
    let diagnostic = failed(search(&host, &broken, "", Duration::from_secs(5)));
    assert_eq!(
        (diagnostic.code.as_ref(), diagnostic.to_string().as_str()),
        ("search-threw", "main.js:1:38: search threw TypeError: nope")
    );
    assert_eq!(
        host.helper_pid(),
        Some(pid),
        "a guest error leaves the helper running"
    );
}

#[test]
fn every_compile_runs_in_a_helper_no_other_code_has_reached() {
    skip_inside_a_sandbox!();
    let (host, started) = counting_host();
    let first = compile(&host, &[(DEFAULT_ENTRY, ECHO)]).unwrap();
    assert_eq!(started.load(Ordering::Relaxed), 1);
    search(&host, &first, "warm", Duration::from_secs(5)).unwrap();
    let resident = host.helper_pid().unwrap();
    assert_eq!(started.load(Ordering::Relaxed), 2);

    let second = compile(
        &host,
        &[(
            DEFAULT_ENTRY,
            "export const search = () => [{ title: 'second' }];",
        )],
    )
    .unwrap();
    assert_eq!(
        started.load(Ordering::Relaxed),
        3,
        "the helper that ran a search compiles nothing"
    );
    assert_eq!(host.helper_pid(), Some(resident));
    assert_eq!(
        titles(&search(&host, &second, "", Duration::from_secs(5)).unwrap()),
        ["second"]
    );
    assert_eq!(started.load(Ordering::Relaxed), 3);
}

#[test]
fn a_compile_neither_waits_for_a_running_search_nor_counts_the_helper_start_against_its_limit() {
    skip_inside_a_sandbox!();
    let host = Arc::new(host());
    let busy = package("export function search() { for (;;) {} }");
    let running = {
        let host = host.clone();
        std::thread::spawn(move || search(&host, &busy, "", Duration::from_secs(3)))
    };
    std::thread::sleep(Duration::from_millis(200));
    compile(&host, &[(DEFAULT_ENTRY, ECHO)]).expect("compiled beside the running search");
    assert!(!running.is_finished(), "the search was still running");
    assert_eq!(failed(running.join().unwrap()).code, "deadline");

    let (_folder, sources) = sources(&[(DEFAULT_ENTRY, ECHO)]);
    slow_host("1")
        .compile(&sources, Duration::from_millis(100), &|| false)
        .expect("the limit starts once the helper is ready");
}

#[test]
fn compile_errors_come_back_with_the_file_line_and_import_at_fault() {
    skip_inside_a_sandbox!();
    let host = host();
    let syntax = failed(compile(
        &host,
        &[
            (
                DEFAULT_ENTRY,
                "import './lib/util.js'; export function search() {}",
            ),
            ("lib/util.js", "export const x = 1;\nexport function ( {"),
        ],
    ));
    assert_eq!(
        (
            syntax.code.as_ref(),
            syntax.file.as_deref(),
            syntax.line,
            syntax.hint.is_some()
        ),
        ("syntax-error", Some("lib/util.js"), Some(2), true),
        "{syntax:?}"
    );

    let import = failed(compile(
        &host,
        &[(
            DEFAULT_ENTRY,
            "import { x } from './missing.js'; export function search() {}",
        )],
    ));
    assert_eq!(
        (
            import.code.as_ref(),
            import.file.as_deref(),
            import.field.as_deref()
        ),
        (
            "import-missing",
            Some(DEFAULT_ENTRY),
            Some("import \"./missing.js\"")
        ),
        "{import:?}"
    );
}

#[test]
fn typescript_compiles_in_the_helper_and_reports_its_own_lines() {
    skip_inside_a_sandbox!();
    let host = host();
    let compile_ts = |files: &[(&str, &str)]| {
        let (_folder, sources) = sources_from("main.ts", files);
        host.compile(&sources, Duration::from_secs(5), &|| false)
    };
    let greet = (
        "lib/greet.ts",
        "export const greet = <T extends string>(name: T): string => `Hello, ${name}`;\n",
    );
    let package = compile_ts(&[
        (
            "main.ts",
            "import type { Item } from 'dupin';\n\
             import { greet } from './lib/greet.ts';\n\
             \n\
             interface Options {\n\
             \tlimit: number;\n\
             }\n\
             \n\
             export function search(query: string, _options: Options): Item[] {\n\
             \tif (query === 'throw') throw new Error(`no ${query as string}`);\n\
             \treturn [{ title: greet(query)! }];\n\
             }\n",
        ),
        greet,
    ])
    .unwrap();
    assert_eq!(
        titles(&search(&host, &package, "Dupin", Duration::from_secs(5)).unwrap()),
        ["Hello, Dupin"]
    );
    let thrown = failed(search(&host, &package, "throw", Duration::from_secs(5)));
    assert_eq!(
        (thrown.code.as_ref(), thrown.file.as_deref(), thrown.line),
        ("search-threw", Some("main.ts"), Some(9)),
        "{thrown:?}"
    );

    let rejected = failed(compile_ts(&[
        (
            "main.ts",
            "import './lib/greet.ts';\nexport function search() { return []; }\n",
        ),
        ("lib/greet.ts", "export const a = 1;\n\nenum Mode {\n\tOn,\n}\n"),
    ]));
    assert_eq!(
        (
            rejected.code.as_ref(),
            rejected.file.as_deref(),
            rejected.line,
            rejected.column
        ),
        ("typescript-needs-build", Some("lib/greet.ts"), Some(3), Some(1)),
        "{rejected:?}"
    );
    assert!(
        rejected
            .hint
            .as_deref()
            .is_some_and(|hint| hint.contains("dupin ext build")),
        "{rejected:?}"
    );
}

#[test]
fn compiled_bytecode_outlives_the_helper_that_ran_it() {
    skip_inside_a_sandbox!();
    let host = host();
    let echo = compile(&host, &[(DEFAULT_ENTRY, ECHO)]).unwrap();
    search(&host, &echo, "first", Duration::from_secs(5)).unwrap();
    let pid = host.helper_pid().unwrap();
    // SAFETY: `pid` is the helper this host started and still owns.
    assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGKILL) }, 0);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        titles(&search(&host, &echo, "again", Duration::from_secs(5)).unwrap()),
        ["echo again"]
    );
    assert_ne!(host.helper_pid(), Some(pid));
}

#[test]
fn a_compile_that_outlasts_its_deadline_is_stopped() {
    skip_inside_a_sandbox!();
    let host = host();
    let rows: String = (0..90_000).map(|index| format!("\"row {index:08}\",")).collect();
    let (_folder, large) = sources(&[(
        DEFAULT_ENTRY,
        &format!("export const rows = [{rows}];\nexport function search() {{ return []; }}"),
    )]);
    let started = Instant::now();
    let diagnostic = failed(host.compile(&large, Duration::from_millis(1), &|| false));
    assert_eq!(
        (diagnostic.code.as_ref(), diagnostic.file.as_deref()),
        ("deadline", Some(DEFAULT_ENTRY)),
        "{diagnostic:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert!(
        diagnostic.message.starts_with("compiling the package"),
        "{diagnostic}"
    );
}

#[test]
fn a_large_package_compiles_and_answers_within_the_helper_allowance() {
    skip_inside_a_sandbox!();
    let host = host();
    let rows: String = (0..90_000).map(|index| format!("\"row {index:08}\",")).collect();
    let large = compile(
        &host,
        &[(
            DEFAULT_ENTRY,
            &format!(
                "const rows = [{rows}];\nexport function search() {{ return [{{ title: String(rows.length) }}]; }}"
            ),
        )],
    )
    .unwrap();
    let mut pids = Vec::new();
    for _ in 0..3 {
        assert_eq!(
            titles(&search(&host, &large, "", Duration::from_secs(5)).unwrap()),
            ["90000"]
        );
        pids.push(host.helper_pid().unwrap());
    }
    assert!(pids.iter().all(|pid| *pid == pids[0]), "{pids:?}");
}

#[test]
fn guests_see_the_local_time_zone() {
    skip_inside_a_sandbox!();
    let host = host();
    let probe = package(
        "export const search = () => [{ title: `${new Date(1790000000000).getTimezoneOffset()} ${new Date(0).getHours()}` }];",
    );
    assert_eq!(
        titles(&search(&host, &probe, "", Duration::from_secs(5)).unwrap()),
        ["-540 9"]
    );
}

#[test]
fn a_helper_still_starting_neither_holds_a_call_past_its_deadline_nor_ignores_cancellation() {
    skip_inside_a_sandbox!();
    let echo = package(ECHO);
    let host = slow_host("1");
    let started = Instant::now();
    let diagnostic = failed(search(&host, &echo, "", Duration::from_millis(100)));
    assert_eq!(diagnostic.code, "host-starting", "{diagnostic:?}");
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "{:?}",
        started.elapsed()
    );
    let pid = host.helper_pid().expect("the starting helper is kept");

    let started = Instant::now();
    let result = host.search(&echo, "", 10, Instant::now() + Duration::from_secs(10), &|| {
        started.elapsed() > Duration::from_millis(50)
    });
    assert_eq!(result, Err(RunError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "{:?}",
        started.elapsed()
    );

    assert_eq!(
        titles(&search(&host, &echo, "ready", Duration::from_secs(5)).unwrap()),
        ["echo ready"]
    );
    assert_eq!(host.helper_pid(), Some(pid), "the same helper finished starting");
}

const ROGUE_ENV: &str = "DUPIN_EXTENSION_HOST_ROGUE";

/// Not a test: a compromised helper. It answers each call, then forges an answer to the next
/// one and keeps writing frames nobody asked for.
#[test]
fn rogue_helper_entry() {
    use std::os::fd::FromRawFd;
    use std::os::unix::net::UnixStream;

    use dupin_plugin_protocol::PluginItem;
    use wire::{Answer, FromHelper, ToHelper};

    if std::env::var_os(ROGUE_ENV).is_none() {
        return;
    }
    // SAFETY: the host starts this process with the socket as standard input.
    let mut socket = unsafe { UnixStream::from_raw_fd(0) };
    let rows = |title: &str| Answer::Rows {
        items: vec![PluginItem {
            id: "1".into(),
            view: View::Text {
                title: title.into(),
                subtitle: String::new(),
            },
            actions: Vec::new(),
        }],
    };
    let hello = FromHelper::Hello {
        version: wire::VERSION,
        engine: dupin_js_runtime::engine_version().into(),
    };
    wire::write(&mut socket, &hello, &[]).unwrap();
    while let Ok(Some((message, _))) = wire::read::<ToHelper>(&mut socket) {
        let ToHelper::Run { id, .. } = message else {
            continue;
        };
        let answer = FromHelper::Done {
            id,
            answer: rows("genuine"),
        };
        wire::write(&mut socket, &answer, &[]).unwrap();
        for forged in id + 1.. {
            let forged = FromHelper::Done {
                id: forged,
                answer: rows("forged"),
            };
            if wire::write(&mut socket, &forged, &[]).is_err() {
                std::process::exit(0);
            }
        }
    }
    std::process::exit(0);
}

#[test]
fn a_helper_that_speaks_out_of_turn_is_replaced_before_the_next_call() {
    skip_inside_a_sandbox!();
    let echo = package(ECHO);
    let host = Host::new(Box::new(|| {
        let mut command = helper_command("tests::rogue_helper_entry")?;
        command.env(ROGUE_ENV, "1");
        Ok(command)
    }));
    assert_eq!(
        titles(&search(&host, &echo, "", Duration::from_secs(5)).unwrap()),
        ["genuine"]
    );
    let first = host.helper_pid().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        titles(&search(&host, &echo, "", Duration::from_secs(5)).unwrap()),
        ["genuine"]
    );
    assert_ne!(host.helper_pid(), Some(first));
}

const UNINTERRUPTIBLE: &str = "export function search() { return [{ title: String('a'.repeat(4e6).indexOf('a'.repeat(2e6) + 'b')) }]; }";

#[test]
fn a_native_call_the_engine_cant_interrupt_ends_by_its_deadline_and_the_helper_restarts() {
    skip_inside_a_sandbox!();
    let host = host();
    let echo = package(ECHO);
    let stuck = package(UNINTERRUPTIBLE);
    search(&host, &echo, "warm", Duration::from_secs(5)).unwrap();
    let first = host.helper_pid().unwrap();

    let deadline = Duration::from_millis(300);
    let started = Instant::now();
    let diagnostic = failed(search(&host, &stuck, "", deadline));
    let took = started.elapsed();
    assert_eq!(diagnostic.code, "deadline", "{diagnostic:?}");
    assert!(
        took < deadline + Duration::from_millis(400),
        "ended after {took:?}"
    );
    assert_eq!(host.helper_pid(), None, "the stuck helper was stopped");

    assert_eq!(
        titles(&search(&host, &echo, "after", Duration::from_secs(5)).unwrap()),
        ["echo after"]
    );
    assert_ne!(host.helper_pid(), Some(first));
}

#[test]
fn cancellation_is_honored_by_the_engine_or_enforced_by_stopping_the_helper() {
    skip_inside_a_sandbox!();
    let host = host();
    let echo = package(ECHO);
    let busy = package("export function search() { for (;;) {} }");
    let stuck = package(UNINTERRUPTIBLE);
    search(&host, &echo, "warm", Duration::from_secs(5)).unwrap();
    let pid = host.helper_pid().unwrap();

    let cancel_after = |delay: Duration| {
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            flag.store(true, Ordering::Relaxed);
        });
        move || cancelled.load(Ordering::Relaxed)
    };

    let is_cancelled = cancel_after(Duration::from_millis(50));
    let started = Instant::now();
    let result = host.search(
        &busy,
        "",
        10,
        Instant::now() + Duration::from_secs(10),
        &is_cancelled,
    );
    assert_eq!(result, Err(RunError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        host.helper_pid(),
        Some(pid),
        "an interruptible guest keeps the helper"
    );

    let is_cancelled = cancel_after(Duration::from_millis(50));
    let started = Instant::now();
    let result = host.search(
        &stuck,
        "",
        10,
        Instant::now() + Duration::from_secs(10),
        &is_cancelled,
    );
    assert_eq!(result, Err(RunError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(host.helper_pid(), None);

    assert_eq!(
        titles(&search(&host, &echo, "next", Duration::from_secs(5)).unwrap()),
        ["echo next"]
    );
}

#[test]
fn a_helper_that_dies_becomes_a_failed_query_and_the_next_one_starts_another() {
    skip_inside_a_sandbox!();
    let host = Arc::new(host());
    let echo = package(ECHO);
    let busy = package("export function search() { for (;;) {} }");
    search(&host, &echo, "warm", Duration::from_secs(5)).unwrap();
    let pid = host.helper_pid().unwrap();

    let running = {
        let host = host.clone();
        std::thread::spawn(move || search(&host, &busy, "", Duration::from_secs(10)))
    };
    std::thread::sleep(Duration::from_millis(200));
    // SAFETY: `pid` is the helper this host started and still owns.
    assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGKILL) }, 0);
    let diagnostic = failed(running.join().unwrap());
    assert_eq!(diagnostic.code, "host-crashed", "{diagnostic:?}");

    assert_eq!(
        titles(&search(&host, &echo, "again", Duration::from_secs(5)).unwrap()),
        ["echo again"]
    );
    let restarted = host.helper_pid().unwrap();
    assert_ne!(restarted, pid);

    // A helper that dies between queries is replaced before the next one.
    // SAFETY: as above, for the replacement.
    assert_eq!(unsafe { libc::kill(restarted as i32, libc::SIGKILL) }, 0);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        titles(&search(&host, &echo, "still", Duration::from_secs(5)).unwrap()),
        ["echo still"]
    );
}

#[test]
fn guests_reach_no_module_or_global_beyond_the_language() {
    skip_inside_a_sandbox!();
    let host = host();
    let probe = package(
        r#"
        const names = ["require", "process", "fetch", "XMLHttpRequest", "WebSocket", "os", "std",
            "Deno", "Bun", "setTimeout", "print", "console", "scriptArgs", "importScripts"];
        export async function search() {
            const found = names.filter((name) => typeof globalThis[name] !== "undefined");
            for (const specifier of ["os", "std", "qjs:os", "qjs:std", "/etc/hosts", "file:///etc/hosts", "node:fs", "https://example.com/x.js"]) {
                try {
                    await import(specifier);
                    found.push(specifier);
                } catch {}
            }
            return found.map((title) => ({ title }));
        }
        "#,
    );
    assert_eq!(search(&host, &probe, "", Duration::from_secs(5)).unwrap(), []);
}

/// Not a test on its own: run in a child by the test below, it enters the sandbox and reports
/// what the process can still do.
#[test]
fn sandbox_probe() {
    if std::env::var_os(HELPER_ENV).is_none() {
        return;
    }
    let attempts = || {
        [
            ("read", fs::read("/etc/hosts").is_ok()),
            ("list", fs::read_dir("/").is_ok()),
            ("write", fs::write(probe_file(), b"x").is_ok()),
            (
                "connect",
                std::net::TcpStream::connect_timeout(&([1, 1, 1, 1], 80).into(), Duration::from_secs(1))
                    .is_ok(),
            ),
            ("bind", std::net::UdpSocket::bind(("127.0.0.1", 0)).is_ok()),
            (
                "unix",
                std::os::unix::net::UnixStream::connect("/var/run/mDNSResponder").is_ok(),
            ),
            ("exec", Command::new("/usr/bin/true").status().is_ok()),
        ]
    };
    let report = |stage: &str| {
        let allowed: Vec<_> = attempts()
            .into_iter()
            .filter(|(_, ok)| *ok)
            .map(|(name, _)| name)
            .collect();
        println!("probe {stage} {}", allowed.join(","));
    };
    report("before");
    // Without the resource limits, which would deny some of these on their own.
    sandbox::enter().unwrap();
    report("after");
}

#[test]
fn the_sandbox_denies_files_network_and_processes() {
    skip_inside_a_sandbox!();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::sandbox_probe",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(HELPER_ENV, "1")
        .env("TMPDIR", std::env::temp_dir())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = |stage: &str| {
        stdout
            .lines()
            .find_map(|line| line.split_once(&format!("probe {stage} ")).map(|(_, rest)| rest))
            .unwrap_or_else(|| panic!("no {stage} report in {stdout}"))
            .to_owned()
    };
    assert!(
        line("before").contains("read"),
        "the probe can read before: {stdout}"
    );
    assert_eq!(line("after"), "", "{stdout}");
    let _ = fs::remove_file(probe_file());
}

fn probe_file() -> std::path::PathBuf {
    std::env::temp_dir().join("dupin-extension-host-probe")
}

/// Not a test on its own: run in a child under `sandbox-exec` by the test below, so it is
/// already sandboxed when it starts the helper.
#[test]
fn nested_sandbox_probe() {
    if std::env::var_os(HELPER_ENV).is_none() {
        return;
    }
    println!("nested sandboxed {}", is_sandboxed());
    let (_folder, sources) = sources(&[(DEFAULT_ENTRY, ECHO)]);
    let error = failed(host().compile(&sources, Duration::from_secs(5), &|| false));
    println!("nested {} {}", error.code, error.message);
}

#[test]
fn a_helper_inside_another_sandbox_refuses_and_says_why() {
    skip_inside_a_sandbox!();
    let output = Command::new("/usr/bin/sandbox-exec")
        .args(["-p", "(version 1)\n(allow default)\n"])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::nested_sandbox_probe",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(HELPER_ENV, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("nested sandboxed true"), "{stdout}");
    assert!(
        stdout.contains("nested sandbox-unavailable")
            && stdout.contains("already runs inside another sandbox"),
        "{stdout}"
    );
}
