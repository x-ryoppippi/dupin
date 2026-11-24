use std::fs;

use serde_json::json;

use super::*;

fn scaffolded(template: dupin_ext_sdk::Template) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("greeter");
    dupin_ext_sdk::scaffold(&package, template).unwrap();
    (root, package)
}

fn write_fixture(package: &Path, name: &str, fixture: Value) {
    fs::write(package.join("fixtures").join(name), fixture.to_string()).unwrap();
}

fn steps(report: &Report, fixture: usize) -> &Vec<Value> {
    report.json["fixtures"][fixture]["steps"].as_array().unwrap()
}

#[test]
fn a_scaffolded_package_passes_its_own_fixture() {
    dupin_extension_host::skip_inside_a_sandbox!();
    for template in [
        dupin_ext_sdk::Template::JavaScript,
        dupin_ext_sdk::Template::TypeScript,
    ] {
        let (_root, package) = scaffolded(template);
        let report = test(&package, false).unwrap();
        assert!(report.passed, "{}", report.text);
        assert_eq!(report.json["package"], "greeter");
        assert_eq!(
            report.text,
            "ok fixtures/search.json\n1 fixture: 1 passed, 0 failed\n"
        );
        let steps = steps(&report, 0);
        assert_eq!(steps[0]["action"], r#"query "Dupin""#);
        assert_eq!(
            steps[1]["outcome"],
            json!({ "effect": { "copy": "Hello, Dupin!" } })
        );
    }
}

#[test]
fn a_mismatch_fails_with_the_path_and_both_values_and_update_rewrites_it() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_root, package) = scaffolded(dupin_ext_sdk::Template::JavaScript);
    let main = package.join("main.js");
    let source = fs::read_to_string(&main).unwrap();
    fs::write(&main, source.replace("`Hello, ${name}!`", "`Hi, ${name}!`")).unwrap();

    let report = test(&package, false).unwrap();
    assert!(!report.passed);
    assert_eq!(
        report.text,
        concat!(
            "FAIL fixtures/search.json\n",
            "  step 1, query \"Dupin\":\n",
            "    rows[0].actions[0].copy: expected \"Hello, Dupin!\", got \"Hi, Dupin!\"\n",
            "    rows[0].title: expected \"Hello, Dupin!\", got \"Hi, Dupin!\"\n",
            "  step 2, activate \"Copy\":\n",
            "    effect.copy: expected \"Hello, Dupin!\", got \"Hi, Dupin!\"\n",
            "1 fixture: 0 passed, 1 failed\n",
        )
    );
    assert_eq!(
        steps(&report, 0)[0]["differences"][1],
        json!({ "path": "rows[0].title", "expected": "Hello, Dupin!", "actual": "Hi, Dupin!" })
    );

    let fixture = package.join("fixtures/search.json");
    let updated = test(&package, true).unwrap();
    assert!(updated.passed, "{}", updated.text);
    assert_eq!(updated.json["fixtures"][0]["updated"], true);
    assert!(fs::read_to_string(&fixture).unwrap().contains("\"Hi, Dupin!\""));
    assert!(test(&package, false).unwrap().passed);
    let again = test(&package, true).unwrap();
    assert_eq!(again.json["fixtures"][0]["updated"], false);
}

#[test]
fn errors_steps_that_cannot_run_and_broken_fixtures_are_reported_apart() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_root, package) = scaffolded(dupin_ext_sdk::Template::JavaScript);
    let main = package.join("main.js");
    let source = fs::read_to_string(&main).unwrap();
    fs::write(
        &main,
        source.replace(
            "if (!name) return [];",
            "if (!name) return [];\n\tif (name === 'boom') throw new Error('boom');",
        ),
    )
    .unwrap();
    write_fixture(
        &package,
        "boom.json",
        json!({ "fixture": 1, "steps": [{ "query": "boom", "expect": { "rows": [], "error": { "code": "search-threw" } } }] }),
    );
    write_fixture(
        &package,
        "missing-row.json",
        json!({ "fixture": 1, "steps": [{ "query": "x" }, { "select": 3 }, { "activate": "Copy" }] }),
    );
    write_fixture(
        &package,
        "unknown.json",
        json!({ "fixture": 1, "steps": [{ "type": "x" }] }),
    );

    let report = test(&package, true).unwrap();
    assert!(!report.passed);
    let fixtures = report.json["fixtures"].as_array().unwrap();
    let names: Vec<&str> = fixtures.iter().map(|f| f["file"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "fixtures/boom.json",
            "fixtures/missing-row.json",
            "fixtures/search.json",
            "fixtures/unknown.json"
        ]
    );

    let boom = &fixtures[0]["steps"][0]["outcome"]["error"];
    assert_eq!(boom["code"], "search-threw");
    assert_eq!(
        (boom["file"].as_str(), boom["line"].as_u64()),
        (Some("main.js"), Some(10))
    );

    assert_eq!(fixtures[1]["passed"], false);
    assert_eq!(fixtures[1]["updated"], false);
    let missing = fixtures[1]["steps"].as_array().unwrap();
    assert_eq!(missing.len(), 2, "replay stops at the step that can't run");
    assert_eq!(
        missing[1]["problem"],
        "there is no row 3 to select; the answer has 1 row"
    );

    assert_eq!(fixtures[3]["diagnostic"]["code"], "invalid-fixture");
    assert!(
        report
            .text
            .contains("FAIL fixtures/unknown.json\n  fixtures/unknown.json:"),
        "{}",
        report.text
    );
    assert!(
        report.text.ends_with("4 fixtures: 2 passed, 2 failed\n"),
        "{}",
        report.text
    );
}

