use crate::control::{self, Command, Request, exchange};
use serde_json::Value;
use std::io::{self, Read};
const HELP: &str = "dupin open [--query QUERY] [--json]\ndupin pick [--input-json] [--json]\ndupin snapshots list|close [--json]\ndupin snapshots open [--id ID] [--json]\ndupin config get [--json]\ndupin config set BASE.json DESIRED.json [--json]\ndupin snap VIEW.json OUTPUT.png [--json]\ndupin ext check DIR [--json]\ndupin ext run DIR --query QUERY [--json]\ndupin ext check MODULE.wasm REQUEST.json [--json]\ndupin ext snap MODULE.wasm REQUEST.json OUTPUT.png [--json]\ndupin mcp\n\nNo subcommand starts the resident launcher. Existing Rune tooling flags remain available.";
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
    let mut parser = pico_args::Arguments::from_vec(args.iter().map(std::ffi::OsString::from).collect());
    let json = parser.contains("--json");
    let rest = parser.finish();
    let args: Vec<_> = rest
        .iter()
        .map(|s| s.to_str().ok_or_else(|| "arguments must be UTF-8".to_owned()))
        .collect::<Result<_, _>>()?;
    match args.as_slice() {
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
            let extension = crate::extensions::load_folder(std::path::Path::new(&folder))?;
            extension.check(std::time::Instant::now() + crate::extensions::QUERY_DEADLINE)?;
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
        Action::Mcp => crate::mcp::run()
            .map(|()| Value::Null)
            .map_err(|e| e.to_string().into()),
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
        assert!(parse(&args(&["ext", "check", "--query"])).is_err());
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
