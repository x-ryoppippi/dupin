use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use dupin_plugin_protocol::{Effect, View};
use serde_json::{Value, json};

use crate::build::node_builtin_import;
use crate::dev::{DevCopy, Fingerprint};
use crate::{BuildOptions, Bundler, Diagnostic, Template, build, check, scaffold};

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn manifest(name: &str, extra: Value) -> String {
    let mut manifest = json!({
        "name": name,
        "version": "0.1.0",
        "dupin": { "title": "Test", "keyword": name },
    });
    for (key, value) in extra.as_object().unwrap() {
        if key == "main" {
            manifest["dupin"]["main"] = value.clone();
        } else {
            manifest[key] = value.clone();
        }
    }
    manifest.to_string()
}

/// What `search(query)` answers, as `{ title, subtitle, text }` objects like the fixture's.
fn answer(dir: &Path, query: &str) -> Vec<Value> {
    let package = crate::manifest::read_package(dir).unwrap();
    let module = dupin_js_runtime::Package::load(dir, &package.main).unwrap();
    let control = dupin_js_runtime::Control {
        deadline: Instant::now() + Duration::from_secs(2),
        is_cancelled: Box::new(|| false),
    };
    module
        .search(query, 10, control)
        .unwrap()
        .into_iter()
        .map(|item| {
            let View::Text { title, subtitle } = item.view else {
                panic!("expected a text row");
            };
            let mut row = json!({ "title": title });
            if !subtitle.is_empty() {
                row["subtitle"] = subtitle.into();
            }
            for action in item.actions {
                if let Effect::Copy { text } = action.effect {
                    row["text"] = text.into();
                }
            }
            row
        })
        .collect()
}

fn assert_fixture_matches(dir: &Path) {
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("fixtures/search.json")).unwrap()).unwrap();
    assert_eq!(
        answer(dir, fixture["query"].as_str().unwrap()),
        fixture["items"].as_array().unwrap().clone()
    );
}

fn validate(dir: &Path) -> Result<(), Diagnostic> {
    check(dir).map(drop)
}

fn bundlers() -> Vec<Bundler> {
    let found = Bundler::all_on(&std::env::var_os("PATH").unwrap_or_default());
    if found.is_empty() {
        eprintln!("neither bun nor esbuild is on PATH; skipping the bundling half of this test");
    }
    found
}

#[test]
fn declarations_state_the_hosts_api_version() {
    let declared: u32 = crate::DECLARATIONS
        .split_once("export const apiVersion: ")
        .and_then(|(_, rest)| rest.split_once(';'))
        .and_then(|(version, _)| version.parse().ok())
        .expect("dupin.d.ts declares `export const apiVersion: <n>;`");
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("package.json"), &manifest("version", json!({})));
    write(
        &dir.path().join("main.js"),
        "import { apiVersion } from 'dupin';\nexport function search() { return [{ title: String(apiVersion) }]; }\n",
    );
    assert_eq!(
        answer(dir.path(), ""),
        vec![json!({ "title": declared.to_string() })]
    );
}

#[test]
fn javascript_scaffold_checks_and_answers_its_fixture() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("hello-world");
    let mut files = scaffold(&dir, Template::JavaScript).unwrap();
    files.sort();
    assert_eq!(
        files,
        [
            "README.md",
            "dupin.d.ts",
            "fixtures/search.json",
            "main.js",
            "package.json"
        ]
        .map(std::path::PathBuf::from)
    );
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap()).unwrap();
    assert_eq!(manifest["name"], "hello-world");
    assert_eq!(manifest["keywords"], json!(["dupin-extension"]));
    assert_eq!(
        manifest["dupin"],
        json!({ "title": "Hello World", "keyword": "hello-world" })
    );
    assert!(manifest.get("source").is_none());
    assert_eq!(
        std::fs::read_to_string(dir.join("dupin.d.ts")).unwrap(),
        crate::DECLARATIONS
    );
    let readme = std::fs::read_to_string(dir.join("README.md")).unwrap();
    assert!(
        readme.starts_with("# Hello World\n") && !readme.contains("{{"),
        "{readme}"
    );
    assert!(!readme.contains("## Build"));

    let package = check(&dir).unwrap();
    assert_eq!(package.main, "main.js");
    assert_fixture_matches(&dir);

    let built = build(&dir, None, BuildOptions::default(), &validate).unwrap();
    assert_eq!(built.bundler, None);

    assert_eq!(
        scaffold(&dir, Template::JavaScript).unwrap_err().code,
        "folder-not-empty"
    );
    assert_eq!(
        scaffold(&root.path().join("Hello"), Template::JavaScript)
            .unwrap_err()
            .code,
        "invalid-name"
    );
}

