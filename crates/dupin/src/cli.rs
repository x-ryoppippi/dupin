use crate::control::{self, Command, Request, exchange};
use serde_json::Value;
use std::io::{self, Read};
const HELP: &str = "dupin open [--query QUERY] [--json]\ndupin pick [--input-json] [--json]\ndupin snapshots list|close [--json]\ndupin snapshots open [--id ID] [--json]\ndupin config get [--json]\ndupin config set BASE.json DESIRED.json [--json]\ndupin snap VIEW.json OUTPUT.png [--json]\ndupin ext check DIR [--json]\ndupin ext run DIR --query QUERY [--json]\ndupin ext new DIR [--ts] [--json]\ndupin ext build DIR [--sourcemap] [--json]\ndupin ext dev DIR [--sourcemap] [--json]\ndupin ext check MODULE.wasm REQUEST.json [--json]\ndupin ext snap MODULE.wasm REQUEST.json OUTPUT.png [--json]\ndupin ext install github:OWNER/REPO[@REF]|URL[@REF]|DIR [--json]\ndupin ext list|sync [--json]\ndupin ext update [NAME] [--json]\ndupin ext remove NAME [--json]\ndupin mcp\n\nNo subcommand starts the resident launcher. Existing Rune tooling flags remain available.";
#[derive(Debug, PartialEq)]
enum Action {
    Help,
    Version,
    Open(Option<String>),
    Pick(bool),
    Get,
    SnapshotsList,
    SnapshotsOpen(Option<String>),
    SnapshotsClose,
    Set(String, String),
    Snap(String, String),
    Ext(String, String, Option<String>),
    /// Check an extension folder.
    ExtCheck(String),
    /// Ask an extension folder for a query.
    ExtRun(String, String),
    /// Install, list, update, remove or restore packages in the extensions folder.
    ExtPackages(crate::extension_install::Command),
    ExtNew(String, dupin_ext_sdk::Template),
    ExtBuild(String, dupin_ext_sdk::BuildOptions),
    ExtDev(String, dupin_ext_sdk::BuildOptions),
    Mcp,
}

/// Why a command failed: a plain message, or a diagnostic about an extension package.
#[derive(Debug, PartialEq)]
enum Failure {
    Message(String),
    Diagnostic(dupin_js_runtime::Diagnostic),
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::Message(message.into())
    }
}

impl From<dupin_js_runtime::Diagnostic> for Failure {
    fn from(diagnostic: dupin_js_runtime::Diagnostic) -> Self {
        Self::Diagnostic(diagnostic)
    }
}