#[test]
fn the_fixture_fixes_the_clock_seed_and_time_zone() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_root, package) = scaffolded(dupin_ext_sdk::Template::JavaScript);
    fs::write(
        package.join("main.js"),
        "export function search() {\n\tconst now = new Date();\n\treturn [{ title: `${now.toISOString()} ${now.getHours()}:${now.getMinutes()} ${Math.random()}` }];\n}\n",
    )
    .unwrap();
    fs::remove_file(package.join("fixtures/search.json")).unwrap();
    let fixture = |zone: &str, seed: u32| {
        json!({
            "fixture": 1,
            "environment": { "now": "2026-06-01T12:30:00Z", "seed": seed, "timeZone": zone },
            "steps": [{ "query": "now" }],
        })
    };
    let title = |report: &Report| {
        steps(report, 0)[0]["outcome"]["rows"][0]["title"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    write_fixture(&package, "clock.json", fixture("Asia/Tokyo", 1));
    let tokyo = title(&test(&package, false).unwrap());
    assert!(tokyo.starts_with("2026-06-01T12:30:00.000Z 21:30 0."), "{tokyo}");
    assert_eq!(tokyo, title(&test(&package, false).unwrap()));
    write_fixture(&package, "clock.json", fixture("Europe/London", 1));
    let london = title(&test(&package, false).unwrap());
    assert!(london.starts_with("2026-06-01T12:30:00.000Z 13:30 "), "{london}");
    assert_eq!(tokyo.rsplit(' ').next(), london.rsplit(' ').next());
    write_fixture(&package, "clock.json", fixture("Europe/London", 2));
    assert_ne!(london, title(&test(&package, false).unwrap()));

    write_fixture(&package, "clock.json", fixture("Nowhere/Atlantis", 1));
    let report = test(&package, false).unwrap();
    assert_eq!(
        report.json["fixtures"][0]["diagnostic"]["field"],
        "environment.timeZone"
    );
}

/// A package whose `package.json` declares `capabilities`, with `main.js`.
fn capable(capabilities: Value, main: &str) -> (tempfile::TempDir, PathBuf) {
    let (root, package) = scaffolded(dupin_ext_sdk::Template::JavaScript);
    let manifest = package.join("package.json");
    let mut json: Value = serde_json::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    json["dupin"]["capabilities"] = capabilities;
    fs::write(&manifest, json.to_string()).unwrap();
    fs::write(package.join("main.js"), main).unwrap();
    fs::remove_file(package.join("fixtures/search.json")).unwrap();
    (root, package)
}

fn only_title(report: &Report) -> String {
    let steps = steps(report, 0);
    assert!(
        report.passed || steps.iter().all(|step| step["problem"].is_null()),
        "{}",
        report.text
    );
    steps[0]["outcome"]["rows"][0]["title"]
        .as_str()
        .unwrap_or_else(|| panic!("{}", report.text))
        .to_owned()
}

#[test]
fn host_requests_are_answered_from_the_fixture_through_the_broker() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_root, package) = capable(
        json!({
            "http": [{ "origin": "https://example.com", "methods": ["GET"] }],
            "files": { "notes": { "kind": "directory", "access": ["read", "list"] } },
            "dictionary": ["lookup"],
            "clipboard": ["read"],
        }),
        "import { http, files, dictionary, clipboard } from 'dupin';\n\
         const settle = async (work) => { try { return String(await work()); } catch (error) { return error.code; } };\n\
         export async function search() {\n\
         \tconst reached = ['fetch', 'XMLHttpRequest', 'WebSocket', 'require', 'process', 'std', 'os', 'scriptArgs', 'setTimeout']\n\
         \t\t.filter((name) => typeof globalThis[name] !== 'undefined');\n\
         \tconst parts = [\n\
         \t\treached.join(',') || 'nothing',\n\
         \t\tawait settle(async () => (await http.fetch('https://example.com/')).text()),\n\
         \t\tawait settle(async () => (await http.fetch('https://example.com/elsewhere')).text()),\n\
         \t\tawait settle(async () => (await http.fetch('https://api.github.com/')).text()),\n\
         \t\tawait settle(async () => (await http.fetch('https://example.com/', { method: 'POST' })).text()),\n\
         \t\tawait settle(() => files.readText('notes', 'today.md')),\n\
         \t\tawait settle(async () => (await files.list('notes')).map((entry) => entry.name).join('+')),\n\
         \t\tawait settle(() => files.readText('notes', '../outside.md')),\n\
         \t\tawait settle(async () => (await dictionary.lookup('run'))[0].headword),\n\
         \t\tawait settle(async () => (await dictionary.lookup('walk')).length),\n\
         \t\tawait settle(() => clipboard.readText()),\n\
         \t];\n\
         \treturn [{ title: parts.join(' | ') }];\n\
         }\n",
    );
    write_fixture(
        &package,
        "services.json",
        json!({
            "fixture": 1,
            "replies": {
                "http": [{ "request": { "url": "https://example.com/" }, "response": { "body": "from the fixture" } }],
                "files": { "notes": { "today.md": "# Today", "archive/old.md": "old" } },
                "dictionary": { "run": [{ "headword": "run" }] },
                "clipboard": { "type": "text", "text": "copied" },
            },
            "steps": [{ "query": "x" }],
        }),
    );
    let title = only_title(&test(&package, false).unwrap());
    // The fixture answers what it has a reply for and the package declared; a URL it has no
    // reply for fails instead of reaching the network, and reading the clipboard needs the user
    // to choose an action, so typing never reads it.
    assert_eq!(
        title.split(" | ").collect::<Vec<_>>(),
        [
            "nothing",
            "from the fixture",
            "network",
            "undeclared",
            "out-of-scope",
            "# Today",
            "archive+today.md",
            "invalid",
            "run",
            "unavailable",
            "needs-activation",
        ]
    );

    write_fixture(
        &package,
        "services.json",
        json!({
            "fixture": 1,
            "replies": { "consent": { "http:https://example.com": "deny", "files:notes": "pending" } },
            "steps": [{ "query": "x" }],
        }),
    );
    let denied = only_title(&test(&package, false).unwrap());
    assert_eq!(
        denied.split(" | ").collect::<Vec<_>>(),
        [
            "nothing",
            "revoked",
            "revoked",
            "undeclared",
            "out-of-scope",
            "needs-consent",
            "needs-consent",
            "invalid",
            "unavailable",
            "unavailable",
            "needs-activation",
        ]
    );

    write_fixture(
        &package,
        "services.json",
        json!({ "fixture": 1, "replies": { "consent": { "exec:anything": "allow" } }, "steps": [{ "query": "x" }] }),
    );
    let report = test(&package, false).unwrap();
    assert_eq!(
        report.json["fixtures"][0]["diagnostic"]["field"],
        "replies.consent.exec:anything"
    );
}

