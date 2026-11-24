use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use dupin_js_runtime::row::RowEffect;
use serde_json::{Value, json};

use crate::build::{CONFIG, node_builtin_import, summarize};
use crate::dev::{DevCopy, Fingerprint};
use crate::{BuildOptions, Diagnostic, Rolldown, Template, build, scaffold};

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

fn control() -> dupin_js_runtime::Control {
    dupin_js_runtime::Control {
        deadline: Instant::now() + Duration::from_secs(2),
        is_cancelled: Box::new(|| false),
    }
}

fn failure(error: dupin_js_runtime::RunError) -> Diagnostic {
    match error {
        dupin_js_runtime::RunError::Failed(diagnostic) => diagnostic,
        dupin_js_runtime::RunError::Cancelled => unreachable!("never cancelled"),
    }
}

fn compile(dir: &Path) -> Result<dupin_js_runtime::Package, Diagnostic> {
    let package = crate::manifest::read_package(dir)?;
    let sources = dupin_js_runtime::Sources::read(dir, &package.main)?;
    dupin_js_runtime::Package::compile(sources, control()).map_err(failure)
}

/// What `search(query)` answers, as `{ title, subtitle, text }` objects like the fixture's.
fn answer(dir: &Path, query: &str) -> Vec<Value> {
    compile(dir)
        .unwrap()
        .search(query, 10, control())
        .unwrap()
        .into_iter()
        .map(|item| {
            let mut row = json!({ "title": item.title });
            if !item.subtitle.is_empty() {
                row["subtitle"] = item.subtitle.into();
            }
            for action in item.actions {
                if let RowEffect::Copy { text } = action.effect {
                    row["text"] = text.into();
                }
            }
            row
        })
        .collect()
}

/// The scaffold's fixture states what its first query answers, and `--update` would write it
/// back unchanged.
fn assert_fixture_matches(dir: &Path) {
    let source = std::fs::read_to_string(dir.join("fixtures/search.json")).unwrap();
    let fixture = crate::fixture::parse("fixtures/search.json", source.as_bytes()).unwrap();
    assert_eq!(crate::fixture::to_json(&fixture), source);
    let crate::fixture::Action::Query(query) = fixture.steps[0].action() else {
        panic!("the fixture starts with a query");
    };
    let items = compile(dir).unwrap().search(query, 10, control()).unwrap();
    assert_eq!(
        fixture.steps[0].expect.as_ref().unwrap()["rows"],
        crate::fixture::rows(&items)
    );
}

/// Links the entry with the launcher's JavaScript runtime. The launcher's full loader, which
/// `dupin ext build` and `dupin ext dev` really validate with, is exercised by the `dupin` crate.
fn validate(dir: &Path) -> Result<(), Diagnostic> {
    compile(dir)?.check(control()).map(drop).map_err(failure)
}

