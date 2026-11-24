use super::*;

#[test]
fn every_capability_parses_into_its_scopes() {
    let parsed = everything();
    let scopes: Vec<_> = parsed.scopes().cloned().collect();
    assert_eq!(
        scopes,
        [
            http("https://api.github.com", &[Method::Get, Method::Post]),
            Scope::Files {
                name: name("notes"),
                kind: FilesKind::Directory,
                access: [FilesAccess::Read, FilesAccess::List].into(),
            },
            Scope::Exec {
                name: name("git-status"),
                args: vec!["status".into(), "--short".into(), "{path}".into()],
                output: true,
            },
            Scope::Script {
                name: name("open-in-zed"),
                output: false,
            },
            Scope::Storage {
                quota_bytes: 1024,
                retention_days: Some(30),
            },
            Scope::Clipboard {
                access: ClipboardAccess::Read,
            },
            Scope::Clipboard {
                access: ClipboardAccess::Write,
            },
            Scope::Clipboard {
                access: ClipboardAccess::Watch,
            },
            Scope::Dictionary,
            Scope::Calendar {
                past_days: 1,
                future_days: 30,
            },
        ]
    );
    assert!(Capabilities::parse(&json!({})).unwrap().is_empty());
}

fn problems(value: serde_json::Value) -> Vec<(&'static str, String)> {
    Capabilities::parse(&value)
        .expect_err("expected diagnostics")
        .into_iter()
        .map(|diagnostic| (diagnostic.code, diagnostic.field))
        .collect()
}

fn problem(value: serde_json::Value) -> (&'static str, String) {
    let mut all = problems(value);
    assert_eq!(all.len(), 1, "{all:?}");
    all.remove(0)
}

#[test]
fn the_declaration_must_be_an_object_of_known_capabilities() {
    assert_eq!(problem(json!([])), ("invalid-type", "capabilities".into()));
    assert_eq!(
        problem(json!({ "network": true })),
        ("unknown-capability", "capabilities.network".into())
    );
}

#[test]
fn http_origins_must_be_canonical_https_origins() {
    let field = "capabilities.http[0].origin".to_string();
    for bad in [
        "http://api.github.com",
        "https://api.github.com/",
        "https://api.github.com/repos",
        "https://API.github.com",
        "https://api.github.com:443",
        "https://user@api.github.com",
        "https://[::1]",
        "https://api.github.com:0",
        "https://-bad.example",
        "https://",
    ] {
        assert_eq!(
            problem(json!({ "http": [{ "origin": bad, "methods": ["GET"] }] })),
            ("invalid-origin", field.clone()),
            "{bad}"
        );
    }
    let parsed = caps(json!({ "http": [{ "origin": "https://localhost:8443", "methods": ["GET"] }] }));
    assert!(parsed.get(&key("http:https://localhost:8443")).is_some());
}

#[test]
fn http_entries_need_known_distinct_methods_and_distinct_origins() {
    assert_eq!(
        problem(json!({ "http": [{ "origin": "https://a.example", "methods": ["get"] }] })),
        ("invalid-value", "capabilities.http[0].methods[0]".into())
    );
    assert_eq!(
        problem(json!({ "http": [{ "origin": "https://a.example", "methods": ["GET", "GET"] }] })),
        ("duplicate", "capabilities.http[0].methods[1]".into())
    );
    assert_eq!(
        problem(json!({ "http": [{ "origin": "https://a.example", "methods": [] }] })),
        ("empty", "capabilities.http[0].methods".into())
    );
    assert_eq!(
        problem(json!({ "http": [
            { "origin": "https://a.example", "methods": ["GET"] },
            { "origin": "https://a.example", "methods": ["POST"] }
        ] })),
        ("duplicate", "capabilities.http[1].origin".into())
    );
    assert_eq!(
        problems(json!({ "http": [{ "origin": "https://a.example", "headers": {} }] })),
        [
            ("unknown-field", "capabilities.http[0].headers".into()),
            ("missing-field", "capabilities.http[0].methods".into()),
        ]
    );
    assert_eq!(
        problem(json!({ "http": [] })),
        ("empty", "capabilities.http".into())
    );
    let many: Vec<_> = (0..33)
        .map(|i| json!({ "origin": format!("https://h{i}.example"), "methods": ["GET"] }))
        .collect();
    assert_eq!(
        problem(json!({ "http": many })),
        ("too-many", "capabilities.http".into())
    );
}

#[test]
fn files_scopes_are_named_read_or_list_and_never_write() {
    assert_eq!(
        problem(json!({ "files": { "notes": { "kind": "directory", "access": ["write"] } } })),
        ("unsupported-access", "capabilities.files.notes.access[0]".into())
    );
    assert_eq!(
        problem(json!({ "files": { "notes": { "kind": "file", "access": ["read", "list"] } } })),
        ("invalid-value", "capabilities.files.notes.access".into())
    );
    assert_eq!(
        problem(json!({ "files": { "notes": { "kind": "socket", "access": ["read"] } } })),
        ("invalid-value", "capabilities.files.notes.kind".into())
    );
    assert_eq!(
        problem(json!({ "files": { "../up": { "kind": "file", "access": ["read"] } } })),
        ("invalid-name", "capabilities.files.../up".into())
    );
    assert_eq!(
        problem(json!({ "files": {} })),
        ("empty", "capabilities.files".into())
    );
}

