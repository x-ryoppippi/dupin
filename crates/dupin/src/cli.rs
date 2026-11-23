use crate::control::{self, Command, Reply, Request};
use serde_json::Value;
use std::io::{self, Read};
const HELP: &str = "dupin open [--query QUERY] [--json]\ndupin pick [--input-json] [--json]\ndupin config get [--json]\ndupin config set BASE.json DESIRED.json [--json]\ndupin snap VIEW.json OUTPUT.png [--json]\ndupin ext check MODULE.wasm REQUEST.json [--json]\ndupin ext snap MODULE.wasm REQUEST.json OUTPUT.png [--json]\n\nNo subcommand starts the resident launcher. Existing Rune tooling flags remain available.";
#[derive(Debug, PartialEq)]
enum Action {
    Help,
    Version,
    Open(Option<String>),
    Pick(bool),
    Get,
    Set(String, String),
    Snap(String, String),
    Ext(String, String, Option<String>),
}
fn parse(args: &[String]) -> Result<Action, String> {
    let mut parser = pico_args::Arguments::from_vec(args.iter().map(std::ffi::OsString::from).collect());
    let _json = parser.contains("--json");
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
        ["pick"] => Ok(Action::Pick(false)),
        ["pick", "--input-json"] => Ok(Action::Pick(true)),
        ["config", "get"] => Ok(Action::Get),
        ["config", "set", base, desired] if !base.starts_with('-') && !desired.starts_with('-') => {
            Ok(Action::Set(base.to_string(), desired.to_string()))
        }
        ["snap", view, out] if !view.starts_with('-') && !out.starts_with('-') => {
            Ok(Action::Snap(view.to_string(), out.to_string()))
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
fn exchange(command: Command) -> Result<Value, String> {
    let request = Request { id: 1, command };
    control::validate(&request).map_err(|e| e.to_string())?;
    let mut stream = control::connect().map_err(|e| e.to_string())?;
    control::write_frame(&mut stream, &request).map_err(|e| e.to_string())?;
    let reply: Reply = control::read_frame(&mut stream).map_err(|e| e.to_string())?;
    if reply.id != request.id {
        return Err("control correlation mismatch".into());
    }
    reply.result
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
    // Rune owns its desktop tooling flags; named commands are parsed before any app work.
    if [
        "--script",
        "--snap",
        "--window",
        "--list-windows",
        "--tree",
        "--trace",
        "--desktop",
        "--native",
        "--scene",
        "--list-scenes",
    ]
    .contains(&args[0].as_str())
    {
        return false;
    }
    let json = args.iter().any(|v| v == "--json");
    let action = parse(&args);
    let picked = matches!(action, Ok(Action::Pick(_)));
    let result = action.and_then(|action| {
        let output = match &action {
            Action::Snap(_, output) | Action::Ext(_, _, Some(output)) if json => Some(output.clone()),
            _ => None,
        };
        if let Some(output) = output {
            isolated_render(&args, &output)
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
        Err(error) => {
            if json {
                println!("{}", serde_json::json!({"error": error}));
            } else {
                eprintln!("dupin: {error}");
            }
            std::process::exit(1)
        }
    }
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

fn execute(action: Action, json: bool) -> Result<Value, String> {
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
        Action::Open(query) => exchange(Command::Open { query }),
        Action::Pick(json_input) => exchange(Command::Pick {
            values: selection(&bounded(io::stdin().lock(), control::MAX_FRAME / 2)?, json_input)?,
        }),
        Action::Get => exchange(Command::ConfigGet {}),
        Action::Set(base, desired) => exchange(Command::ConfigSet {
            base: serde_json::from_slice(&file(&base, 65536)?).map_err(|_| "invalid base configuration")?,
            desired: serde_json::from_slice(&file(&desired, 65536)?)
                .map_err(|_| "invalid desired configuration")?,
        }),
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
    }
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
        assert!(parse(&["open".into(), "--bad".into()]).is_err());
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
