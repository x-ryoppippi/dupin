use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dupin_plugin_protocol::{Effect, View};

use super::*;

const DEADLINE: Duration = Duration::from_secs(2);

fn timeout(timeout: Duration) -> Control {
    Control {
        deadline: Instant::now() + timeout,
        is_cancelled: Box::new(|| false),
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn load(root: &Path) -> Result<Package, Diagnostic> {
    let sources = Sources::read(root, DEFAULT_ENTRY)?;
    Package::compile(sources, timeout(DEADLINE)).map_err(|error| match error {
        RunError::Failed(diagnostic) => diagnostic,
        RunError::Cancelled => panic!("nothing cancels a test's compile"),
    })
}

fn package(files: &[(&str, &str)]) -> (tempfile::TempDir, Result<Package, Diagnostic>) {
    let folder = tempfile::tempdir().unwrap();
    for (name, source) in files {
        let path = folder.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let loaded = load(folder.path());
    (folder, loaded)
}

fn search(package: &Package, query: &str) -> Result<Vec<PluginItem>, RunError> {
    package.search(query, MAX_ROWS, timeout(DEADLINE))
}

fn failure(result: Result<Vec<PluginItem>, RunError>) -> Diagnostic {
    match result {
        Err(RunError::Failed(diagnostic)) => diagnostic,
        other => panic!("expected a failure, got {other:?}"),
    }
}

fn run(source: &str, query: &str) -> Result<Vec<PluginItem>, RunError> {
    let (_folder, loaded) = package(&[("main.js", source)]);
    search(&loaded.unwrap(), query)
}

#[test]
fn the_word_count_example_counts_words_and_copies_the_count() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/word-count-js");
    let package = load(&root).unwrap();
    package.check(timeout(DEADLINE)).unwrap();
    let rows = search(&package, "two words").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].view,
        View::Text {
            title: "2 words".into(),
            subtitle: String::new()
        }
    );
    assert_eq!(rows[0].actions.len(), 1);
    assert_eq!(rows[0].actions[0].effect, Effect::Copy { text: "2".into() });
    assert!(search(&package, "   ").unwrap().is_empty());
}

#[test]
fn async_search_lists_relative_imports_and_the_host_module_load() {
    let (_folder, loaded) = package(&[
        (
            "main.js",
            r#"import { apiVersion } from 'dupin';
               import { rows } from './lib/rows.js';
               export const search = async (query, ctx) => ({ type: 'list', items: await rows(query, ctx.limit, apiVersion) });"#,
        ),
        (
            "lib/rows.js",
            r#"import { site } from '../site.mjs';
               export async function rows(query, limit, version) {
                 return [
                   { id: 'docs', title: `Docs for ${query}`, subtitle: `v${version}, ${limit} rows`, url: `${site}?q=${encodeURIComponent(query)}`, text: query },
                   { title: 'Plain' },
                 ];
               }"#,
        ),
        ("site.mjs", "export const site = 'https://example.com/search';"),
        ("notes.txt", "not JavaScript"),
    ]);
    let package = loaded.unwrap();
    let mut names: Vec<_> = package
        .modules
        .keys()
        .filter(|name| *name != HOST_MODULE)
        .collect();
    names.sort();
    assert_eq!(names, ["lib/rows.js", "main.js", "site.mjs"]);
    let rows = package.search("a b", 7, timeout(DEADLINE)).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "docs");
    assert_eq!(
        rows[0].view,
        View::Text {
            title: "Docs for a b".into(),
            subtitle: "v1, 7 rows".into()
        }
    );
    let effects: Vec<_> = rows[0].actions.iter().map(|action| &action.effect).collect();
    assert_eq!(
        effects,
        [
            &Effect::OpenUrl {
                url: "https://example.com/search?q=a%20b".into()
            },
            &Effect::Copy { text: "a b".into() },
        ]
    );
    assert_eq!(rows[1].id, "#1");
    assert!(rows[1].actions.is_empty());
}