/// Rolldown from `PATH`. The Nix checks and the dev shell put one there; elsewhere the tests that
/// need it skip their bundling half and say so.
fn rolldown() -> Option<Rolldown> {
    let found = Rolldown::find_on(
        Path::new("/nonexistent"),
        &std::env::var_os("PATH").unwrap_or_default(),
    );
    if found.is_none() {
        eprintln!("rolldown is not on PATH; skipping the bundling half of this test");
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

    validate(&dir).unwrap();
    assert_eq!(crate::manifest::read_package(&dir).unwrap().main, "main.js");
    assert_fixture_matches(&dir);

    let built = build(&dir, None, BuildOptions::default(), &validate).unwrap();
    assert_eq!(built.rolldown, None);

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
fn typescript_scaffold_runs_as_written_without_a_build() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("greeter");
    let mut files = scaffold(&dir, Template::TypeScript).unwrap();
    files.sort();
    assert_eq!(
        files,
        [
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
    assert!(manifest.get("source").is_none());
    assert_eq!(manifest["dupin"]["main"], "src/main.ts");
    let readme = std::fs::read_to_string(dir.join("README.md")).unwrap();
    assert!(
        readme.contains("## TypeScript")
            && readme.contains("dupin ext new --bundle")
            && !readme.contains("## Build")
            && !readme.contains("{{"),
        "{readme}"
    );
    let tsconfig = std::fs::read_to_string(dir.join("tsconfig.json")).unwrap();
    assert!(tsconfig.contains("\"erasableSyntaxOnly\": true"), "{tsconfig}");

    validate(&dir).unwrap();
    assert_fixture_matches(&dir);
    let built = build(&dir, None, BuildOptions::default(), &validate).unwrap();
    assert_eq!((built.rolldown, built.output.as_str()), (None, "src/main.ts"));
}

#[test]
fn bundled_scaffold_builds_with_rolldown() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("greeter");
    let mut files = scaffold(&dir, Template::Bundled).unwrap();
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
            "rolldown.config.ts",
            "src/main.ts",
            "tsconfig.json",
        ]
        .map(std::path::PathBuf::from)
    );
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap()).unwrap();
    assert_eq!(manifest["source"], "src/main.ts");
    assert_eq!(manifest["dupin"]["main"], "dist/main.js");
    let pinned = manifest["devDependencies"]["rolldown"].as_str().unwrap();
    assert!(
        pinned.split('.').count() == 3 && pinned.split('.').all(|part| part.parse::<u32>().is_ok()),
        "rolldown must be pinned to an exact version, not {pinned}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("rolldown.config.ts")).unwrap(),
        CONFIG
    );
    let workflow = std::fs::read_to_string(dir.join(".github/workflows/build.yml")).unwrap();
    assert!(
        workflow.contains("bun --bun run rolldown --config rolldown.config.ts")
            && workflow.contains("- 'rolldown.config.ts'")
            && workflow.contains("bun install --frozen-lockfile --ignore-scripts")
            && workflow.contains("persist-credentials: false")
            && workflow.contains("path: 'dist'\n")
            && workflow.contains("git add dist/main.js")
            && !workflow.contains("{{"),
        "{workflow}"
    );
    let (build_job, commit_job) = workflow.split_once("\n  commit:\n").unwrap();
    assert!(build_job.contains("contents: read") && !build_job.contains("contents: write"));
    assert!(!commit_job.contains("bun "), "{commit_job}");
    let readme = std::fs::read_to_string(dir.join("README.md")).unwrap();
    assert!(
        readme.contains("## Build")
            && readme.contains("dupin ext build .\ndupin ext check .")
            && !readme.contains("{{"),
        "{readme}"
    );

    assert_eq!(
        build(&dir, None, BuildOptions::default(), &validate)
            .unwrap_err()
            .code,
        "bundler-missing"
    );
    let Some(rolldown) = rolldown() else { return };
    let built = build(&dir, Some(&rolldown), BuildOptions { sourcemap: true }, &validate)
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(built.rolldown.as_deref(), Some(rolldown.program.as_path()));
    assert_eq!(built.output, "dist/main.js");
    assert_eq!(built.sourcemap.as_deref(), Some("dist/main.js.map"));
    assert!(dir.join("dist/main.js.map").is_file());
    let bundle = std::fs::read_to_string(dir.join("dist/main.js")).unwrap();
    assert!(
        !bundle.contains("import type") && bundle.ends_with("//# sourceMappingURL=main.js.map"),
        "{bundle}"
    );
    assert_fixture_matches(&dir);
}

#[test]
fn rolldown_is_found_in_the_package_before_path() {
    let package = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let executable = |path: &Path| {
        use std::os::unix::fs::PermissionsExt;
        write(path, "#!/bin/sh\n");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let on_path = elsewhere.path().join("rolldown");
    let path = std::env::join_paths([elsewhere.path()]).unwrap();
    assert_eq!(Rolldown::find_on(package.path(), &path), None);
    write(&on_path, "#!/bin/sh\n");
    assert_eq!(
        Rolldown::find_on(package.path(), &path),
        None,
        "a file that isn't executable is not a program"
    );
    executable(&on_path);
    assert_eq!(Rolldown::find_on(package.path(), &path).unwrap().program, on_path);
    let local = package.path().join("node_modules/.bin/rolldown");
    executable(&local);
    assert_eq!(Rolldown::find_on(package.path(), &path).unwrap().program, local);
}

#[test]
fn scaffolding_names_a_package_after_the_folder_a_dot_path_leads_to() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    // The crate's folder is a valid name and not empty, so getting past the name check proves
    // the name came from the folder `src/..` leads to.
    assert_eq!(
        scaffold(&crate_dir.join("src/.."), Template::JavaScript)
            .unwrap_err()
            .code,
        "folder-not-empty"
    );
}

#[test]
fn bundling_a_package_named_by_a_relative_path_writes_its_own_entry() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("relative");
    scaffold(&dir, Template::Bundled).unwrap();
    let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
    let mut relative: std::path::PathBuf = cwd.components().skip(1).map(|_| "..").collect();
    relative.push(dir.canonicalize().unwrap().strip_prefix("/").unwrap());
    assert!(relative.is_relative());
    let Some(rolldown) = rolldown() else { return };
    build(&relative, Some(&rolldown), BuildOptions::default(), &validate)
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_fixture_matches(&dir);
}

