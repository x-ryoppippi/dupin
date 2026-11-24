//! `dupin ext test` and `dupin ext snap` as an author runs them: the built executable, which
//! starts itself as the extension host and, for `snap`, as a headless launcher.
#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn dupin(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dupin"))
        .args(args)
        .output()
        .unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&output.stdout)))
}

/// The PNG's width and height in pixels, from its header.
fn dimensions(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[12..16], b"IHDR");
    let read = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
    (read(16), read(20))
}

#[test]
fn test_replays_fixtures_and_snap_renders_the_same_bytes_every_time() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("greeter");
    dupin_ext_sdk::scaffold(&package, dupin_ext_sdk::Template::JavaScript).unwrap();
    let fixture = package.join("fixtures/search.json");

    let passed = dupin(&[
        "ext".as_ref(),
        "test".as_ref(),
        package.as_os_str(),
        "--json".as_ref(),
    ]);
    assert!(passed.status.success(), "{passed:?}");
    assert_eq!(json(&passed)["passed"], true);

    let shots: Vec<Vec<u8>> = (0..2)
        .map(|run| {
            let png = root.path().join(format!("panel-{run}.png"));
            let output = dupin(&[
                "ext".as_ref(),
                "snap".as_ref(),
                package.as_os_str(),
                fixture.as_os_str(),
                png.as_os_str(),
                "--json".as_ref(),
            ]);
            assert!(output.status.success(), "{output:?}");
            let described = json(&output);
            assert_eq!(described["step"], 2);
            assert_eq!(described["width"], 680);
            assert_eq!(described["scale"], 2);
            let bytes = std::fs::read(&png).unwrap();
            let height = described["height"].as_u64().unwrap() as u32;
            assert!(height > 60, "the panel shows the row: {height}");
            assert_eq!(dimensions(&bytes), (1360, height * 2));
            bytes
        })
        .collect();
    assert!(shots[0] == shots[1], "two renders of one fixture differ");

    let first_step = root.path().join("query.png");
    let output = dupin(&[
        "ext".as_ref(),
        "snap".as_ref(),
        package.as_os_str(),
        fixture.as_os_str(),
        first_step.as_os_str(),
        "--step".as_ref(),
        "1".as_ref(),
    ]);
    assert!(output.status.success(), "{output:?}");
    assert!(
        std::fs::read(&first_step).unwrap() == shots[0],
        "an activation doesn't change the panel"
    );

    let main = package.join("main.js");
    let source = std::fs::read_to_string(&main).unwrap();
    std::fs::write(&main, source.replace("`Hello, ${name}!`", "`Hi, ${name}!`")).unwrap();
    let failed = dupin(&["ext".as_ref(), "test".as_ref(), package.as_os_str()]);
    assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    let text = String::from_utf8(failed.stdout).unwrap();
    assert!(
        text.contains("    rows[0].title: expected \"Hello, Dupin!\", got \"Hi, Dupin!\"\n"),
        "{text}"
    );

    let missing = dupin(&[
        "ext".as_ref(),
        "snap".as_ref(),
        package.as_os_str(),
        fixture.as_os_str(),
        Path::new("unused.png").as_os_str(),
        "--step".as_ref(),
        "3".as_ref(),
        "--json".as_ref(),
    ]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(json(&missing)["diagnostic"]["code"], "invalid-step");
}
