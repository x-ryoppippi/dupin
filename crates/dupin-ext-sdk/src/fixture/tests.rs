use serde_json::json;

use super::*;

fn fixture(value: Value) -> Result<Fixture, Diagnostic> {
    parse("fixtures/test.json", value.to_string().as_bytes())
}

fn minimal() -> Value {
    json!({ "fixture": 1, "steps": [{ "query": "x" }] })
}

#[test]
fn the_template_is_what_update_writes_and_fills_every_default() {
    let template = include_str!("../../templates/fixture.json");
    let parsed = parse("fixtures/search.json", template.as_bytes()).unwrap();
    assert_eq!(to_json(&parsed), template);
    assert_eq!(parsed.environment, Environment::default());
    assert_eq!(fixture(minimal()).unwrap().environment, Environment::default());
}

#[test]
fn malformed_fixtures_name_the_field_at_fault() {
    let cases = [
        (
            json!({ "fixture": 2, "steps": [{ "query": "x" }] }),
            Some("fixture"),
        ),
        (
            json!({ "fixture": 1, "environment": { "timeZone": "../../etc/passwd" }, "steps": [{ "query": "x" }] }),
            Some("environment.timeZone"),
        ),
        (
            json!({ "fixture": 1, "environment": { "now": "2026-02-30T00:00:00Z" }, "steps": [{ "query": "x" }] }),
            Some("environment.now"),
        ),
        (
            json!({ "fixture": 1, "environment": { "locale": "en_GB" }, "steps": [{ "query": "x" }] }),
            Some("environment.locale"),
        ),
        (
            json!({ "fixture": 1, "environment": { "window": { "width": 10 } }, "steps": [{ "query": "x" }] }),
            Some("environment.window.width"),
        ),
        (json!({ "fixture": 1, "steps": [] }), Some("steps")),
        (
            json!({ "fixture": 1, "steps": [{ "select": 0 }] }),
            Some("steps[0]"),
        ),
        (
            json!({ "fixture": 1, "steps": [{ "query": "x" }, { "query": "y", "select": 1 }] }),
            Some("steps[1]"),
        ),
        (
            json!({ "fixture": 1, "steps": [{ "query": "x", "expect": { "rowz": [] } }] }),
            Some("steps[0]"),
        ),
        (
            json!({ "fixture": 1, "steps": [{ "query": "x\ny" }] }),
            Some("steps[0]"),
        ),
        (
            json!({ "fixture": 1, "steps": [{ "query": "x" }], "script": "rm -rf ~" }),
            None,
        ),
        (
            json!({ "fixture": 1, "replies": { "network": [] }, "steps": [{ "query": "x" }] }),
            None,
        ),
    ];
    for (value, field) in cases {
        let diagnostic = fixture(value.clone()).unwrap_err();
        assert_eq!(diagnostic.code, "invalid-fixture", "{value}");
        assert_eq!(diagnostic.file.as_deref(), Some("fixtures/test.json"));
        assert_eq!(diagnostic.field.as_deref(), field, "{value}: {diagnostic:?}");
        if field.is_none() {
            assert!(diagnostic.line.is_some(), "{diagnostic:?}");
        }
    }
    let huge = vec![b' '; MAX_FIXTURE_BYTES + 1];
    assert_eq!(
        parse("fixtures/huge.json", &huge).unwrap_err().code,
        "invalid-fixture"
    );
}

#[test]
fn times_are_utc_milliseconds() {
    assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_time("2026-01-01T09:00:00Z"), Some(1_767_258_000_000));
    assert_eq!(parse_time("2024-02-29T23:59:59.999Z"), Some(1_709_251_199_999));
    assert_eq!(parse_time("1969-12-31T23:59:59Z"), Some(-1000));
    for invalid in [
        "2026-01-01T09:00:00",
        "2026-01-01 09:00:00Z",
        "2026-13-01T00:00:00Z",
        "2023-02-29T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T09:00:00.5Z",
        "2026-01-01T09:00:00+09:00",
    ] {
        assert_eq!(parse_time(invalid), None, "{invalid}");
    }
}

#[test]
fn differences_name_each_path_that_changed() {
    let expected = json!({
        "rows": [{ "title": "Hello", "actions": [{ "title": "Copy", "copy": "Hello" }] }, { "title": "Gone" }],
        "selected": 0,
    });
    let actual = json!({
        "rows": [{ "title": "Hi", "subtitle": "new", "actions": [{ "title": "Copy", "copy": "Hello" }] }],
        "selected": 0,
        "error": null,
    });
    let differences = compare(expected.as_object().unwrap(), actual.as_object().unwrap());
    let lines: Vec<String> = differences.iter().map(ToString::to_string).collect();
    assert_eq!(
        lines,
        [
            r#"rows[0].title: expected "Hello", got "Hi""#,
            r#"rows[0].subtitle: unexpected "new""#,
            r#"rows[1]: expected {"title":"Gone"}, got nothing"#,
        ]
    );
    assert!(compare(actual.as_object().unwrap(), actual.as_object().unwrap()).is_empty());
    assert_eq!(
        compare(
            json!({ "error": { "code": "search-threw" } })
                .as_object()
                .unwrap(),
            &Map::new()
        )[0]
        .to_string(),
        r#"error: expected {"code":"search-threw"}, got nothing"#
    );
}