impl From<dupin_js_runtime::RunError> for Failure {
    fn from(error: dupin_js_runtime::RunError) -> Self {
        match error {
            dupin_js_runtime::RunError::Cancelled => Self::Message("the extension was cancelled".into()),
            dupin_js_runtime::RunError::Failed(diagnostic) => Self::Diagnostic(diagnostic),
        }
    }
}
fn parse(args: &[String]) -> Result<Action, String> {
    use crate::extension_install::Command as Packages;
    let mut parser = pico_args::Arguments::from_vec(args.iter().map(std::ffi::OsString::from).collect());
    let json = parser.contains("--json");
    let typescript = parser.contains("--ts");
    let options = dupin_ext_sdk::BuildOptions {
        sourcemap: parser.contains("--sourcemap"),
    };
    let rest = parser.finish();
    let args: Vec<_> = rest
        .iter()
        .map(|s| s.to_str().ok_or_else(|| "arguments must be UTF-8".to_owned()))
        .collect::<Result<_, _>>()?;
    match args.as_slice() {
        ["ext", "new", folder] if !folder.starts_with('-') && !options.sourcemap => {
            let template = if typescript {
                dupin_ext_sdk::Template::TypeScript
            } else {
                dupin_ext_sdk::Template::JavaScript
            };
            Ok(Action::ExtNew(folder.to_string(), template))
        }
        ["ext", "build", folder] if !folder.starts_with('-') && !typescript => {
            Ok(Action::ExtBuild(folder.to_string(), options))
        }
        ["ext", "dev", folder] if !folder.starts_with('-') && !typescript => {
            Ok(Action::ExtDev(folder.to_string(), options))
        }
        _ if typescript || options.sourcemap => Err("invalid arguments; use dupin --help".into()),
        ["--help" | "-h"] => Ok(Action::Help),
        ["--version" | "-V"] => Ok(Action::Version),
        ["open"] => Ok(Action::Open(None)),
        ["open", "--query", q] => Ok(Action::Open(Some(q.to_string()))),
        ["mcp"] if !json => Ok(Action::Mcp),
        ["pick"] => Ok(Action::Pick(false)),
        ["pick", "--input-json"] => Ok(Action::Pick(true)),
        ["snapshots", "list"] => Ok(Action::SnapshotsList),
        ["snapshots", "close"] => Ok(Action::SnapshotsClose),
        ["snapshots", "open"] => Ok(Action::SnapshotsOpen(None)),
        ["snapshots", "open", "--id", id] if !id.starts_with("-") && !id.is_empty() && id.len() <= 128 => {
            Ok(Action::SnapshotsOpen(Some(id.to_string())))
        }
        ["config", "get"] => Ok(Action::Get),
        ["config", "set", base, desired] if !base.starts_with('-') && !desired.starts_with('-') => {
            Ok(Action::Set(base.to_string(), desired.to_string()))
        }
        ["snap", view, out] if !view.starts_with('-') && !out.starts_with('-') => {
            Ok(Action::Snap(view.to_string(), out.to_string()))
        }
        ["ext", "install", source] if !source.starts_with('-') => {
            Ok(Action::ExtPackages(Packages::Install(source.to_string())))
        }
        ["ext", "list"] => Ok(Action::ExtPackages(Packages::List)),
        ["ext", "sync"] => Ok(Action::ExtPackages(Packages::Sync)),
        ["ext", "update"] => Ok(Action::ExtPackages(Packages::Update(None))),
        ["ext", "update", name] if !name.starts_with('-') => {
            Ok(Action::ExtPackages(Packages::Update(Some(name.to_string()))))
        }
        ["ext", "remove", name] if !name.starts_with('-') => {
            Ok(Action::ExtPackages(Packages::Remove(name.to_string())))
        }
        ["ext", "check", folder] if !folder.starts_with('-') => Ok(Action::ExtCheck(folder.to_string())),
        ["ext", "run", folder, "--query", query] if !folder.starts_with('-') => {
            Ok(Action::ExtRun(folder.to_string(), query.to_string()))
        }
        ["ext", "check", module, request] if !module.starts_with('-') && !request.starts_with('-') => {
            Ok(Action::Ext(module.to_string(), request.to_string(), None))
        }
        ["ext", "snap", module, request, out]
            if !module.starts_with('-') && !request.starts_with('-') && !out.starts_with('-') =>
        {
            Ok(Action::Ext(
                module.to_string(),
                request.to_string(),
                Some(out.to_string()),
            ))
        }
        _ => Err("invalid arguments; use dupin --help".into()),
    }
}
fn bounded(mut reader: impl Read, cap: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read input")?;
    if bytes.len() > cap {
        return Err("input too large".into());
    }
    Ok(bytes)
}
fn file(path: &str, cap: usize) -> Result<Vec<u8>, String> {
    bounded(
        std::fs::File::open(path).map_err(|_| "could not open input")?,
        cap,
    )
}
fn selection(bytes: &[u8], json: bool) -> Result<Vec<Value>, String> {
    let values = if json {
        serde_json::from_slice::<Vec<Value>>(bytes).map_err(|_| "expected a JSON array")?
    } else {
        std::str::from_utf8(bytes)
            .map_err(|_| "stdin must be UTF-8")?
            .lines()
            .map(|s| Value::String(s.into()))
            .collect()
    };
    control::validate(&Request {
        id: 1,
        command: Command::Pick {
            values: values.clone(),
        },
    })
    .map_err(|e| e.to_string())?;
    Ok(values)
}
pub fn run() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return false;
    }
    if !owns_arguments(&args) {
        return false;
    }
    let json = args.iter().any(|v| v == "--json");
    let action = parse(&args);
    let picked = matches!(action, Ok(Action::Pick(_)));
    let result = action.map_err(Failure::from).and_then(|action| {
        let output = match &action {
            Action::Snap(_, output) | Action::Ext(_, _, Some(output)) if json => Some(output.clone()),
            _ => None,
        };
        if let Some(output) = output {
            isolated_render(&args, &output).map_err(Failure::from)
        } else {
            execute(action, json)
        }
    });
    match result {
        Ok(value) => {
            if json || !value.is_null() || picked {
                if !json && picked {
                    if let Some(s) = value.as_str() {
                        println!("{s}");
                    } else {
                        println!("{value}");
                    }
                } else {
                    println!("{value}");
                }
            }
            true
        }
        Err(Failure::Message(error)) => {
            if json {
                println!("{}", serde_json::json!({"error": error}));
            } else {
                eprintln!("dupin: {error}");
            }
            std::process::exit(1)
        }
        Err(Failure::Diagnostic(diagnostic)) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"error": diagnostic.to_string(), "diagnostic": diagnostic})
                );
            } else {
                eprintln!("dupin: {diagnostic}");
                if let Some(hint) = &diagnostic.hint {
                    eprintln!("hint: {hint}");
                }
            }
            std::process::exit(1)
        }
    }
}

