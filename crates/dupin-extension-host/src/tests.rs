use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use dupin_js_runtime::{DEFAULT_ENTRY, Diagnostic, Package, RunError};
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
    Host::new(Box::new(|| {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "--exact",
                "tests::helper_entry",
                "--nocapture",
                "--test-threads=1",
            ])
            .env_clear()
            .env(HELPER_ENV, "1")
            .current_dir("/");
        Ok(command)
    }))
}

fn package(source: &str) -> (tempfile::TempDir, Package) {
    let folder = tempfile::tempdir().unwrap();
    fs::write(folder.path().join(DEFAULT_ENTRY), source).unwrap();
    let package = Package::load(folder.path(), DEFAULT_ENTRY).unwrap();
    (folder, package)
}

fn search(
    host: &Host,
    package: &Package,
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

fn failure(result: Result<Vec<dupin_plugin_protocol::PluginItem>, RunError>) -> Diagnostic {
    match result {
        Err(RunError::Failed(diagnostic)) => diagnostic,
        other => panic!("expected a failure, got {other:?}"),
    }
}

fn is_sandboxed(pid: u32) -> bool {
    unsafe extern "C" {
        fn sandbox_check(pid: libc::pid_t, operation: *const std::ffi::c_char, kind: i32, ...) -> i32;
    }
    // SAFETY: a null operation asks only whether `pid` is sandboxed, and takes no other
    // arguments.
    unsafe { sandbox_check(pid as libc::pid_t, std::ptr::null(), 0) == 1 }
}

const ECHO: &str = "export function search(query) { return [{ title: `echo ${query}` }]; }";

#[test]
fn queries_run_in_one_resident_helper_and_packages_are_sent_once() {
    let host = host();
    let (_folder, echo) = package(ECHO);
    let (_other, other) = package("export const search = () => [{ title: 'other' }];");
    assert_eq!(host.helper_pid(), None, "the helper starts on first use");
    assert_eq!(
        titles(&search(&host, &echo, "one", Duration::from_secs(5)).unwrap()),
        ["echo one"]
    );
    let pid = host.helper_pid().unwrap();
    assert_ne!(pid, std::process::id());
    assert!(is_sandboxed(pid));
    assert!(!is_sandboxed(std::process::id()));
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

    let (_folder, broken) = package("export function search() { throw new TypeError('nope'); }");
    let diagnostic = failure(search(&host, &broken, "", Duration::from_secs(5)));
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

const UNINTERRUPTIBLE: &str = "export function search() { return [{ title: String('a'.repeat(4e6).indexOf('a'.repeat(2e6) + 'b')) }]; }";

#[test]
fn a_native_call_the_engine_cant_interrupt_ends_by_its_deadline_and_the_helper_restarts() {
    let host = host();
    let (_folder, echo) = package(ECHO);
    search(&host, &echo, "warm", Duration::from_secs(5)).unwrap();
    let first = host.helper_pid().unwrap();

    let (_folder, stuck) = package(UNINTERRUPTIBLE);
    let deadline = Duration::from_millis(300);
    let started = Instant::now();
    let diagnostic = failure(search(&host, &stuck, "", deadline));
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
    let host = host();
    let (_folder, echo) = package(ECHO);
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

    let (_folder, busy) = package("export function search() { for (;;) {} }");
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

    let (_folder, stuck) = package(UNINTERRUPTIBLE);
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
    let host = Arc::new(host());
    let (_folder, echo) = package(ECHO);
    search(&host, &echo, "warm", Duration::from_secs(5)).unwrap();
    let pid = host.helper_pid().unwrap();

    let (_folder, busy) = package("export function search() { for (;;) {} }");
    let running = {
        let host = host.clone();
        std::thread::spawn(move || search(&host, &busy, "", Duration::from_secs(10)))
    };
    std::thread::sleep(Duration::from_millis(200));
    // SAFETY: `pid` is the helper this host started and still owns.
    assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGKILL) }, 0);
    let diagnostic = failure(running.join().unwrap());
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
    let host = host();
    let (_folder, probe) = package(
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
    sandbox::enter().unwrap();
    sandbox::limit_resources();
    report("after");
}

#[test]
fn the_sandbox_denies_files_network_and_processes() {
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