#[test]
fn typescript_scaffold_builds_with_each_bundler_on_path() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("greeter");
    let mut files = scaffold(&dir, Template::TypeScript).unwrap();
    files.sort();
    assert_eq!(
        files,
        [
            ".github/workflows/build.yml",
            ".gitignore",
            "README.md",
            "dupin.d.ts",
            "fixtures/search.json",
            "package.json",
            "src/main.ts",
            "tsconfig.json",
        ]
        .map(std::path::PathBuf::from)
    );
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap()).unwrap();
    assert_eq!(manifest["source"], "src/main.ts");
    assert_eq!(manifest["dupin"]["main"], "dist/main.js");
    let workflow = std::fs::read_to_string(dir.join(".github/workflows/build.yml")).unwrap();
    assert!(
        workflow.contains("bun build src/main.ts --outfile=dist/main.js")
            && workflow.contains("git add dist/main.js")
            && !workflow.contains("{{"),
        "{workflow}"
    );
    let readme = std::fs::read_to_string(dir.join("README.md")).unwrap();
    assert!(readme.contains("## Build") && !readme.contains("{{"), "{readme}");

    assert_eq!(
        build(&dir, None, BuildOptions::default(), &validate)
            .unwrap_err()
            .code,
        "bundler-missing"
    );
    for bundler in bundlers() {
        let _ = std::fs::remove_dir_all(dir.join("dist"));
        let built = build(&dir, Some(&bundler), BuildOptions { sourcemap: true }, &validate)
            .unwrap_or_else(|error| panic!("{}: {error:?}", bundler.kind.name()));
        assert_eq!(built.bundler, Some(bundler.kind));
        assert_eq!(built.output, "dist/main.js");
        assert!(dir.join("dist/main.js.map").is_file(), "{}", bundler.kind.name());
        let bundle = std::fs::read_to_string(dir.join("dist/main.js")).unwrap();
        assert!(!bundle.contains("import type"), "{bundle}");
        assert_fixture_matches(&dir);
    }
}

#[test]
fn bundling_rejects_node_builtins_and_keeps_the_previous_entry() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    write(
        &dir.join("package.json"),
        &manifest(
            "reader",
            json!({ "source": "src/main.js", "main": "dist/main.js" }),
        ),
    );
    write(
        &dir.join("src/main.js"),
        "import { load } from 'reader-dep';\nexport function search() { return [{ title: String(load) }]; }\n",
    );
    write(
        &dir.join("node_modules/reader-dep/package.json"),
        r#"{ "name": "reader-dep", "type": "module", "main": "index.js" }"#,
    );
    write(
        &dir.join("node_modules/reader-dep/index.js"),
        "import { readFileSync } from 'node:fs';\nexport const load = () => readFileSync('x');\n",
    );
    let previous = "export function search() { return []; }\n";
    write(&dir.join("dist/main.js"), previous);

    for bundler in bundlers() {
        let error = build(dir, Some(&bundler), BuildOptions::default(), &validate).unwrap_err();
        assert_eq!(error.code, "node-builtin", "{}: {error:?}", bundler.kind.name());
        assert!(
            error
                .file
                .as_deref()
                .unwrap()
                .contains("node_modules/reader-dep/index.js"),
            "{error:?}"
        );
        assert!(error.message.contains("node:fs"), "{error:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("dist/main.js")).unwrap(),
            previous
        );
    }
}

#[test]
fn metafiles_from_either_bundler_reveal_node_builtins() {
    let bun = json!({ "inputs": {
        "src/main.ts": { "imports": [{ "path": "dupin", "kind": "import-statement", "external": true }] },
        "node_modules/dep/index.js": { "imports": [
            { "path": "node:path", "kind": "import-statement", "original": "path" },
        ] },
    } });
    assert_eq!(
        node_builtin_import(&bun),
        Some(("node_modules/dep/index.js".into(), "path".into()))
    );
    let esbuild = json!({ "inputs": {
        "node_modules/dep/net.js": { "imports": [{ "path": "net", "kind": "require-call", "external": true }] },
    } });
    assert_eq!(
        node_builtin_import(&esbuild),
        Some(("node_modules/dep/net.js".into(), "net".into()))
    );
    let clean = json!({ "inputs": {
        "src/main.ts": { "imports": [
            { "path": "dupin", "external": true },
            { "path": "node_modules/pathe/index.js", "original": "pathe" },
        ] },
    } });
    assert_eq!(node_builtin_import(&clean), None);
}

fn link_target(link: &Path) -> String {
    std::fs::read_link(link).unwrap().display().to_string()
}