fn owns_arguments(args: &[String]) -> bool {
    args.first().is_some_and(|first| {
        !first.starts_with('-') || matches!(first.as_str(), "--help" | "-h" | "--version" | "-V" | "--json")
    })
}
fn isolated_render(args: &[String], output: &str) -> Result<Value, String> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    // Rune's headless API exits the process on renderer errors. Contain it so JSON callers
    // receive a structured error, including when GPU initialization or PNG output fails.
    let mut child = Command::new(std::env::current_exe().map_err(|_| "could not locate renderer")?)
        .args(args.iter().filter(|arg| arg.as_str() != "--json"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "could not start headless renderer")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(serde_json::json!({"png": output}))
                } else {
                    Err("headless rendering failed".into())
                };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("headless rendering exceeded its deadline or stopped".into());
            }
        }
    }
}

fn execute(action: Action, json: bool) -> Result<Value, Failure> {
    use dupin_plugin_protocol as protocol;
    match action {
        Action::Help => {
            if json {
                return Ok(serde_json::json!({"help": HELP}));
            }
            println!("{HELP}");
            Ok(Value::Null)
        }
        Action::Version => {
            let version = include_str!("../../../VERSION").trim();
            if json {
                Ok(serde_json::json!({"version":version}))
            } else {
                println!("dupin {version}");
                Ok(Value::Null)
            }
        }
        Action::Open(query) => Ok(exchange(Command::Open { query })?),
        Action::Pick(json_input) => Ok(exchange(Command::Pick {
            values: selection(&bounded(io::stdin().lock(), control::MAX_FRAME / 2)?, json_input)?,
        })?),
        Action::SnapshotsList => Ok(exchange(Command::SnapshotsList {})?),
        Action::SnapshotsOpen(id) => Ok(exchange(Command::SnapshotsOpen { id })?),
        Action::SnapshotsClose => Ok(exchange(Command::SnapshotsClose {})?),
        Action::Get => Ok(exchange(Command::ConfigGet {})?),
        Action::Set(base, desired) => Ok(exchange(Command::ConfigSet {
            base: serde_json::from_slice(&file(&base, 65536)?).map_err(|_| "invalid base configuration")?,
            desired: serde_json::from_slice(&file(&desired, 65536)?)
                .map_err(|_| "invalid desired configuration")?,
        })?),
        Action::Snap(input, output) => {
            let view: protocol::View = serde_json::from_slice(&file(&input, protocol::MAX_RESPONSE_BYTES)?)
                .map_err(|_| "invalid view")?;
            let req = protocol::Request {
                version: protocol::VERSION,
                request_id: "snap".into(),
                generation: "1".into(),
                command: "snap".into(),
                query: String::new(),
                limit: 1,
                state: None,
                view: None,
                context: None,
            };
            let response = protocol::Response {
                version: protocol::VERSION,
                request_id: req.request_id.clone(),
                generation: req.generation.clone(),
                outcome: protocol::Outcome::Results {
                    items: vec![protocol::PluginItem {
                        id: "view".into(),
                        view,
                        actions: vec![],
                    }],
                    state: None,
                },
            };
            response.encode(&req).map_err(|e| e.to_string())?;
            render(response, &output);
            Ok(serde_json::json!({"png": output}))
        }
        Action::Ext(module, request, output) => {
            let request = protocol::Request::decode(&file(&request, protocol::MAX_REQUEST_BYTES)?)
                .map_err(|e| e.to_string())?;
            let module = dupin_plugin_runtime::GuestModule::load(&file(
                &module,
                dupin_plugin_runtime::MAX_MODULE_BYTES,
            )?)
            .map_err(|e| e.to_string())?;
            let response = module.query(&request).map_err(|e| e.to_string())?;
            if let Some(output) = output {
                render(response, &output);
                Ok(serde_json::json!({"png":output}))
            } else {
                serde_json::to_value(response).map_err(|_| "could not encode response".into())
            }
        }
        Action::ExtCheck(folder) => {
            let extension = crate::extensions::check_folder(std::path::Path::new(&folder))?;
            if json {
                return Ok(serde_json::json!({
                    "id": extension.id,
                    "name": extension.name,
                    "version": extension.version,
                    "keyword": extension.keyword,
                    "runtime": extension.runtime(),
                }));
            }
            println!(
                "{} {} ({}) answers `{}`",
                extension.name,
                extension.version,
                extension.runtime(),
                extension.keyword
            );
            Ok(Value::Null)
        }
        Action::ExtRun(folder, query) => {
            let extension = crate::extensions::load_folder(std::path::Path::new(&folder))?;
            let outcome = extension.query(
                &query,
                None,
                dupin_js_runtime::MAX_ROWS,
                std::time::Instant::now() + crate::extensions::QUERY_DEADLINE,
                || false,
            )?;
            if let protocol::Outcome::Error { message, .. } = &outcome {
                return Err(Failure::Message(format!(
                    "the extension answered with an error: {message}"
                )));
            }
            if json {
                return serde_json::to_value(outcome).map_err(|_| "could not encode the answer".into());
            }
            print!("{}", describe_outcome(&outcome));
            Ok(Value::Null)
        }
        Action::ExtPackages(command) => Ok(crate::extension_install::run(command, json)?),
        Action::ExtNew(folder, template) => {
            let files = dupin_ext_sdk::scaffold(std::path::Path::new(&folder), template)?;
            let files: Vec<String> = files.iter().map(|file| file.display().to_string()).collect();
            if json {
                return Ok(serde_json::json!({ "created": files }));
            }
            println!("Created {folder}:");
            for file in files {
                println!("  {file}");
            }
            Ok(Value::Null)
        }
        Action::ExtBuild(folder, options) => {
            let folder = std::path::Path::new(&folder);
            let built = dupin_ext_sdk::build(
                folder,
                dupin_ext_sdk::Rolldown::find(folder).as_ref(),
                options,
                &check_package,
            )?;
            let rolldown = built
                .rolldown
                .as_ref()
                .map(|program| program.display().to_string());
            if json {
                return Ok(serde_json::json!({
                    "name": built.package.name,
                    "output": built.output,
                    "rolldown": rolldown,
                    "sourcemap": built.sourcemap,
                }));
            }
            match rolldown {
                Some(rolldown) => println!("Built {} with {rolldown}", built.output),
                None => println!(
                    "{} is plain JavaScript, needs no build and passes the check",
                    built.output
                ),
            }
            Ok(Value::Null)
        }
        Action::ExtDev(folder, options) => {
            let extensions = crate::extensions::directory()
                .ok_or("couldn't find the extensions folder beside config.toml")?;
            let package_dir = std::path::Path::new(&folder);
            let rolldown = dupin_ext_sdk::Rolldown::find(package_dir);
            let copy = dupin_ext_sdk::dev::DevCopy {
                package_dir,
                extensions: &extensions,
                rolldown: rolldown.as_ref(),
                options,
                validate: &check_package,
            };
            copy.watch(std::time::Duration::from_millis(250), &|| true, &mut |outcome| {
                report_dev(outcome, json)
            });
            Ok(Value::Null)
        }
        Action::Mcp => crate::mcp::run()
            .map(|()| Value::Null)
            .map_err(|e| e.to_string().into()),
    }
}