/// A package without a Rolldown configuration of its own, bundled with [`CONFIG`]. Its source is
/// TypeScript because the loader compiles every JavaScript file in the folder.
fn package_without_config(dir: &Path, main: &str, dependency: &str) {
    write(
        &dir.join("package.json"),
        &manifest(
            "reader",
            json!({ "source": "src/main.ts", "main": "dist/main.js" }),
        ),
    );
    write(&dir.join("src/main.ts"), main);
    write(
        &dir.join("node_modules/reader-dep/package.json"),
        r#"{ "name": "reader-dep", "main": "lib/entry.js" }"#,
    );
    write(&dir.join("node_modules/reader-dep/lib/entry.js"), dependency);
}

#[test]
fn bundling_inlines_main_field_dependencies_and_dynamic_imports_into_one_module() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    package_without_config(
        dir,
        "import { title } from 'reader-dep';\n\
         export async function search() {\n\
           const { suffix } = await import('./suffix.ts');\n\
           return [{ title: title + suffix }];\n\
         }\n",
        "exports.title = 'from a CommonJS main';\n",
    );
    write(&dir.join("src/suffix.ts"), "export const suffix = '!';\n");
    let Some(rolldown) = rolldown() else { return };
    build(dir, Some(&rolldown), BuildOptions::default(), &validate)
        .unwrap_or_else(|error| panic!("{error:?}"));
    let emitted: Vec<_> = std::fs::read_dir(dir.join("dist"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(emitted, ["main.js"]);
    assert_eq!(answer(dir, ""), vec![json!({ "title": "from a CommonJS main!" })]);
}

#[test]
fn bundling_rejects_node_builtins_and_keeps_the_previous_entry() {
    let previous = "export function search() { return []; }\n";
    let cases = [
        (
            "import { readFileSync } from 'node:fs';\nexports.load = () => readFileSync('x');\n",
            "node:fs",
        ),
        ("exports.load = require('path').join;\n", "path"),
        ("exports.load = () => import('os');\n", "os"),
    ];
    for (dependency, builtin) in cases {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        package_without_config(
            dir,
            "import { load } from 'reader-dep';\nexport function search() { return [{ title: String(load) }]; }\n",
            dependency,
        );
        write(&dir.join("dist/main.js"), previous);
        let Some(rolldown) = rolldown() else { return };
        let error = build(dir, Some(&rolldown), BuildOptions::default(), &validate).unwrap_err();
        assert_eq!(error.code, "node-builtin", "{error:?}");
        assert_eq!(
            error.file.as_deref(),
            Some("node_modules/reader-dep/lib/entry.js"),
            "{error:?}"
        );
        assert!(error.message.contains(&format!("`{builtin}`")), "{error:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("dist/main.js")).unwrap(),
            previous
        );
    }
}

#[test]
fn bundling_refuses_a_configuration_the_loader_would_run_as_extension_code() {
    let previous = "export function search() { return []; }\n";
    for name in ["rolldown.config.js", "rolldown.config.mjs"] {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        package_without_config(dir, previous, "");
        write(&dir.join(name), CONFIG);
        write(&dir.join("dist/main.js"), previous);
        let never_runs = Rolldown {
            program: dir.join("no-rolldown"),
        };
        let error = build(dir, Some(&never_runs), BuildOptions::default(), &validate).unwrap_err();
        assert_eq!(error.code, "config-is-extension-code", "{error:?}");
        assert_eq!(error.file.as_deref(), Some(name), "{error:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("dist/main.js")).unwrap(),
            previous
        );
    }
}

#[test]
fn bundling_fails_on_an_import_rolldown_cannot_resolve() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    package_without_config(
        dir,
        "import { missing } from 'not-installed';\nexport function search() { return [{ title: missing }]; }\n",
        "",
    );
    let Some(rolldown) = rolldown() else { return };
    let error = build(dir, Some(&rolldown), BuildOptions::default(), &validate).unwrap_err();
    assert_eq!(error.code, "build-failed", "{error:?}");
    assert!(
        error.message.contains("not-installed") && !error.message.contains("\n    at "),
        "{error:?}"
    );
    assert!(!dir.join("dist/main.js").exists());
}

#[test]
fn reports_name_the_module_and_the_builtin() {
    assert_eq!(
        node_builtin_import(
            br#"{ "nodeBuiltin": { "importer": "node_modules/dep/net.js", "specifier": "net" } }"#
        ),
        Some(("node_modules/dep/net.js".into(), "net".into()))
    );
    assert_eq!(node_builtin_import(b"{}"), None);
    assert_eq!(node_builtin_import(b"not json"), None);
    assert_eq!(
        summarize("RolldownError: nope\n    at error (file:///x.mjs:1:2)\n    at main (file:///y.mjs:3:4)\n"),
        "RolldownError: nope"
    );
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
        rolldown: None,
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
        rolldown: None,
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
            rolldown: None,
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
