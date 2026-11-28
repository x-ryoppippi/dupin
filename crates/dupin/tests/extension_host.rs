//! The extension host as Dupin ships it: this package's `dupin` executable started with
//! `--extension-host`, which runs guests only where it has a sandbox: macOS.
#![cfg(target_os = "macos")]

use dupin_spawn::Command;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dupin_extension_host::{Compiled, FLAG, Host, Trust};
use dupin_js_runtime::{DEFAULT_ENTRY, Sources};

fn host() -> Host {
    Host::new(Box::new(|| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dupin"));
        command.arg(FLAG).env_clear().current_dir("/");
        Ok(command)
    }))
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn compile(host: &Host, folder: &Path) -> Compiled {
    let sources = Sources::read(folder, DEFAULT_ENTRY).unwrap();
    host.compile(&sources, Trust::Own, Duration::from_secs(5), &|| false)
        .unwrap()
}

#[test]
fn the_dupin_executable_serves_as_the_extension_host() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let host = host();
    let package = compile(&host, &root().join("examples/extensions/word-count-js"));
    let rows = host
        .search(
            &package,
            "two words",
            10,
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &(std::sync::Arc::new(dupin_extension_host::NoServices) as _),
        )
        .unwrap();
    assert_eq!(rows[0].title, "2 words");
    assert!(host.helper_pid(&package).is_some());
}

/// Run with `cargo test --release -p dupin --test extension_host -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn latency_through_the_extension_host() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join(DEFAULT_ENTRY), search_source()).unwrap();
    let sources = Sources::read(folder.path(), DEFAULT_ENTRY).unwrap();
    let compile_in = |host: &Host| {
        let started = Instant::now();
        let package = host
            .compile(&sources, Trust::Own, Duration::from_secs(2), &|| false)
            .unwrap();
        (package, started.elapsed())
    };
    let package = compile_in(&host()).0;
    let query = |host: &Host| {
        let started = Instant::now();
        let rows = host
            .search(
                &package,
                "pr",
                50,
                Instant::now() + Duration::from_secs(2),
                &|| false,
                &(std::sync::Arc::new(dupin_extension_host::NoServices) as _),
            )
            .unwrap();
        assert_eq!(rows.len(), 50);
        started.elapsed()
    };

    let compiles: Vec<_> = (0..100).map(|_| compile_in(&host()).1).collect();
    let cold: Vec<_> = (0..100).map(|_| query(&host())).collect();
    let host = host();
    for _ in 0..50 {
        query(&host);
    }
    let warm: Vec<_> = (0..2000).map(|_| query(&host)).collect();
    println!(
        "compile, in a helper of its own (spawn, sandbox, handshake, compile, bytecode back): {}",
        summary(compiles)
    );
    println!(
        "cold first result from cached bytecode (spawn, sandbox, handshake, load, search): {}",
        summary(cold)
    );
    println!(
        "warm query (send bytecode and query, search, reply, validation): {}",
        summary(warm)
    );
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    let pid = host.helper_pid(&package).unwrap() as i32;
    // SAFETY: `usage` is large enough for the RUSAGE_INFO_V4 flavor the call writes.
    let status = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, usage.as_mut_ptr().cast()) };
    assert_eq!(status, 0);
    // SAFETY: a zero status means the call filled `usage`.
    let usage = unsafe { usage.assume_init() };
    println!(
        "helper footprint after the warm queries: {:.2} MiB now, {:.2} MiB at most",
        usage.ri_phys_footprint as f64 / 1024.0 / 1024.0,
        usage.ri_lifetime_max_phys_footprint as f64 / 1024.0 / 1024.0
    );
}

fn summary(mut samples: Vec<Duration>) -> String {
    samples.sort();
    let at = |quantile: f64| {
        let index = ((samples.len() as f64 * quantile).ceil() as usize).saturating_sub(1);
        samples[index].as_secs_f64() * 1000.0
    };
    format!(
        "n={} p50 {:.2} ms, p95 {:.2} ms, p99 {:.2} ms",
        samples.len(),
        at(0.50),
        at(0.95),
        at(0.99)
    )
}

/// The 400-candidate search from `experiments/js-runtime`, with the same generated data.
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
    let capitalize = |word: &str| word[..1].to_uppercase() + &word[1..];
    let data: Vec<_> = adjectives
        .iter()
        .flat_map(|adjective| {
            nouns.iter().map(move |noun| {
                let id = format!("{adjective}-{noun}");
                serde_json::json!({
                    "id": id,
                    "title": format!("{} {}", capitalize(adjective), capitalize(noun)),
                    "subtitle": format!("~/Projects/{id}"),
                    "path": format!("/Users/example/Projects/{id}"),
                })
            })
        })
        .collect();
    let search =
        std::fs::read_to_string(root().join("experiments/js-runtime/bench/fixtures/search.js")).unwrap();
    format!("const DATA = {};\n{search}", serde_json::Value::Array(data))
}