#[test]
fn the_row_limit_keeps_the_first_rows() {
    let (_folder, loaded) = package(&[(
        "main.js",
        "export function search(query, ctx) { return ['a', 'b', 'c'].map((title) => ({ title: `${title} ${ctx.limit}` })); }",
    )]);
    let rows = loaded.unwrap().search("", 2, timeout(DEADLINE)).unwrap();
    let titles: Vec<_> = rows.iter().map(|row| row.view.clone()).collect();
    assert_eq!(
        titles,
        [
            View::Text {
                title: "a 2".into(),
                subtitle: String::new()
            },
            View::Text {
                title: "b 2".into(),
                subtitle: String::new()
            },
        ]
    );
}

#[test]
fn a_busy_loop_stops_at_the_deadline() {
    let package = load(&fixture("busy-loop")).unwrap();
    let started = Instant::now();
    let diagnostic = failure(package.search("", MAX_ROWS, timeout(Duration::from_millis(100))));
    assert_eq!(diagnostic.code, "deadline");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn native_backed_work_stops_at_the_deadline() {
    for source in [
        "export function search() { /(a+)+$/.test('a'.repeat(40) + 'b'); return []; }",
        "export function search() { Array.from({ length: 200000 }, (_, i) => -i).sort((a, b) => { for (let i = 0; i < 1000; i++); return a - b; }); return []; }",
        "export async function search() { await new Promise((resolve) => { for (;;); }); }",
    ] {
        let (_folder, loaded) = package(&[("main.js", source)]);
        let started = Instant::now();
        let result = loaded
            .unwrap()
            .search("", MAX_ROWS, timeout(Duration::from_millis(100)));
        assert_eq!(failure(result).code, "deadline", "{source}");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{source}: {:?}",
            started.elapsed()
        );
    }
}