#[test]
fn actions_events_storage_and_the_clipboard_are_replayed_and_recorded() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_root, package) = capable(
        json!({ "storage": { "quotaBytes": 4096 }, "clipboard": ["write", "watch"] }),
        "import { storage, clipboard } from 'dupin';\n\
         export async function search() {\n\
         \tconst count = (await storage.get('count')) ?? 0;\n\
         \tconst last = (await storage.get('last')) ?? 'none';\n\
         \treturn [{ title: `count ${count}, last ${last}`, actions: [{ title: 'Bump', run: 'bump', data: { by: 2 } }] }];\n\
         }\n\
         export const actions = {\n\
         \tasync bump(data) {\n\
         \t\tconst count = ((await storage.get('count')) ?? 0) + data.by;\n\
         \t\tawait storage.set('count', count);\n\
         \t\tawait clipboard.writeText(String(count));\n\
         \t\treturn { type: 'toast', text: `now ${count}` };\n\
         \t},\n\
         };\n\
         export const events = {\n\
         \tasync clipboard(event) {\n\
         \t\tawait storage.set('last', event.text);\n\
         \t},\n\
         };\n",
    );
    write_fixture(
        &package,
        "actions.json",
        json!({
            "fixture": 1,
            "replies": { "storage": { "count": 1 } },
            "steps": [
                { "query": "x" },
                { "activate": "Bump" },
                { "event": "clipboard", "data": { "type": "text", "text": "elsewhere" } },
                { "query": "x" },
            ],
        }),
    );
    let report = test(&package, false).unwrap();
    assert!(report.passed, "{}", report.text);
    let steps = steps(&report, 0);
    assert_eq!(steps[0]["outcome"]["rows"][0]["title"], "count 1, last none");
    assert_eq!(
        steps[1]["outcome"],
        json!({
            "effect": { "run": { "action": "bump", "data": { "by": 2 } } },
            "toasts": ["now 3"],
            "clipboard": ["3"],
            "storage": { "count": 3 },
        })
    );
    assert_eq!(
        steps[2]["outcome"],
        json!({ "storage": { "count": 3, "last": "elsewhere" } })
    );
    assert_eq!(steps[3]["outcome"]["rows"][0]["title"], "count 3, last elsewhere");
}