fn check_package(folder: &std::path::Path) -> Result<(), dupin_js_runtime::Diagnostic> {
    crate::extensions::check_folder(folder).map(drop)
}

/// `dupin ext dev` keeps running, so each revision is reported as it happens: a JSON line with
/// `--json`, otherwise a line on stdout or a diagnostic on stderr.
fn report_dev(outcome: Result<dupin_ext_sdk::dev::Published, dupin_js_runtime::Diagnostic>, json: bool) {
    match (outcome, json) {
        (Ok(published), true) => println!(
            "{}",
            serde_json::json!({
                "published": published.name,
                "revision": published.revision,
                "path": published.link,
            })
        ),
        (Ok(published), false) => println!(
            "Published {} revision {} to {}",
            published.name,
            published.revision,
            published.link.display()
        ),
        (Err(diagnostic), true) => println!(
            "{}",
            serde_json::json!({"error": diagnostic.to_string(), "diagnostic": diagnostic})
        ),
        (Err(diagnostic), false) => {
            eprintln!("dupin: {diagnostic}");
            if let Some(hint) = &diagnostic.hint {
                eprintln!("hint: {hint}");
            }
            eprintln!("Waiting for the next change.");
        }
    }
}

/// An answer as plain text: each row's text, then its actions, indented.
fn describe_outcome(outcome: &dupin_plugin_protocol::Outcome) -> String {
    use dupin_plugin_protocol::{Effect, Outcome, PluginAction, View};
    let mut out = String::new();
    let actions = |out: &mut String, actions: &[PluginAction]| {
        for action in actions {
            let target = match &action.effect {
                Effect::Copy { text } => text.clone(),
                Effect::OpenUrl { url } => url.clone(),
                Effect::Push { view, .. } => format!("view {view}"),
            };
            out.push_str(&format!("  {}: {target}\n", action.title));
        }
    };
    match outcome {
        Outcome::Results { items, .. } if items.is_empty() => out.push_str("No results\n"),
        Outcome::Results { items, .. } => {
            for item in items {
                match &item.view {
                    View::Text { title, subtitle } if subtitle.is_empty() => {
                        out.push_str(&format!("{title}\n"))
                    }
                    View::Text { title, subtitle } => out.push_str(&format!("{title}\n  {subtitle}\n")),
                    View::Detail { title, body } => out.push_str(&format!("{title}\n  {body}\n")),
                    View::Pair { source, result } => out.push_str(&format!("{source} → {result}\n")),
                }
                actions(&mut out, &item.actions);
            }
        }
        Outcome::Detail {
            title,
            body,
            actions: offered,
        } => {
            out.push_str(&format!("{title}\n  {body}\n"));
            actions(&mut out, offered);
        }
        Outcome::Error { message, .. } => out.push_str(&format!("{message}\n")),
    }
    out
}
fn render(response: dupin_plugin_protocol::Response, output: &str) {
    use dupin_plugin_protocol::{Outcome, View};
    use rune::{App, AppSpec, Color, col, text};
    let app = App::new(AppSpec::new(
        "Dupin view",
        (680.0, 480.0),
        || unreachable!(),
        |state: &dupin_plugin_protocol::Response, _| {
            let mut body = col().w_full().h_full().px(20.0).py(20.0).fill(Color::BLACK);
            match &state.outcome {
                Outcome::Results { items, .. } => {
                    for item in items {
                        let content = match &item.view {
                            View::Text { title, subtitle } => format!("{title}\n{subtitle}"),
                            View::Detail { title, body } => format!("{title}\n{body}"),
                            View::Pair { source, result } => format!("{source}\n{result}"),
                        };
                        body = body.child(text(content).color(Color::WHITE).wrap());
                    }
                }
                Outcome::Detail {
                    title, body: detail, ..
                } => body = body.child(text(format!("{title}\n{detail}")).color(Color::WHITE).wrap()),
                Outcome::Error { message, .. } => {
                    body = body.child(text(message.clone()).color(Color::WHITE))
                }
            }
            body
        },
    ))
    .init(move || response);
    let mut args = rune::tooling::Args::default();
    args.snap = Some(output.into());
    rune::tooling::headless(app, args);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_arguments() {
        for (args, expected) in [
            (vec!["snapshots", "list", "--json"], Action::SnapshotsList),
            (vec!["snapshots", "close"], Action::SnapshotsClose),
            (vec!["snapshots", "open"], Action::SnapshotsOpen(None)),
            (
                vec!["snapshots", "open", "--id", "日本語"],
                Action::SnapshotsOpen(Some("日本語".into())),
            ),
        ] {
            assert_eq!(
                parse(&args.into_iter().map(String::from).collect::<Vec<_>>()).unwrap(),
                expected
            );
        }
        for args in [
            vec!["snapshots", "delete"],
            vec!["snapshots", "resume"],
            vec!["snapshots", "open", "--id", "-x"],
        ] {
            assert!(parse(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
        }
    }
    #[test]
    fn extension_folders_check_and_run_with_diagnostics() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let folder = |path: &str| root.join(path).to_string_lossy().into_owned();
        let word_count = folder("examples/extensions/word-count-js");
        assert_eq!(
            execute(Action::ExtCheck(word_count.clone()), true).unwrap()["runtime"],
            "javascript"
        );
        let answer = execute(Action::ExtRun(word_count, "two words".into()), true).unwrap();
        assert_eq!(answer["items"][0]["view"]["title"], "2 words");
        assert_eq!(
            answer["items"][0]["actions"][0]["effect"],
            serde_json::json!({"kind": "copy", "text": "2"})
        );

        let fixtures = "crates/dupin-js-runtime/tests/fixtures";
        for (action, code) in [
            (
                Action::ExtCheck(folder(&format!("{fixtures}/path-escape"))),
                "import-escape",
            ),
            (
                Action::ExtRun(folder(&format!("{fixtures}/throwing-search")), "x".into()),
                "search-threw",
            ),
            (
                Action::ExtRun(folder(&format!("{fixtures}/invalid-rows")), "x".into()),
                "invalid-field",
            ),
            (
                Action::ExtCheck(folder("examples/extensions")),
                "missing-manifest",
            ),
        ] {
            match execute(action, true) {
                Err(Failure::Diagnostic(diagnostic)) => {
                    assert_eq!(diagnostic.code, code);
                    assert!(
                        diagnostic.hint.is_some() && diagnostic.file.is_some(),
                        "{diagnostic:?}"
                    );
                }
                other => panic!("expected {code}, got {other:?}"),
            }
        }
    }

    #[test]
    fn development_copies_publish_only_what_the_launcher_loads() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let root = tempfile::tempdir().unwrap();
        let extensions = tempfile::tempdir().unwrap();
        let package = root.path().join("greeter");
        dupin_ext_sdk::scaffold(&package, dupin_ext_sdk::Template::JavaScript).unwrap();
        let publish = || {
            let mut outcome = None;
            dupin_ext_sdk::dev::DevCopy {
                package_dir: &package,
                extensions: extensions.path(),
                rolldown: None,
                options: dupin_ext_sdk::BuildOptions::default(),
                validate: &check_package,
            }
            .watch(std::time::Duration::ZERO, &|| false, &mut |published| {
                outcome = Some(published)
            });
            outcome.unwrap()
        };
        let link = extensions.path().join("greeter");

        assert_eq!(publish().unwrap().revision, 1);
        let loaded = crate::extensions::check_folder(&link).unwrap();
        assert_eq!((loaded.id.as_str(), loaded.runtime()), ("greeter", "javascript"));
        let folder = package.to_string_lossy().into_owned();
        assert_eq!(
            execute(Action::ExtCheck(folder.clone()), true).unwrap()["id"],
            "greeter"
        );
        assert_eq!(
            execute(Action::ExtRun(folder.clone(), "Dupin".into()), true).unwrap()["items"][0]["view"]["title"],
            "Hello, Dupin!"
        );

        let manifest = package.join("package.json");
        let original = std::fs::read_to_string(&manifest).unwrap();
        for (edit, code) in [
            (
                r#""keyword": "greeter", "capabilities": { "http": {} }"#,
                "unsupported-capability",
            ),
            (r#""keyword": "greeter", "icon": "x""#, "invalid-manifest"),
        ] {
            std::fs::write(&manifest, original.replace(r#""keyword": "greeter""#, edit)).unwrap();
            assert_eq!(publish().unwrap_err().code, code);
            assert_eq!(
                std::fs::read_link(&link).unwrap(),
                std::path::Path::new(".dev/greeter/r1")
            );
            match execute(Action::ExtBuild(folder.clone(), Default::default()), true) {
                Err(Failure::Diagnostic(diagnostic)) => assert_eq!(diagnostic.code, code),
                other => panic!("expected {code}, got {other:?}"),
            }
        }
        crate::extensions::check_folder(&link).unwrap();
    }

    #[test]
    fn plain_text_answers_list_rows_and_actions() {
        use dupin_plugin_protocol::{Effect, Outcome, PluginAction, PluginItem, View};
        let outcome = Outcome::Results {
            items: vec![PluginItem {
                id: "docs".into(),
                view: View::Text {
                    title: "Docs".into(),
                    subtitle: "Read them".into(),
                },
                actions: vec![
                    PluginAction {
                        id: "open".into(),
                        title: "Open URL".into(),
                        effect: Effect::OpenUrl {
                            url: "https://example.com/".into(),
                        },
                    },
                    PluginAction {
                        id: "copy".into(),
                        title: "Copy".into(),
                        effect: Effect::Copy { text: "docs".into() },
                    },
                ],
            }],
            state: None,
        };
        assert_eq!(
            describe_outcome(&outcome),
            "Docs\n  Read them\n  Open URL: https://example.com/\n  Copy: docs\n"
        );
        assert_eq!(
            describe_outcome(&Outcome::Results {
                items: vec![],
                state: None
            }),
            "No results\n"
        );
    }

    #[test]
    fn picker_filters_indices_without_transforming_values() {
        let values = vec![
            serde_json::json!({"action":"execute", "path":"日本語"}),
            serde_json::json!("Other"),
        ];
        assert_eq!(crate::ui::pick_indices(&values, "日本語"), vec![0]);
        assert_eq!(crate::ui::pick_indices(&values, "OTHER"), vec![1]);
        assert!(crate::ui::pick_indices(&values, "missing").is_empty());
        assert_eq!(values[0]["action"], "execute");
    }
    #[test]
    fn arguments_and_selection() {
        for flag in [
            "--size",
            "--scale",
            "--hot",
            "--video",
            "--strip",
            "--html",
            "--playground",
        ] {
            assert!(!owns_arguments(&[flag.into()]));
        }
        assert!(owns_arguments(&["pick".into()]));
        assert!(owns_arguments(&["invalid-subcommand".into()]));
        assert!(parse(&["open".into(), "--bad".into()]).is_err());
        let args = |args: &[&str]| args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse(&args(&["ext", "check", "word-count", "--json"])).unwrap(),
            Action::ExtCheck("word-count".into())
        );
        assert_eq!(
            parse(&args(&["ext", "run", "word-count", "--query", "two words"])).unwrap(),
            Action::ExtRun("word-count".into(), "two words".into())
        );
        assert_eq!(
            parse(&args(&["ext", "check", "guest.wasm", "request.json"])).unwrap(),
            Action::Ext("guest.wasm".into(), "request.json".into(), None)
        );
        assert!(parse(&args(&["ext", "run", "word-count"])).is_err());
        assert_eq!(
            parse(&args(&["ext", "new", "--ts", "greeter"])).unwrap(),
            Action::ExtNew("greeter".into(), dupin_ext_sdk::Template::TypeScript)
        );
        assert_eq!(
            parse(&args(&["ext", "new", "greeter"])).unwrap(),
            Action::ExtNew("greeter".into(), dupin_ext_sdk::Template::JavaScript)
        );
        let sourcemap = dupin_ext_sdk::BuildOptions { sourcemap: true };
        assert_eq!(
            parse(&args(&["ext", "build", "greeter", "--sourcemap", "--json"])).unwrap(),
            Action::ExtBuild("greeter".into(), sourcemap)
        );
        assert_eq!(
            parse(&args(&["ext", "dev", "greeter"])).unwrap(),
            Action::ExtDev("greeter".into(), dupin_ext_sdk::BuildOptions::default())
        );
        for invalid in [
            &["ext", "new", "greeter", "--sourcemap"][..],
            &["ext", "build", "greeter", "--ts"],
            &["ext", "check", "greeter", "--ts"],
            &["ext", "dev"],
        ] {
            assert!(parse(&args(invalid)).is_err(), "{invalid:?}");
        }
        assert!(parse(&args(&["ext", "check", "--query"])).is_err());
        {
            use crate::extension_install::Command as Packages;
            assert_eq!(
                parse(&args(&["ext", "install", "github:ryo/wc@v1", "--json"])).unwrap(),
                Action::ExtPackages(Packages::Install("github:ryo/wc@v1".into()))
            );
            assert_eq!(
                parse(&args(&["ext", "update"])).unwrap(),
                Action::ExtPackages(Packages::Update(None))
            );
            assert_eq!(
                parse(&args(&["ext", "remove", "wc"])).unwrap(),
                Action::ExtPackages(Packages::Remove("wc".into()))
            );
            assert!(parse(&args(&["ext", "install", "--upload-pack=x"])).is_err());
        }
        assert_eq!(parse(&["mcp".into()]).unwrap(), Action::Mcp);
        assert!(parse(&["mcp".into(), "--json".into()]).is_err());
        assert_eq!(
            parse(&["open".into(), "--query".into(), "日本語".into()]).unwrap(),
            Action::Open(Some("日本語".into()))
        );
        assert_eq!(
            selection("日本語\n🦀\n".as_bytes(), false).unwrap(),
            vec![serde_json::json!("日本語"), serde_json::json!("🦀")]
        );
        assert_eq!(
            selection(br#"[{"path":"/untrusted","action":"execute"}]"#, true).unwrap()[0]["action"],
            "execute"
        );
        assert!(selection(&[255], false).is_err());
        assert!(selection(b"[]", true).is_err());
    }
}