#[test]
fn replies_come_only_from_the_fixture() {
    let replies: Replies = serde_json::from_value(json!({
        "http": [{
            "request": { "url": "https://api.github.com/repos/x-ryoppippi/dupin" },
            "response": { "json": { "archived": false } },
        }],
        "files": [{ "scope": "notes", "path": "", "entries": [{ "name": "a.md", "kind": "file" }] }],
        "exec": [{ "target": "@script:open-in-zed", "stdout": "opened" }],
        "storage": { "count": 3 },
        "clipboard": { "read": { "change": 1, "content": { "type": "text", "text": "hi" } } },
        "dictionary": [{ "term": "run", "entries": [{ "headword": "run" }] }],
        "calendar": [{ "id": "a", "title": "Review", "start": 100, "end": 200 }],
        "consent": { "http:https://example.com": "deny", "clipboard:read": "pending" },
    }))
    .unwrap();
    let scope = "http:https://api.github.com";
    let get = |url| Request::Http { method: "get", url };
    let answer = replies
        .answer(scope, &get("https://api.github.com/repos/x-ryoppippi/dupin"))
        .unwrap();
    assert_eq!(answer["status"], 200);
    assert_eq!(answer["ok"], true);
    assert_eq!(answer["body"], r#"{"archived":false}"#);

    let missing = replies
        .answer(scope, &get("https://api.github.com/user"))
        .unwrap_err();
    assert_eq!(missing.code, "unavailable");
    assert_eq!(
        missing.message,
        "the fixture has no reply for GET https://api.github.com/user"
    );
    assert_eq!(
        replies
            .answer(
                scope,
                &Request::Http {
                    method: "POST",
                    url: "https://api.github.com/repos/x-ryoppippi/dupin"
                }
            )
            .unwrap_err()
            .code,
        "unavailable"
    );
    assert_eq!(
        replies
            .answer("http:https://example.com", &get("https://example.com/"))
            .unwrap_err()
            .code,
        "denied"
    );
    assert_eq!(
        replies
            .answer("clipboard:read", &Request::ClipboardRead)
            .unwrap_err()
            .code,
        "needs-consent"
    );

    assert_eq!(
        replies
            .answer(
                "files:notes",
                &Request::FilesList {
                    scope: "notes",
                    path: ""
                }
            )
            .unwrap()[0]["name"],
        "a.md"
    );
    assert_eq!(
        replies
            .answer(
                "files:notes",
                &Request::FilesRead {
                    scope: "notes",
                    path: ""
                }
            )
            .unwrap_err()
            .code,
        "unavailable"
    );
    let args = ["--flag".to_owned()];
    assert_eq!(
        replies
            .answer(
                "exec:@script:open-in-zed",
                &Request::Exec {
                    target: "@script:open-in-zed",
                    args: &args
                }
            )
            .unwrap()["stdout"],
        "opened"
    );
    assert_eq!(
        replies
            .answer("storage", &Request::StorageGet { key: "count" })
            .unwrap(),
        3
    );
    assert_eq!(
        replies
            .answer("storage", &Request::StorageGet { key: "other" })
            .unwrap(),
        Value::Null
    );
    assert_eq!(
        replies
            .answer("dictionary:lookup", &Request::DictionaryLookup { term: "run" })
            .unwrap()[0]["headword"],
        "run"
    );
    assert_eq!(
        replies
            .answer("calendar:read", &Request::CalendarRead { start: 150, end: 300 })
            .unwrap()[0]["id"],
        "a"
    );
    assert_eq!(
        replies
            .answer("calendar:read", &Request::CalendarRead { start: 200, end: 300 })
            .unwrap(),
        json!([])
    );
    assert_eq!(
        Replies::default()
            .answer("clipboard:read", &Request::ClipboardRead)
            .unwrap_err()
            .code,
        "unavailable"
    );
}

#[test]
fn discovery_refuses_links_and_needs_a_fixture() {
    let package = tempfile::tempdir().unwrap();
    assert_eq!(discover(package.path()).unwrap_err().code, "no-fixtures");
    let folder = package.path().join(FOLDER);
    std::fs::create_dir(&folder).unwrap();
    assert_eq!(discover(package.path()).unwrap_err().code, "no-fixtures");
    std::fs::write(folder.join("b.json"), "{}").unwrap();
    std::fs::write(folder.join("a.json"), "{}").unwrap();
    std::fs::write(folder.join("notes.txt"), "").unwrap();
    let names: Vec<String> = discover(package.path())
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["fixtures/a.json", "fixtures/b.json"]);

    let outside = tempfile::NamedTempFile::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), folder.join("c.json")).unwrap();
    let diagnostic = discover(package.path()).unwrap_err();
    assert_eq!(
        (diagnostic.code.as_ref(), diagnostic.file.as_deref()),
        ("invalid-fixture", Some("fixtures/c.json"))
    );

    let linked = tempfile::tempdir().unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
    std::os::unix::fs::symlink(linked.path(), &folder).unwrap();
    assert_eq!(discover(package.path()).unwrap_err().code, "fixtures-unreadable");
}