#[test]
fn exec_profiles_need_well_formed_argument_templates() {
    for bad in ["{path", "path}", "{Path}", "{}", "{1st}"] {
        assert_eq!(
            problem(json!({ "exec": { "run": { "args": [bad] } } })),
            ("invalid-placeholder", "capabilities.exec.run.args[0]".into()),
            "{bad}"
        );
    }
    assert_eq!(
        problem(json!({ "exec": { "run": { "output": true } } })),
        ("missing-field", "capabilities.exec.run.args".into())
    );
    assert_eq!(
        problem(json!({ "exec": { "run": { "args": ["a\u{0}b"] } } })),
        ("invalid-value", "capabilities.exec.run.args[0]".into())
    );
    assert_eq!(
        problem(json!({ "exec": { "@script:open": { "args": [] } } })),
        ("unknown-field", "capabilities.exec.@script:open.args".into())
    );
    assert_eq!(
        problem(json!({ "exec": { "@script:": {} } })),
        ("invalid-name", "capabilities.exec.@script:".into())
    );
    assert_eq!(
        problem(json!({ "exec": { "run": { "args": [], "output": "yes" } } })),
        ("invalid-type", "capabilities.exec.run.output".into())
    );
}

#[test]
fn storage_clipboard_dictionary_and_calendar_are_bounded() {
    assert_eq!(
        problem(json!({ "storage": { "quotaBytes": 0 } })),
        ("out-of-range", "capabilities.storage.quotaBytes".into())
    );
    assert_eq!(
        problem(json!({ "storage": { "quotaBytes": 67_108_865 } })),
        ("out-of-range", "capabilities.storage.quotaBytes".into())
    );
    assert_eq!(
        problem(json!({ "storage": { "quotaBytes": 1.5 } })),
        ("out-of-range", "capabilities.storage.quotaBytes".into())
    );
    assert_eq!(
        problem(json!({ "storage": { "quotaBytes": 1, "retentionDays": 0 } })),
        ("out-of-range", "capabilities.storage.retentionDays".into())
    );
    assert_eq!(
        caps(json!({ "storage": { "quotaBytes": 1 } }))
            .get(&ScopeKey::Storage)
            .cloned(),
        Some(Scope::Storage {
            quota_bytes: 1,
            retention_days: None
        })
    );
    assert_eq!(
        problem(json!({ "clipboard": ["paste"] })),
        ("invalid-value", "capabilities.clipboard[0]".into())
    );
    assert_eq!(
        problem(json!({ "dictionary": ["define"] })),
        ("invalid-value", "capabilities.dictionary[0]".into())
    );
    assert_eq!(
        problem(json!({ "dictionary": true })),
        ("invalid-type", "capabilities.dictionary".into())
    );
    assert_eq!(
        problem(json!({ "calendar": { "access": ["write"], "pastDays": 0, "futureDays": 1 } })),
        ("invalid-value", "capabilities.calendar.access[0]".into())
    );
    assert_eq!(
        problem(json!({ "calendar": { "access": ["read"], "pastDays": 0, "futureDays": 367 } })),
        ("out-of-range", "capabilities.calendar.futureDays".into())
    );
    assert_eq!(
        problem(json!({ "calendar": { "access": ["read"], "pastDays": -1, "futureDays": 1 } })),
        ("out-of-range", "capabilities.calendar.pastDays".into())
    );
}

#[test]
fn every_problem_is_reported_at_once_with_hints_where_useful() {
    let diagnostics = Capabilities::parse(&json!({
        "http": [{ "origin": "http://a.example", "methods": ["GET"] }],
        "clipboard": ["paste"],
        "calendar": { "access": ["read"] }
    }))
    .unwrap_err();
    let mut codes: Vec<_> = diagnostics.iter().map(|d| d.code).collect();
    codes.sort_unstable();
    assert_eq!(
        codes,
        [
            "invalid-origin",
            "invalid-value",
            "missing-field",
            "missing-field"
        ]
    );
    assert!(
        diagnostics
            .iter()
            .find(|d| d.code == "invalid-origin")
            .unwrap()
            .hint
            .is_some()
    );
    let missing = diagnostics.iter().find(|d| d.code == "missing-field").unwrap();
    let encoded = serde_json::to_value(missing).unwrap();
    assert_eq!(
        encoded,
        json!({
            "code": "missing-field",
            "field": "capabilities.calendar.pastDays",
            "message": "`pastDays` is required"
        })
    );
}