#[test]
fn an_empty_clipboard_reply_reads_as_empty_and_a_missing_one_fails() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_root, package) = capable(
        json!({ "clipboard": ["read"] }),
        "import { clipboard } from 'dupin';\n\
         export function search() {\n\
         \treturn [{ title: 'Paste', actions: [{ title: 'Paste', run: 'paste' }] }];\n\
         }\n\
         export const actions = {\n\
         \tasync paste() {\n\
         \t\ttry { return { type: 'toast', text: (await clipboard.readText()) ?? 'empty' }; }\n\
         \t\tcatch (error) { return { type: 'toast', text: error.code }; }\n\
         \t},\n\
         };\n",
    );
    for (replies, toast) in [
        (json!({ "clipboard": null }), "empty"),
        (
            json!({ "clipboard": { "type": "text", "text": "copied" } }),
            "copied",
        ),
        (json!({}), "unavailable"),
    ] {
        write_fixture(
            &package,
            "paste.json",
            json!({ "fixture": 1, "replies": replies, "steps": [{ "query": "x" }, { "activate": "Paste" }] }),
        );
        let report = test(&package, false).unwrap();
        assert_eq!(
            steps(&report, 0)[1]["outcome"]["toasts"],
            json!([toast]),
            "{}",
            report.text
        );
    }
}

#[test]
fn a_linked_fixture_is_refused() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_root, package) = scaffolded(dupin_ext_sdk::Template::JavaScript);
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), r#"{ "fixture": 1, "steps": [{ "query": "x" }] }"#).unwrap();
    std::os::unix::fs::symlink(outside.path(), package.join("fixtures/linked.json")).unwrap();
    match test(&package, false) {
        Err(diagnostic) => assert_eq!(
            (diagnostic.code.as_ref(), diagnostic.file.as_deref()),
            ("invalid-fixture", Some("fixtures/linked.json"))
        ),
        Ok(report) => panic!("a linked fixture ran: {}", report.text),
    }
}

#[test]
fn snap_scripts_type_each_query_and_move_to_each_selection() {
    let fixture = dupin_ext_sdk::fixture::parse(
        "fixtures/x.json",
        json!({
            "fixture": 1,
            "steps": [{ "query": "a" }, { "select": 2 }, { "activate": "Copy" }, { "query": "b c" }, { "select": 1 }, { "select": 0 }],
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let events = script_for("kw", &fixture, 6)["events"].clone();
    let kinds: Vec<String> = events
        .as_array()
        .unwrap()
        .iter()
        .map(|event| match event["type"].as_str().unwrap() {
            "type" => format!("type {}", event["text"].as_str().unwrap()),
            "key" if event["cmd"] == true => format!("cmd+{}", event["name"].as_str().unwrap()),
            "key" => event["name"].as_str().unwrap().to_owned(),
            other => other.to_owned(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "type kw a",
            "Down",
            "Down",
            "cmd+a",
            "type kw b c",
            "Down",
            "Up",
            "mark"
        ]
    );
    let times: Vec<f64> = events
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["t"].as_f64().unwrap())
        .collect();
    assert!(times.windows(2).all(|pair| pair[0] < pair[1]), "{times:?}");
    assert_eq!(
        script_for("kw", &fixture, 1)["events"].as_array().unwrap().len(),
        2
    );
}