#[test]
fn cancelling_stops_a_busy_loop_without_an_error() {
    let package = load(&fixture("busy-loop")).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let canceler = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        flag.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let result = package.search(
        "",
        MAX_ROWS,
        Control {
            deadline: Instant::now() + Duration::from_secs(10),
            is_cancelled: Box::new(move || cancelled.load(Ordering::Acquire)),
        },
    );
    canceler.join().unwrap();
    assert_eq!(result, Err(RunError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn an_allocation_spike_hits_the_heap_limit() {
    let package = load(&fixture("allocation-spike")).unwrap();
    let diagnostic = failure(search(&package, ""));
    assert_eq!(diagnostic.code, "memory-limit", "{diagnostic:?}");
}

#[test]
fn deep_recursion_hits_the_stack_limit() {
    let package = load(&fixture("deep-recursion")).unwrap();
    let diagnostic = failure(search(&package, ""));
    assert_eq!(diagnostic.code, "stack-limit", "{diagnostic:?}");
}

#[test]
fn a_promise_storm_spends_the_job_budget() {
    let package = load(&fixture("promise-storm")).unwrap();
    let diagnostic = failure(search(&package, ""));
    assert_eq!(diagnostic.code, "job-limit", "{diagnostic:?}");
}

#[test]
fn a_promise_that_never_settles_is_reported_at_once() {
    let package = load(&fixture("unsettled-promise")).unwrap();
    let started = Instant::now();
    let diagnostic = failure(search(&package, ""));
    assert_eq!(diagnostic.code, "unsettled", "{diagnostic:?}");
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn oversized_output_is_refused() {
    let package = load(&fixture("oversized-output")).unwrap();
    let diagnostic = failure(search(&package, ""));
    assert_eq!(diagnostic.code, "output-too-large", "{diagnostic:?}");
    assert_eq!(diagnostic.file.as_deref(), Some("main.js"));
    assert!(diagnostic.hint.is_some());
}

#[test]
fn invalid_rows_name_the_field_at_fault() {
    let package = load(&fixture("invalid-rows")).unwrap();
    let diagnostic = failure(search(&package, ""));
    assert_eq!(diagnostic.code, "invalid-field");
    assert_eq!(diagnostic.field.as_deref(), Some("items[1].title"));
    assert_eq!(
        diagnostic.to_string(),
        "main.js: `items[1].title` must be a string, not a number"
    );
    assert_eq!(
        diagnostic.hint.as_deref(),
        Some("Convert values with `String(value)`.")
    );
}

#[test]
fn an_import_outside_the_package_is_refused() {
    let diagnostic = load(&fixture("path-escape")).unwrap_err();
    assert_eq!(diagnostic.code, "import-escape", "{diagnostic:?}");
    assert_eq!(diagnostic.file.as_deref(), Some("main.js"));
    assert_eq!(diagnostic.field.as_deref(), Some("import \"../outside.js\""));

    let (_folder, loaded) = package(&[
        (
            "main.js",
            "export async function search() { await import('./lib/dynamic.js'); return []; }",
        ),
        ("lib/dynamic.js", "await import('../../outside.js');"),
    ]);
    let diagnostic = failure(search(&loaded.unwrap(), ""));
    assert_eq!(diagnostic.code, "import-escape", "{diagnostic:?}");
    assert_eq!(diagnostic.file.as_deref(), Some("lib/dynamic.js"));
}

#[test]
fn a_symlink_out_of_the_package_is_refused_when_loading() {
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("main.js"), "export function search() { return []; }").unwrap();
    symlink(fixture("outside.js"), package.join("outside.js")).unwrap();
    let diagnostic = load(&package).unwrap_err();
    assert_eq!(diagnostic.code, "symlink-escape");
    assert_eq!(diagnostic.file.as_deref(), Some("outside.js"));

    fs::remove_file(package.join("outside.js")).unwrap();
    symlink(root.path(), package.join("parent")).unwrap();
    assert_eq!(load(&package).unwrap_err().code, "symlink-escape");

    fs::remove_file(package.join("parent")).unwrap();
    symlink(&package, package.join("again")).unwrap();
    assert_eq!(load(&package).unwrap_err().code, "symlink-loop");
}

#[test]
fn text_a_guest_chooses_stays_one_short_line_in_diagnostics() {
    let huge = "'x'.repeat(4_000_000) + '\\nnext line\\u001b[31m'";
    for (source, code) in [
        (
            format!("export function search() {{ throw new Error({huge}); }}"),
            "search-threw",
        ),
        (
            format!("export function search() {{ throw {huge}; }}"),
            "search-threw",
        ),
        (
            format!(
                "export function search() {{ const error = new Error('short'); error.name = {huge}; throw error; }}"
            ),
            "search-threw",
        ),
        (
            format!(
                "export function search() {{ const error = new Error('short'); error.stack = '    at f (' + {huge} + ':1:1)'; throw error; }}"
            ),
            "search-threw",
        ),
        (
            format!("throw new Error({huge});\nexport function search() {{}}"),
            "module-threw",
        ),
        (
            format!("export function search() {{ return {{ [{huge}]: 1 }}; }}"),
            "unknown-field",
        ),
    ] {
        let diagnostic = failure(run(&source, ""));
        assert_eq!(diagnostic.code, code, "{diagnostic:?}");
        let line = diagnostic.to_string();
        assert!(line.len() <= 2048, "{code}: {} bytes", line.len());
        assert!(!line.chars().any(char::is_control), "{code}: {line:?}");
    }

    let diagnostic = failure(run(
        "export function search() { throw new Error('first\\nsecond'); }",
        "",
    ));
    assert_eq!(diagnostic.message, "search threw Error: first\\nsecond");
}

#[test]
fn a_throwing_search_reports_the_error_and_where_it_was_thrown() {
    let package = load(&fixture("throwing-search")).unwrap();
    let diagnostic = failure(search(&package, "it"));
    assert_eq!(diagnostic.code, "search-threw");
    assert_eq!(diagnostic.message, "search threw Error: no answer for it");
    assert_eq!(
        (diagnostic.file.as_deref(), diagnostic.line),
        (Some("main.js"), Some(2))
    );
}

#[test]
fn imports_other_than_package_files_and_dupin_are_refused() {
    for (import, code) in [
        ("node:fs", "import-unavailable"),
        ("std", "import-unavailable"),
        ("os", "import-unavailable"),
        ("https://example.com/x.js", "import-unavailable"),
        ("/etc/passwd", "import-unavailable"),
        ("./missing.js", "import-missing"),
        ("./util", "import-missing"),
    ] {
        let source = format!("import * as x from '{import}'; export function search() {{ return []; }}");
        let (_folder, loaded) = package(&[("main.js", &source), ("util.js", "")]);
        let diagnostic = loaded.unwrap_err();
        assert_eq!(diagnostic.code, code, "{import}: {diagnostic:?}");
        assert!(diagnostic.hint.is_some());
    }
    let (_folder, loaded) = package(&[(
        "main.js",
        "import { apiVersion } from 'dupin'; export const search = () => [{ title: `${apiVersion}` }];",
    )]);
    assert_eq!(
        search(&loaded.unwrap(), "").unwrap()[0].view,
        View::Text {
            title: "1".into(),
            subtitle: String::new()
        }
    );
}

#[test]
fn no_ambient_host_apis_are_defined() {
    let rows = run(
        r#"export function search() {
             const names = ['std', 'os', 'require', 'process', 'fetch', 'setTimeout', 'XMLHttpRequest', 'print', 'scriptArgs'];
             return names.filter((name) => typeof globalThis[name] !== 'undefined').map((title) => ({ title }));
           }"#,
        "",
    )
    .unwrap();
    assert!(rows.is_empty(), "{rows:?}");
}

#[test]
fn package_and_result_problems_carry_codes_fields_and_hints() {
    for (source, code, field) in [
        ("export function search() { return 'x'; }", "invalid-result", None),
        (
            "export function search() { return { items: 1 }; }",
            "invalid-field",
            Some("items"),
        ),
        (
            "export function search() { return { type: 'detail', items: [] }; }",
            "invalid-field",
            Some("type"),
        ),
        (
            "export function search() { return { rows: [] }; }",
            "unknown-field",
            Some("rows"),
        ),
        (
            "export function search() { return [1]; }",
            "invalid-item",
            Some("items[0]"),
        ),
        (
            "export function search() { return [{}]; }",
            "missing-field",
            Some("items[0].title"),
        ),
        (
            "export function search() { return [{ title: 'a', icon: 'x' }]; }",
            "unknown-field",
            Some("items[0].icon"),
        ),
        (
            "export function search() { return [{ title: 'a\\nb' }]; }",
            "invalid-field",
            Some("items[0].title"),
        ),
        (
            "export function search() { return [{ title: 'a', text: '' }]; }",
            "invalid-field",
            Some("items[0].text"),
        ),
        (
            "export function search() { return [{ title: 'a', url: 'file:///etc/passwd' }]; }",
            "invalid-url",
            Some("items[0].url"),
        ),
        (
            "export function search() { return [{ title: 'a', url: 'javascript:alert(1)' }]; }",
            "invalid-url",
            Some("items[0].url"),
        ),
        (
            "export function search() { return [{ title: 'x'.repeat(513) }]; }",
            "field-too-long",
            Some("items[0].title"),
        ),
        (
            "export function search() { return [{ id: 'a', title: 'a' }, { id: 'a', title: 'b' }]; }",
            "duplicate-id",
            Some("items[1].id"),
        ),
        (
            "export function search() { return Array.from({ length: 51 }, () => ({ title: 'a' })); }",
            "too-many-rows",
            None,
        ),
        ("export const search = 1;", "missing-search", None),
        ("export function find() { return []; }", "missing-search", None),
        (
            "throw new TypeError('top level');\nexport function search() {}",
            "module-threw",
            None,
        ),
        (
            "export function search() { return [{ get title() { throw new Error('getter'); } }]; }",
            "search-threw",
            None,
        ),
        (
            "export function search() { return Promise.reject(new RangeError('later')); }",
            "search-threw",
            None,
        ),
    ] {
        let diagnostic = failure(run(source, ""));
        assert_eq!(
            (diagnostic.code.as_ref(), diagnostic.field.as_deref()),
            (code, field),
            "{source}: {diagnostic:?}"
        );
        assert!(diagnostic.hint.is_some(), "{source}: {diagnostic:?}");
        assert!(diagnostic.file.is_some(), "{source}: {diagnostic:?}");
    }
}

#[test]
fn syntax_errors_and_missing_entries_are_reported_when_loading() {
    let (_folder, loaded) = package(&[("main.js", "export function search() {\n  return [;\n}")]);
    let diagnostic = loaded.unwrap_err();
    assert_eq!(diagnostic.code, "syntax-error");
    assert_eq!(
        (diagnostic.file.as_deref(), diagnostic.line),
        (Some("main.js"), Some(2))
    );

    let (_folder, loaded) = package(&[
        ("main.js", "export function search() { return []; }"),
        ("helper.js", "const = 1;"),
    ]);
    assert_eq!(loaded.unwrap_err().file.as_deref(), Some("helper.js"));

    let (_folder, loaded) = package(&[("index.js", "export function search() { return []; }")]);
    let diagnostic = loaded.unwrap_err();
    assert_eq!(
        (diagnostic.code.as_ref(), diagnostic.file.as_deref()),
        ("missing-entry", Some("main.js"))
    );

    let (_folder, loaded) = package(&[("main.js", "\u{feff}export function search() { return []; }")]);
    assert!(loaded.is_ok());
    let folder = tempfile::tempdir().unwrap();
    fs::write(folder.path().join("main.js"), [0xff, 0xfe]).unwrap();
    assert_eq!(load(folder.path()).unwrap_err().code, "not-utf8");
}

#[test]
fn each_call_starts_from_fresh_globals() {
    let (_folder, loaded) = package(&[(
        "main.js",
        "let calls = 0; export function search() { calls += 1; globalThis.seen = (globalThis.seen ?? 0) + 1; return [{ title: `${calls} ${globalThis.seen}` }]; }",
    )]);
    let package = loaded.unwrap();
    for _ in 0..3 {
        let rows = search(&package, "").unwrap();
        assert_eq!(
            rows[0].view,
            View::Text {
                title: "1 1".into(),
                subtitle: String::new()
            }
        );
    }
}

#[test]
fn the_location_parser_reads_quickjs_frames() {
    assert_eq!(
        location("    at search (main.js:3:9)\n    at <eval>"),
        Some(("main.js".into(), 3, 9))
    );
    assert_eq!(
        location("    at lib/a.js:10:2\n"),
        Some(("lib/a.js".into(), 10, 2))
    );
    assert_eq!(location("    at search (native)\n"), None);
}

#[test]
fn compiling_stops_at_its_deadline_or_when_cancelled() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(
        folder.path().join(DEFAULT_ENTRY),
        "export function search() { return []; }",
    )
    .unwrap();
    let sources = Sources::read(folder.path(), DEFAULT_ENTRY).unwrap();
    let expired = Control {
        deadline: Instant::now(),
        is_cancelled: Box::new(|| false),
    };
    match Package::compile(sources.clone(), expired) {
        Err(RunError::Failed(diagnostic)) => assert_eq!(
            (diagnostic.code.as_ref(), diagnostic.file.as_deref()),
            ("deadline", Some("main.js"))
        ),
        other => panic!("expected a deadline, got {other:?}"),
    }
    let cancelled = Control {
        deadline: Instant::now() + DEADLINE,
        is_cancelled: Box::new(|| true),
    };
    assert_eq!(
        Package::compile(sources, cancelled).unwrap_err(),
        RunError::Cancelled
    );
}

#[test]
fn source_nested_too_deeply_to_compile_is_reported_where_it_is() {
    let depth = 200_000;
    let source = format!(
        "export const nested = {}0{};",
        "[".repeat(depth),
        "]".repeat(depth)
    );
    let (_folder, loaded) = package(&[
        ("lib.js", &source),
        (
            "main.js",
            "import './lib.js'; export function search() { return []; }",
        ),
    ]);
    let diagnostic = loaded.unwrap_err();
    assert_eq!(
        (diagnostic.code.as_ref(), diagnostic.file.as_deref()),
        ("stack-limit", Some("lib.js")),
        "{diagnostic:?}"
    );
}

#[test]
fn sources_from_another_process_meet_the_same_limits() {
    let file = |name: &str, bytes: &[u8]| (name.to_owned(), bytes.to_vec());
    assert!(Sources::new("main.js".into(), vec![file("main.js", b"export {};")]).is_ok());
    assert_eq!(
        Sources::new("main.js".into(), vec![file("other.js", b"")])
            .unwrap_err()
            .code,
        "missing-entry"
    );
    assert_eq!(
        Sources::new("main.js".into(), vec![file("main.js", &[0xff])])
            .unwrap_err()
            .code,
        "not-utf8"
    );
    let half = vec![b' '; 2 * 1024 * 1024 + 1];
    assert_eq!(
        Sources::new(
            "main.js".into(),
            vec![file("main.js", &half), file("lib.js", &half)]
        )
        .unwrap_err()
        .code,
        "package-too-large"
    );
    let many = (0..=256).map(|index| file(&format!("{index}.js"), b"")).collect();
    assert_eq!(
        Sources::new("0.js".into(), many).unwrap_err().code,
        "too-many-files"
    );
}
