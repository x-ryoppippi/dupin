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
    let output = Command::new(env!("CARGO_BIN_EXE_dupin"))
        .current_dir(root.path())
        .args([
            "ext",
            "snap",
            "greeter",
            "greeter/fixtures/search.json",
            "query.png",
            "--step",
            "1",
        ])
        .output()
        .unwrap();
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

/// Each bundled package renders from its fixtures what `dupin ext test` replays: after a
/// fixture's last step the panel shows the first row the fixture expects there, as the built-in
/// the package ships as shows it, and never the row Dupin shows while that package isn't
/// available.
#[test]
fn snap_renders_every_bundled_fixture_as_test_replays_it() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions");
    let root = tempfile::tempdir().unwrap();
    let mut fixtures: Vec<_> = std::fs::read_dir(&bundled)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("fixtures"))
        .filter(|folder| folder.is_dir())
        .flat_map(|folder| {
            std::fs::read_dir(folder)
                .unwrap()
                .map(|entry| entry.unwrap().path())
        })
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .collect();
    fixtures.sort();
    assert!(fixtures.len() > 9, "every bundled package has fixtures");
    let mut wrong = Vec::new();
    for fixture in &fixtures {
        let package = fixture.parent().unwrap().parent().unwrap();
        let png = root.path().join("panel.png");
        let output = dupin(&[
            "ext".as_ref(),
            "snap".as_ref(),
            package.as_os_str(),
            fixture.as_os_str(),
            png.as_os_str(),
            "--json".as_ref(),
        ]);
        let name = fixture.strip_prefix(&bundled).unwrap().display().to_string();
        if !output.status.success() {
            wrong.push(format!("{name}: {}", String::from_utf8_lossy(&output.stdout)));
            continue;
        }
        let shown: Vec<String> = serde_json::from_value(json(&output)["shown"].clone()).unwrap();
        let stated: Value = serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
        // What the last step that lists rows expects; an activation leaves the panel as it was.
        let expected = stated["steps"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find_map(|step| step["expect"]["rows"].as_array())
            .and_then(|rows| rows.first())
            .and_then(|row| row["title"].as_str());
        let unavailable = shown.iter().any(|text| text.ends_with("isn't available"));
        let missing = expected.is_some_and(|title| !shown.iter().any(|text| text.contains(title)));
        if unavailable || missing {
            wrong.push(format!("{name}: expected {expected:?}, shows {shown:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn snap_opens_views_and_fills_forms_as_the_fixture_does() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/notes-js");
    let fixture = package.join("fixtures/notes.json");
    let root = tempfile::tempdir().unwrap();
    let snap = |step: &str, run: usize| {
        let png = root.path().join(format!("step-{step}-{run}.png"));
        let output = dupin(&[
            "ext".as_ref(),
            "snap".as_ref(),
            package.as_os_str(),
            fixture.as_os_str(),
            png.as_os_str(),
            "--step".as_ref(),
            step.as_ref(),
            "--json".as_ref(),
        ]);
        assert!(output.status.success(), "{output:?}");
        (
            json(&output)["height"].as_u64().unwrap(),
            std::fs::read(&png).unwrap(),
        )
    };
    let (list, list_png) = snap("1", 0);
    let (form, form_png) = snap("4", 0);
    let (detail, _) = snap("12", 0);
    assert!(
        form > list,
        "the filled form is taller than the list it opened from"
    );
    assert_ne!(detail, form, "the detail the note opened isn't the form");
    assert!(list_png != form_png);
    assert!(snap("4", 1).1 == form_png, "two renders of one form differ");
}
