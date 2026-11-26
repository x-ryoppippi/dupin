//! `dupin ext test` and `dupin ext render` as an author runs them: the built executable, which
//! starts itself as the extension host and, for `render`, as a headless launcher.
#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

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
fn test_replays_fixtures_and_render_draws_the_same_bytes_every_time() {
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
                "render".as_ref(),
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
            "render",
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
        "render".as_ref(),
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
fn render_draws_every_bundled_fixture_as_test_replays_it() {
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
    assert!(fixtures.len() > 9, "every bundled package has fixtures");
    // A render spends most of its time in headless launchers pacing keys to the wall clock, so
    // the renders run side by side, as many at once as there are cores, the most steps first so
    // the longest doesn't start last.
    let steps = |fixture: &Path| {
        let stated: Value = serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
        stated["steps"].as_array().map_or(0, Vec::len)
    };
    fixtures.sort_by_cached_key(|fixture| (std::cmp::Reverse(steps(fixture)), fixture.clone()));
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    let mut wrong: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers.min(fixtures.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut wrong = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(fixture) = fixtures.get(index) else {
                            return wrong;
                        };
                        let png = root.path().join(format!("panel-{index}.png"));
                        wrong.extend(rendered_wrong(&bundled, fixture, &png));
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });
    wrong.sort();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// What is wrong with the panel `fixture` renders to `png`, if anything: a failed render, the row
/// Dupin shows while the package isn't available, or no first row the fixture expects there.
fn rendered_wrong(bundled: &Path, fixture: &Path, png: &Path) -> Option<String> {
    let package = fixture.parent().unwrap().parent().unwrap();
    let output = dupin(&[
        "ext".as_ref(),
        "render".as_ref(),
        package.as_os_str(),
        fixture.as_os_str(),
        png.as_os_str(),
        "--json".as_ref(),
    ]);
    let name = fixture.strip_prefix(bundled).unwrap().display().to_string();
    if !output.status.success() {
        return Some(format!("{name}: {}", String::from_utf8_lossy(&output.stdout)));
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
    (unavailable || missing).then(|| format!("{name}: expected {expected:?}, shows {shown:?}"))
}

#[test]
fn render_opens_views_and_fills_forms_as_the_fixture_does() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/notes-js");
    let fixture = package.join("fixtures/notes.json");
    let root = tempfile::tempdir().unwrap();
    let render = |step: &str, run: usize| {
        let png = root.path().join(format!("step-{step}-{run}.png"));
        let output = dupin(&[
            "ext".as_ref(),
            "render".as_ref(),
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
    let (list, list_png) = render("1", 0);
    let (form, form_png) = render("4", 0);
    let (detail, _) = render("12", 0);
    assert!(
        form > list,
        "the filled form is taller than the list it opened from"
    );
    assert_ne!(detail, form, "the detail the note opened isn't the form");
    assert!(list_png != form_png);
    assert!(render("4", 1).1 == form_png, "two renders of one form differ");
}

/// A row's freshness is written by the fixture's clock and its state as a label, both by the
/// production panel, and an action that enters a command says so to `test`, which refuses one
/// that would hand another package its text.
#[test]
fn render_writes_freshness_and_state_and_test_records_an_entered_command() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("status");
    std::fs::create_dir_all(package.join("fixtures")).unwrap();
    std::fs::write(
        package.join("package.json"),
        r#"{ "name": "status", "version": "0.1.0", "type": "module",
             "dupin": { "title": "Status", "keyword": "st" } }"#,
    )
    .unwrap();
    std::fs::write(
        package.join("main.js"),
        r#"export const search = () => [
               { title: 'api', accessories: [{ freshness: Date.now() - 12000 }, { state: 'offline' }],
                 actions: [{ title: 'Show Port', run: 'port', data: 3000 }, { title: 'Narrow', run: 'narrow' }] },
               { title: 'cache', accessories: [{ freshness: Date.now() - 300000 }, { state: 'stale' }] },
           ];
           export const actions = {
               port: (port) => ({ type: 'enter', command: 'port', query: String(port) }),
               narrow: () => ({ type: 'enter', command: 'status', query: 'api' }),
           };"#,
    )
    .unwrap();
    let fixture = package.join("fixtures/status.json");
    std::fs::write(
        &fixture,
        r#"{ "fixture": 1, "steps": [
               { "query": "x", "expect": { "selected": 0 } },
               { "activate": "Narrow", "expect": { "panel": { "enter": { "command": "status", "query": "api" } } } },
               { "query": "x" },
               { "activate": "Show Port", "expect": { "panel": null } }
           ] }"#,
    )
    .unwrap();

    let tested = dupin(&[
        "ext".as_ref(),
        "test".as_ref(),
        package.as_os_str(),
        "--json".as_ref(),
    ]);
    assert!(tested.status.success(), "{tested:?}");
    let report = json(&tested);
    assert_eq!(report["passed"], true, "{report}");
    let refused = &report["fixtures"][0]["steps"][3]["outcome"]["error"];
    assert_eq!(refused["code"], "invalid-effect", "{report}");

    let png = root.path().join("status.png");
    let output = dupin(&[
        "ext".as_ref(),
        "render".as_ref(),
        package.as_os_str(),
        fixture.as_os_str(),
        png.as_os_str(),
        "--step".as_ref(),
        "1".as_ref(),
        "--json".as_ref(),
    ]);
    assert!(output.status.success(), "{output:?}");
    let shown: Vec<String> = serde_json::from_value(json(&output)["shown"].clone()).unwrap();
    for text in ["12 s ago", "Offline", "5 min ago", "Stale"] {
        assert!(shown.iter().any(|shown| shown == text), "{text} in {shown:?}");
    }
}