#[test]
fn development_copy_swaps_atomically_and_keeps_the_last_good_revision() {
    let package = tempfile::tempdir().unwrap();
    let extensions = tempfile::tempdir().unwrap();
    let (package, extensions) = (package.path(), extensions.path());
    write(&package.join("package.json"), &manifest("live", json!({})));
    write(
        &package.join("lib/rows.js"),
        "export const rows = [{ title: 'one' }];\n",
    );
    write(
        &package.join("main.js"),
        "import { rows } from './lib/rows.js';\nexport function search() { return rows; }\n",
    );
    let copy = DevCopy {
        package_dir: package,
        extensions,
        bundler: None,
        options: BuildOptions::default(),
        validate: &validate,
    };
    let link = extensions.join("live");

    let first = copy.publish().unwrap();
    assert_eq!((first.revision, first.link.clone()), (1, link.clone()));
    assert_eq!(link_target(&link), ".dev/live/r1");
    assert_eq!(answer(&link, ""), vec![json!({ "title": "one" })]);

    write(&package.join("main.js"), "export function search( {\n");
    let error = copy.publish().unwrap_err();
    assert_eq!(error.code, "syntax-error", "{error:?}");
    assert_eq!(link_target(&link), ".dev/live/r1");
    assert_eq!(answer(&link, ""), vec![json!({ "title": "one" })]);
    let leftovers: Vec<_> = std::fs::read_dir(extensions.join(".dev/live"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(leftovers, ["r1"]);

    write(
        &package.join("main.js"),
        "export function search() { return [{ title: 'two' }]; }\n",
    );
    assert_eq!(copy.publish().unwrap().revision, 2);
    assert_eq!(link_target(&link), ".dev/live/r2");
    assert_eq!(answer(&link, ""), vec![json!({ "title": "two" })]);
    assert_eq!(copy.publish().unwrap().revision, 3);
    assert!(!extensions.join(".dev/live/r1").exists());
    assert!(extensions.join(".dev/live/r2").is_dir());
    assert!(
        std::fs::read_dir(extensions).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with("dev-link"))
    );
}

#[test]
fn development_copy_never_replaces_an_installed_extension() {
    let package = tempfile::tempdir().unwrap();
    let extensions = tempfile::tempdir().unwrap();
    write(
        &package.path().join("package.json"),
        &manifest("taken", json!({})),
    );
    write(
        &package.path().join("main.js"),
        "export function search() { return []; }\n",
    );
    write(&extensions.path().join("taken/main.js"), "installed\n");
    let copy = DevCopy {
        package_dir: package.path(),
        extensions: extensions.path(),
        bundler: None,
        options: BuildOptions::default(),
        validate: &validate,
    };
    assert_eq!(copy.publish().unwrap_err().code, "dev-target-occupied");
    assert_eq!(
        std::fs::read_to_string(extensions.path().join("taken/main.js")).unwrap(),
        "installed\n"
    );
}

#[test]
fn fingerprint_follows_sources_but_not_dependencies_or_the_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    write(
        &dir.join("package.json"),
        &manifest(
            "watched",
            json!({ "source": "src/main.ts", "main": "dist/main.js" }),
        ),
    );
    write(&dir.join("src/main.ts"), "export {};\n");
    let before = Fingerprint::of(dir);
    write(&dir.join("node_modules/dep/index.js"), "x\n");
    write(&dir.join(".git/HEAD"), "x\n");
    write(&dir.join("dist/main.js"), "x\n");
    write(&dir.join("dist/main.js.map"), "x\n");
    assert_eq!(Fingerprint::of(dir), before);
    write(&dir.join("src/lib.ts"), "export {};\n");
    assert_ne!(Fingerprint::of(dir), before);
}

#[test]
fn watching_publishes_again_after_an_edit() {
    let package = tempfile::tempdir().unwrap();
    let extensions = tempfile::tempdir().unwrap();
    write(
        &package.path().join("package.json"),
        &manifest("watch", json!({})),
    );
    write(
        &package.path().join("main.js"),
        "export function search() { return [{ title: 'a' }]; }\n",
    );
    let running = Arc::new(AtomicBool::new(true));
    let (package_dir, extensions_dir) = (package.path().to_owned(), extensions.path().to_owned());
    let flag = running.clone();
    let watcher = std::thread::spawn(move || {
        let mut revisions = Vec::new();
        DevCopy {
            package_dir: &package_dir,
            extensions: &extensions_dir,
            bundler: None,
            options: BuildOptions::default(),
            validate: &validate,
        }
        .watch(
            Duration::from_millis(20),
            &|| flag.load(Ordering::Relaxed),
            &mut |outcome| revisions.push(outcome.unwrap().revision),
        );
        revisions
    });
    let link = extensions.path().join("watch");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !link.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    // Modification times can be coarse; a different length is always a change.
    write(
        &package.path().join("main.js"),
        "export function search() { return [{ title: 'edited' }]; }\n",
    );
    while link_target(&link) != ".dev/watch/r2" && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    running.store(false, Ordering::Relaxed);
    assert_eq!(watcher.join().unwrap(), [1, 2]);
    assert_eq!(answer(&link, ""), vec![json!({ "title": "edited" })]);
}
