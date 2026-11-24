//! `dupin mcp`: the Model Context Protocol over stdio, one JSON-RPC message per line. Each tool
//! is a client of the same local control socket as the `dupin` commands, or reads configuration
//! as the launcher does; none writes configuration, runs a script or opens a file. With
//! `--workspace`, the extension tools check, run, test and render packages inside that folder
//! offline, with fixture replies only, and never write to it.
use crate::control::{self, Command};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// Newest first; an unknown requested version is answered with the newest.
const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_MESSAGE: usize = control::MAX_FRAME;
const MAX_CONFIG: usize = 64 * 1024;
const INSTRUCTIONS: &str = "Dupin is a macOS launcher. These tools read and validate its config.toml and show its panel; \
     open_panel and pick need the launcher running. They never write configuration, run scripts or open files. \
     Started with --workspace, ext_check, ext_run, ext_test and ext_snap check, run, replay the fixtures of and render \
     extension packages in that folder, offline and without writing to it.";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What the tools act on: the running launcher and the config file.
pub trait Host: Sync {
    fn control(&self, command: Command, cancel: &Cancel) -> Result<Value, String>;
    fn config_file(&self) -> Result<Value, String>;
    /// The folder, canonical, whose extension packages the extension tools may use.
    fn workspace(&self) -> Option<&Path> {
        None
    }
}

/// The running launcher, reached through its control socket.
struct Local {
    workspace: Option<PathBuf>,
}
impl Host for Local {
    fn workspace(&self) -> Option<&Path> {
        self.workspace.as_deref()
    }
    fn control(&self, command: Command, cancel: &Cancel) -> Result<Value, String> {
        let pending = control::send(command)?;
        cancel.on_cancel(pending.abort_handle()?);
        pending.wait()
    }
    fn config_file(&self) -> Result<Value, String> {
        config_files(&crate::settings::Paths::discover())
    }
}

fn config_files(paths: &crate::settings::Paths) -> Result<Value, String> {
    use crate::settings::{LockedKeys, read_layer, top_level, writable};

    let base = paths
        .base
        .as_deref()
        .ok_or("no config path: neither XDG_CONFIG_HOME nor HOME is set")?;
    let source = read_layer(Some(base), "config.toml")?;
    let read_only = !writable(base);
    let locked_keys = if read_only {
        top_level(source.as_deref().unwrap_or_default(), "config.toml")
            .ok()
            .map(|table| table.keys().cloned().collect::<LockedKeys>())
    } else {
        Some(LockedKeys::new())
    };
    let overlay = paths.overlay.as_deref();
    let overlay_source = read_layer(overlay, "settings.toml")?;
    Ok(json!({
        "path": base.display().to_string(),
        "exists": source.is_some(),
        "readOnly": read_only,
        "source": source,
        "lockedKeys": locked_keys,
        "overlay": {
            "path": overlay.map(|path| path.display().to_string()),
            "exists": overlay_source.is_some(),
            "readOnly": overlay.is_some_and(|path| !writable(path)),
            "source": overlay_source,
            "active": read_only,
        }
    }))
}

pub fn run(workspace: Option<PathBuf>) -> io::Result<()> {
    let workspace = match workspace {
        Some(folder) => {
            let canonical = folder.canonicalize()?;
            if !canonical.is_dir() {
                return Err(io::Error::other(format!("{} is not a folder", folder.display())));
            }
            Some(canonical)
        }
        None => None,
    };
    serve(io::stdin().lock(), &mut io::stdout(), &Local { workspace })
}

/// Aborts a tool call's launcher command, when the client cancels the call or goes away.
#[derive(Clone, Default)]
pub struct Cancel(Arc<Mutex<Cancellation>>);
#[derive(Default)]
struct Cancellation {
    aborted: bool,
    by_client: bool,
    abort: Option<Box<dyn FnOnce() + Send>>,
}
impl Cancel {
    /// Run `abort` when the call is aborted, or now if it already was.
    pub fn on_cancel(&self, abort: impl FnOnce() + Send + 'static) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.aborted {
            drop(state);
            abort();
        } else {
            state.abort = Some(Box::new(abort));
        }
    }
    fn abort(&self, by_client: bool) {
        let abort = {
            let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            state.aborted = true;
            state.by_client |= by_client;
            state.abort.take()
        };
        if let Some(abort) = abort {
            abort();
        }
    }
    /// Whether the call was aborted, for work that polls instead of registering an abort.
    fn is_aborted(&self) -> bool {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).aborted
    }
    /// A call the client cancelled gets no response, as MCP asks.
    fn by_client(&self) -> bool {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).by_client
    }
}

/// Serve until `input` ends. Tool calls run concurrently, so a pick waiting on the user does not
/// hold up pings or cancellation. When input ends, the launcher commands still running are
/// aborted, so no picker outlives its client, and every call is answered before returning.
pub fn serve(mut input: impl BufRead, output: &mut (impl Write + Send), host: &impl Host) -> io::Result<()> {
    let output = Mutex::new(output);
    let calls: Mutex<HashMap<String, Cancel>> = Mutex::new(HashMap::new());
    std::thread::scope(|scope| {
        let result = loop {
            let message = match read_line(&mut input, MAX_MESSAGE) {
                Ok(None) => break Ok(()),
                Ok(Some(Line::Oversized)) => Err(error(Value::Null, INVALID_REQUEST, "message too large")),
                Ok(Some(Line::Message(bytes))) if bytes.iter().all(u8::is_ascii_whitespace) => continue,
                Ok(Some(Line::Message(bytes))) => serde_json::from_slice::<Value>(&bytes)
                    .map_err(|_| error(Value::Null, PARSE_ERROR, "invalid JSON")),
                Err(e) => break Err(e),
            };
            let reply = match message.and_then(classify) {
                Ok(Message::Request { id, method, params }) if method == "tools/call" => {
                    match tool_call(params, host.workspace().is_some()) {
                        Ok((name, arguments)) => {
                            let key = id.to_string();
                            let cancel = Cancel::default();
                            let mut running = calls.lock().unwrap_or_else(PoisonError::into_inner);
                            if running.contains_key(&key) {
                                Some(error(id, INVALID_REQUEST, "request id already in use"))
                            } else {
                                running.insert(key.clone(), cancel.clone());
                                let (output, calls) = (&output, &calls);
                                scope.spawn(move || {
                                    let result = call_tool(host, &name, &arguments, &cancel);
                                    calls.lock().unwrap_or_else(PoisonError::into_inner).remove(&key);
                                    if !cancel.by_client() {
                                        let _ = write(output, &success(id, tool_result(result)));
                                    }
                                });
                                None
                            }
                        }
                        Err(message) => Some(error(id, INVALID_PARAMS, &message)),
                    }
                }
                Ok(Message::Request { id, method, params }) => Some(match respond(&method, &params, host) {
                    Ok(result) => success(id, result),
                    Err((code, message)) => error(id, code, &message),
                }),
                Ok(Message::Notification { method, params }) => {
                    if method == "notifications/cancelled"
                        && let Some(id) = params.get("requestId")
                        && let Some(cancel) = calls
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .get(&id.to_string())
                    {
                        cancel.abort(true);
                    }
                    None
                }
                Ok(Message::Response) => None,
                Err(reply) => Some(reply),
            };
            if let Some(reply) = reply
                && let Err(e) = write(&output, &reply)
            {
                break Err(e);
            }
        };
        for cancel in calls.lock().unwrap_or_else(PoisonError::into_inner).values() {
            cancel.abort(false);
        }
        result
    })
}

enum Line {
    Message(Vec<u8>),
    Oversized,
}

/// The next line without its terminator, reading past one longer than `cap` without keeping it.
fn read_line(input: &mut impl BufRead, cap: usize) -> io::Result<Option<Line>> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let buffer = match input.fill_buf() {
            Ok(buffer) => buffer,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buffer.is_empty() {
            return Ok((oversized || !line.is_empty()).then_some(if oversized {
                Line::Oversized
            } else {
                Line::Message(line)
            }));
        }
        let end = buffer.iter().position(|byte| *byte == b'\n');
        let chunk = &buffer[..end.unwrap_or(buffer.len())];
        if !oversized {
            if chunk.len() > cap - line.len() {
                oversized = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(chunk);
            }
        }
        let consumed = end.map_or(buffer.len(), |end| end + 1);
        input.consume(consumed);
        if end.is_some() {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(Some(if oversized {
                Line::Oversized
            } else {
                Line::Message(line)
            }));
        }
    }
}

enum Message {
    Request {
        id: Value,
        method: String,
        params: Map<String, Value>,
    },
    Notification {
        method: String,
        params: Map<String, Value>,
    },
    Response,
}

/// Sort a JSON-RPC 2.0 message, or the error response it earns. MCP has no batches and no null
/// request IDs.
fn classify(message: Value) -> Result<Message, Value> {
    let Value::Object(mut message) = message else {
        return Err(error(Value::Null, INVALID_REQUEST, "expected a JSON-RPC object"));
    };
    let id = match message.remove("id") {
        None => None,
        Some(id @ Value::String(_)) => Some(id),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Some(Value::Number(n)),
        Some(_) => {
            return Err(error(
                Value::Null,
                INVALID_REQUEST,
                "id must be a string or an integer",
            ));
        }
    };
    let invalid = |message: &str| error(id.clone().unwrap_or(Value::Null), INVALID_REQUEST, message);
    if message.get("jsonrpc") != Some(&json!("2.0")) {
        return Err(invalid("jsonrpc must be \"2.0\""));
    }
    let method = match message.remove("method") {
        Some(Value::String(method)) => method,
        None if id.is_some() && (message.contains_key("result") || message.contains_key("error")) => {
            return Ok(Message::Response);
        }
        _ => return Err(invalid("method must be a string")),
    };
    let params = match message.remove("params") {
        None => Map::new(),
        Some(Value::Object(params)) => params,
        Some(_) => {
            return Err(error(
                id.unwrap_or(Value::Null),
                INVALID_PARAMS,
                "params must be an object",
            ));
        }
    };
    Ok(match id {
        Some(id) => Message::Request { id, method, params },
        None => Message::Notification { method, params },
    })
}

fn respond(method: &str, params: &Map<String, Value>, host: &impl Host) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str);
            let version = VERSIONS
                .into_iter()
                .find(|version| Some(*version) == requested)
                .unwrap_or(VERSIONS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {
                    "name": "dupin",
                    "title": "Dupin",
                    "version": include_str!("../../../VERSION").trim(),
                },
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools(host.workspace().is_some())})),
        _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
    }
}

/// The tool a call names and its arguments. Arguments that a known tool rejects are its own
/// error result, so a model can correct them; an unknown tool is a protocol error.
fn tool_call(
    mut params: Map<String, Value>,
    workspace: bool,
) -> Result<(String, Map<String, Value>), String> {
    let Some(Value::String(name)) = params.remove("name") else {
        return Err("name must be a string".into());
    };
    if !tools(workspace).iter().any(|tool| tool["name"] == name.as_str()) {
        return Err(format!("unknown tool: {name}"));
    }
    match params.remove("arguments") {
        None => Ok((name, Map::new())),
        Some(Value::Object(arguments)) => Ok((name, arguments)),
        Some(_) => Err("arguments must be an object".into()),
    }
}

fn tools(workspace: bool) -> Vec<Value> {
    let none = json!({"type": "object", "properties": {}, "additionalProperties": false});
    let read_only = json!({"readOnlyHint": true, "openWorldHint": false});
    let mut tools = vec![
        json!({
            "name": "read_config",
            "title": "Read configuration",
            "description": "Reports config.toml and the settings.toml overlay, their paths and TOML sources, and which top-level keys are locked. Works whether or not Dupin is running.",
            "inputSchema": none,
            "annotations": read_only,
        }),
        json!({
            "name": "validate_config",
            "title": "Validate configuration",
            "description": "Check a complete config.toml source as Dupin would load it, without writing anything. Returns valid with the resulting values, or the problem with its line and column.",
            "inputSchema": {
                "type": "object",
                "properties": {"source": {"type": "string", "description": "The TOML source of a whole config.toml.", "maxLength": MAX_CONFIG}},
                "required": ["source"],
                "additionalProperties": false,
            },
            "annotations": read_only,
        }),
        json!({
            "name": "get_active_config",
            "title": "Get active configuration",
            "description": "The configuration values the running launcher has applied, as `dupin config get` prints them.",
            "inputSchema": none,
            "annotations": read_only,
        }),
        json!({
            "name": "open_panel",
            "title": "Open the panel",
            "description": "Show Dupin's panel, optionally with a query typed into it. Nothing is activated; the user chooses what to do.",
            "inputSchema": {
                "type": "object",
                "properties": {"query": {"type": "string", "description": "Text to put in the search field, such as `in invoice`."}},
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "pick",
            "title": "Ask the user to pick",
            "description": "Show values in Dupin's panel and wait, up to five minutes, for the user to choose one. Returns the chosen value unchanged; fails with Cancelled if the user dismisses the panel. Strings show as they are; other JSON values show as JSON.",
            "inputSchema": {
                "type": "object",
                "properties": {"values": {"type": "array", "minItems": 1, "maxItems": 1000, "description": "The choices, any JSON values."}},
                "required": ["values"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false},
        }),
    ];
    if workspace {
        tools.extend(extension_tools(&read_only));
    }
    tools
}

fn extension_tools(read_only: &Value) -> [Value; 4] {
    let package = json!({"type": "string", "description": "The package's folder, relative to the workspace, such as `word-count`."});
    let schema = |properties: Value, required: &[&str]| json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false});
    [
        json!({
            "name": "ext_check",
            "title": "Check an extension",
            "description": "Load a prebuilt extension package as the launcher does, compile it in the sandboxed extension host and confirm it exports `search`. Returns its id, name, version and keyword, or a diagnostic with a code, file, field, position and hint.",
            "inputSchema": schema(json!({"package": package}), &["package"]),
            "annotations": read_only,
        }),
        json!({
            "name": "ext_run",
            "title": "Run an extension",
            "description": "Ask a prebuilt extension package one query in the sandboxed extension host and return its rows and their actions, as `dupin ext run --json` prints them. Nothing is activated.",
            "inputSchema": schema(json!({"package": package, "query": {"type": "string", "maxLength": dupin_ext_sdk::fixture::MAX_QUERY_BYTES}}), &["package", "query"]),
            "annotations": read_only,
        }),
        json!({
            "name": "ext_test",
            "title": "Test an extension",
            "description": "Replay every fixture in the package's fixtures/ folder with its fixed clock, seed and time zone and its synthetic host-service replies, and report each step's outcome and how it differs from the fixture's expectations. Never rewrites the fixtures.",
            "inputSchema": schema(json!({"package": package}), &["package"]),
            "annotations": read_only,
        }),
        json!({
            "name": "ext_snap",
            "title": "Render an extension's panel",
            "description": "Render the panel after a fixture's step (the last by default) with Dupin's own view, headless, and return it as a PNG. Needs a Dupin build with headless rendering.",
            "inputSchema": schema(json!({
                "package": package,
                "fixture": {"type": "string", "description": "The fixture, relative to the package, such as `fixtures/search.json`."},
                "step": {"type": "integer", "minimum": 1, "maximum": dupin_ext_sdk::fixture::MAX_STEPS},
            }), &["package", "fixture"]),
            "annotations": read_only,
        }),
    ]
}

/// A tool's answer: JSON, or JSON with the image it describes.
enum Output {
    Json(Value),
    Png(Value, Vec<u8>),
}

/// `relative` inside `root`, following links, so a tool can't reach outside the workspace.
fn inside(root: &Path, relative: Option<&Value>, what: &str) -> Result<PathBuf, String> {
    let Some(Value::String(relative)) = relative else {
        return Err(format!("{what} must be a string"));
    };
    let path = Path::new(relative);
    if relative.is_empty() || path.is_absolute() {
        return Err(format!("{what} must be a path relative to the workspace"));
    }
    let resolved = root
        .join(path)
        .canonicalize()
        .map_err(|error| format!("{what} {relative:?}: {error}"))?;
    if !resolved.starts_with(root) {
        return Err(format!("{what} {relative:?} is outside the workspace"));
    }
    Ok(resolved)
}

fn diagnostic(diagnostic: dupin_js_runtime::Diagnostic) -> String {
    json!({"error": diagnostic.to_string(), "diagnostic": diagnostic}).to_string()
}

fn extension_tool(
    workspace: &Path,
    name: &str,
    arguments: &Map<String, Value>,
    cancel: &Cancel,
) -> Result<Output, String> {
    let cancelled = {
        let cancel = cancel.clone();
        move || cancel.is_aborted()
    };
    let package = inside(workspace, arguments.get("package"), "package")?;
    match name {
        "ext_check" => {
            let extension = crate::extensions::check_folder(&package).map_err(diagnostic)?;
            Ok(Output::Json(json!({
                "id": extension.id,
                "name": extension.name,
                "version": extension.version,
                "keyword": extension.keyword,
            })))
        }
        "ext_run" => {
            let Some(Value::String(query)) = arguments.get("query") else {
                return Err("query must be a string".into());
            };
            if query.len() > dupin_ext_sdk::fixture::MAX_QUERY_BYTES {
                return Err("query too large".into());
            }
            let extension = crate::extensions::load_folder(&package).map_err(diagnostic)?;
            let batches = crate::extensions::answer(
                &extension,
                query,
                None,
                dupin_js_runtime::MAX_ROWS,
                std::time::Instant::now() + crate::extensions::QUERY_DEADLINE,
                cancelled.clone(),
                &(std::sync::Arc::new(dupin_extension_host::NoServices) as _),
            )
            .map_err(|error| match error {
                dupin_js_runtime::RunError::Failed(failure) => diagnostic(failure),
                dupin_js_runtime::RunError::Cancelled => "the query was cancelled".into(),
            })?;
            Ok(Output::Json(crate::cli::answer_json(&batches)))
        }
        "ext_test" => Ok(Output::Json(
            crate::extensions::test(&package, false, &cancelled)
                .map_err(diagnostic)?
                .json,
        )),
        "ext_snap" => {
            let fixture = inside(&package, arguments.get("fixture"), "fixture")?;
            let step = match arguments.get("step") {
                None => None,
                Some(step) => Some(
                    step.as_u64()
                        .filter(|step| *step >= 1)
                        .ok_or("step must be a step number, counting from 1")? as usize,
                ),
            };
            let scratch = tempfile::Builder::new()
                .prefix("dupin-mcp-snap-")
                .tempdir()
                .map_err(|error| format!("couldn't make a scratch folder: {error}"))?;
            let output = scratch.path().join("panel.png");
            let snapped =
                crate::extensions::snap(&package, &fixture, step, &output, &cancelled).map_err(diagnostic)?;
            let png = std::fs::read(&output).map_err(|error| format!("couldn't read the render: {error}"))?;
            let mut described = crate::cli::snapped_json(&snapped);
            if let Some(described) = described.as_object_mut() {
                described.remove("png");
            }
            Ok(Output::Png(described, png))
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

fn call_tool(
    host: &impl Host,
    name: &str,
    arguments: &Map<String, Value>,
    cancel: &Cancel,
) -> Result<Output, String> {
    let allowed: &[&str] = match name {
        "validate_config" => &["source"],
        "open_panel" => &["query"],
        "pick" => &["values"],
        "ext_check" | "ext_test" => &["package"],
        "ext_run" => &["package", "query"],
        "ext_snap" => &["package", "fixture", "step"],
        _ => &[],
    };
    if let Some(unexpected) = arguments.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("unexpected argument: {unexpected}"));
    }
    if name.starts_with("ext_") {
        let workspace = host
            .workspace()
            .ok_or("the extension tools need `dupin mcp --workspace DIR`")?;
        return extension_tool(workspace, name, arguments, cancel);
    }
    let result = match name {
        "read_config" => host.config_file(),
        "validate_config" => {
            let Some(Value::String(source)) = arguments.get("source") else {
                return Err("source must be a string".into());
            };
            if source.len() > MAX_CONFIG {
                return Err("source too large".into());
            }
            Ok(match crate::settings::check(source) {
                Ok(config) => json!({"valid": true, "config": config}),
                Err(problem) => json!({"valid": false, "problem": problem}),
            })
        }
        "get_active_config" => host.control(Command::ConfigGet {}, cancel),
        "open_panel" => {
            let query = match arguments.get("query") {
                None => None,
                Some(Value::String(query)) => Some(query.clone()),
                Some(_) => return Err("query must be a string".into()),
            };
            host.control(Command::Open { query }, cancel)?;
            Ok(json!({"shown": true}))
        }
        "pick" => {
            let Some(Value::Array(values)) = arguments.get("values") else {
                return Err("values must be an array".into());
            };
            let value = host.control(
                Command::Pick {
                    values: values.clone(),
                },
                cancel,
            )?;
            Ok(json!({"value": value}))
        }
        _ => Err(format!("unknown tool: {name}")),
    };
    result.map(Output::Json)
}

fn tool_result(result: Result<Output, String>) -> Value {
    match result {
        Ok(Output::Json(value)) => json!({
            "content": [{"type": "text", "text": value.to_string()}],
            "structuredContent": value,
            "isError": false,
        }),
        Ok(Output::Png(value, png)) => json!({
            "content": [
                {"type": "image", "data": base64(&png), "mimeType": "image/png"},
                {"type": "text", "text": value.to_string()},
            ],
            "structuredContent": value,
            "isError": false,
        }),
        Err(message) => json!({
            "content": [{"type": "text", "text": message}],
            "isError": true,
        }),
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, byte)| n | u32::from(*byte) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn write(output: &Mutex<&mut (impl Write + Send)>, message: &Value) -> io::Result<()> {
    let mut output = output.lock().unwrap_or_else(PoisonError::into_inner);
    serde_json::to_writer(&mut **output, message)?;
    output.write_all(b"\n")?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// A launcher that records commands and answers from a script, and a config file that
    /// exists only in memory.
    #[derive(Default)]
    struct Fake {
        commands: Mutex<Vec<String>>,
        reply: Option<Result<Value, String>>,
        /// For a pick: say it started, then block until cancelled and say that too.
        blocking: Option<Mutex<(mpsc::Sender<()>, mpsc::Sender<()>)>>,
        workspace: Option<PathBuf>,
    }
    impl Host for Fake {
        fn workspace(&self) -> Option<&Path> {
            self.workspace.as_deref()
        }
        fn control(&self, command: Command, cancel: &Cancel) -> Result<Value, String> {
            self.commands
                .lock()
                .unwrap()
                .push(serde_json::to_string(&command).unwrap());
            if let (Command::Pick { .. }, Some(signals)) = (&command, &self.blocking) {
                let (started, aborted) = signals.lock().unwrap().clone();
                let (wake, woken) = mpsc::channel();
                cancel.on_cancel(move || {
                    let _ = wake.send(());
                });
                started.send(()).unwrap();
                woken.recv().unwrap();
                aborted.send(()).unwrap();
                return Err("control request cancelled".into());
            }
            self.reply.clone().unwrap_or(Ok(Value::Null))
        }
        fn config_file(&self) -> Result<Value, String> {
            Ok(
                json!({"path": "/fixture/config.toml", "exists": true, "readOnly": false, "source": "file_limit = 30\n"}),
            )
        }
    }

    fn exchange(host: &Fake, input: &str) -> Vec<Value> {
        let mut output = Vec::new();
        serve(input.as_bytes(), &mut output, host).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Like [`exchange`], but keeps input open until `replies` have arrived, as a client waiting
    /// on its calls does; closing input aborts the calls still running.
    fn converse(host: &Fake, input: &str, replies: usize) -> Vec<Value> {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let reader = server.try_clone().unwrap();
        std::thread::scope(|scope| {
            let serving = scope.spawn(move || serve(BufReader::new(reader), &mut server, host).unwrap());
            client.write_all(input.as_bytes()).unwrap();
            let received = BufReader::new(client.try_clone().unwrap())
                .lines()
                .take(replies)
                .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
                .collect();
            drop(client);
            serving.join().unwrap();
            received
        })
    }

    fn call(id: u64, name: &str, arguments: Value) -> String {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
            .to_string()
            + "\n"
    }

    #[test]
    fn initializes_lists_tools_and_ignores_notifications() {
        let replies = exchange(
            &Fake::default(),
            concat!(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
                "\n",
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                "\n\n",
                r#"{"jsonrpc":"2.0","id":"two","method":"ping"}"#,
                "\r\n",
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}"#,
                "\n",
                r#"{"jsonrpc":"2.0","id":4,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
            ),
        );
        assert_eq!(replies.len(), 4);
        assert_eq!(replies[0]["id"], 1);
        assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], "dupin");
        assert!(replies[0]["result"]["capabilities"]["tools"].is_object());
        assert_eq!(replies[1], json!({"jsonrpc": "2.0", "id": "two", "result": {}}));
        let names: Vec<_> = replies[2]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "read_config",
                "validate_config",
                "get_active_config",
                "open_panel",
                "pick"
            ]
        );
        assert_eq!(replies[3]["result"]["protocolVersion"], VERSIONS[0]);
    }

    #[test]
    fn malformed_messages_earn_json_rpc_errors_and_serving_continues() {
        let oversized = format!("{}\n", "x".repeat(MAX_MESSAGE + 1));
        let input = [
            "{\n",
            "[]\n",
            r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"ping","params":[]}"#,
            "\n",
            &oversized,
            r#"{"jsonrpc":"2.0","id":4,"result":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"write_config"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"read_config","arguments":[]}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#,
        ]
        .concat();
        let replies = exchange(&Fake::default(), &input);
        let codes: Vec<_> = replies
            .iter()
            .map(|reply| (reply["id"].clone(), reply["error"]["code"].clone()))
            .collect();
        assert_eq!(
            codes,
            [
                (Value::Null, json!(PARSE_ERROR)),
                (Value::Null, json!(INVALID_REQUEST)),
                (json!(1), json!(INVALID_REQUEST)),
                (Value::Null, json!(INVALID_REQUEST)),
                (json!(2), json!(METHOD_NOT_FOUND)),
                (json!(3), json!(INVALID_PARAMS)),
                (Value::Null, json!(INVALID_REQUEST)),
                (json!(5), json!(INVALID_PARAMS)),
                (json!(6), json!(INVALID_PARAMS)),
                (json!(7), Value::Null),
            ]
        );
    }

    #[test]
    fn config_tools_read_and_validate_without_the_launcher() {
        let host = Fake::default();
        let input = [
            call(1, "read_config", json!({})),
            call(2, "validate_config", json!({"source": "file_limit = 30\n"})),
            call(3, "validate_config", json!({"source": "file_limit = \"many\"\n"})),
            call(4, "validate_config", json!({"source": 1})),
            call(5, "read_config", json!({"path": "/etc/passwd"})),
            call(
                6,
                "validate_config",
                json!({"source": "#".repeat(MAX_CONFIG + 1)}),
            ),
        ]
        .concat();
        let mut replies = exchange(&host, &input);
        replies.sort_by_key(|reply| reply["id"].as_u64());
        let result = |n: usize| &replies[n]["result"];
        assert_eq!(result(0)["structuredContent"]["source"], "file_limit = 30\n");
        assert_eq!(result(0)["isError"], false);
        assert_eq!(
            serde_json::from_str::<Value>(result(0)["content"][0]["text"].as_str().unwrap()).unwrap(),
            result(0)["structuredContent"]
        );
        assert_eq!(result(1)["structuredContent"]["valid"], true);
        assert_eq!(result(1)["structuredContent"]["config"]["file_limit"], 30);
        assert_eq!(result(2)["structuredContent"]["valid"], false);
        assert!(
            result(2)["structuredContent"]["problem"]
                .as_str()
                .unwrap()
                .starts_with("line 1, column 14: ")
        );
        for reply in &replies[3..6] {
            assert_eq!(reply["result"]["isError"], true, "{reply}");
        }
        assert_eq!(result(4)["content"][0]["text"], "unexpected argument: path");
        assert!(host.commands.lock().unwrap().is_empty());
    }

    #[test]
    fn launcher_tools_send_control_commands_and_report_failures_as_tool_errors() {
        let host = Fake::default();
        let replies = exchange(
            &host,
            &[
                call(1, "open_panel", json!({"query": "in invoice"})),
                call(2, "open_panel", json!({"query": 3})),
            ]
            .concat(),
        );
        assert_eq!(replies.len(), 2);
        assert_eq!(
            *host.commands.lock().unwrap(),
            [r#"{"kind":"open","query":"in invoice"}"#]
        );
        let not_running = Fake {
            reply: Some(Err("Dupin is not running".into())),
            ..Fake::default()
        };
        let replies = exchange(
            &not_running,
            &[
                call(1, "get_active_config", json!({})),
                call(2, "pick", json!({"values": ["a", {"b": 1}]})),
            ]
            .concat(),
        );
        for reply in &replies {
            assert_eq!(reply["result"]["isError"], true);
            assert_eq!(reply["result"]["content"][0]["text"], "Dupin is not running");
        }
        assert!(
            not_running
                .commands
                .lock()
                .unwrap()
                .contains(&r#"{"kind":"pick","values":["a",{"b":1}]}"#.to_owned())
        );
        let picked = Fake {
            reply: Some(Ok(json!({"b": 1}))),
            ..Fake::default()
        };
        let replies = exchange(&picked, &call(1, "pick", json!({"values": ["a", {"b": 1}]})));
        assert_eq!(
            replies[0]["result"]["structuredContent"],
            json!({"value": {"b": 1}})
        );
    }

    #[test]
    fn client_cancellation_goes_unanswered_and_closing_input_aborts_the_rest() {
        let (started_tx, started) = mpsc::channel();
        let (aborted_tx, aborted) = mpsc::channel();
        let host = Fake {
            blocking: Some(Mutex::new((started_tx, aborted_tx))),
            ..Fake::default()
        };
        let (mut client, server) = UnixStream::pair().unwrap();
        let replies = std::thread::scope(|scope| {
            let serving = scope.spawn(|| {
                let mut output = Vec::new();
                serve(BufReader::new(server), &mut output, &host).unwrap();
                output
            });
            client
                .write_all(call(9, "pick", json!({"values": ["a"]})).as_bytes())
                .unwrap();
            started.recv_timeout(Duration::from_secs(5)).unwrap();
            client
                .write_all(call(9, "pick", json!({"values": ["b"]})).as_bytes())
                .unwrap();
            client
                .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":9}}\n")
                .unwrap();
            aborted.recv_timeout(Duration::from_secs(5)).unwrap();
            client
                .write_all(call(10, "pick", json!({"values": ["c"]})).as_bytes())
                .unwrap();
            started.recv_timeout(Duration::from_secs(5)).unwrap();
            drop(client);
            aborted.recv_timeout(Duration::from_secs(5)).unwrap();
            serving.join().unwrap()
        });
        let replies: Vec<Value> = String::from_utf8(replies)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(replies.len(), 2, "{replies:?}");
        assert_eq!(replies[0]["id"], 9);
        assert_eq!(replies[0]["error"]["code"], INVALID_REQUEST);
        assert_eq!(replies[1]["id"], 10);
        assert_eq!(replies[1]["result"]["isError"], true);
    }

    #[test]
    fn extension_tools_need_a_workspace_and_stay_inside_it() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let replies = exchange(
            &Fake::default(),
            &call(1, "ext_check", json!({"package": "greeter"})),
        );
        assert_eq!(replies[0]["error"]["code"], INVALID_PARAMS);

        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        dupin_ext_sdk::scaffold(&root.join("greeter"), dupin_ext_sdk::Template::JavaScript).unwrap();
        let outside = tempfile::tempdir().unwrap();
        dupin_ext_sdk::scaffold(
            &outside.path().join("elsewhere"),
            dupin_ext_sdk::Template::JavaScript,
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path().join("elsewhere"), root.join("linked")).unwrap();
        let host = Fake {
            workspace: Some(root.clone()),
            ..Fake::default()
        };
        let fixture = root.join("greeter/fixtures/search.json");
        let before = std::fs::read(&fixture).unwrap();
        let mut replies = converse(
            &host,
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.to_owned() + "\n",
                call(2, "ext_check", json!({"package": "greeter"})),
                call(3, "ext_run", json!({"package": "greeter", "query": "Dupin"})),
                call(4, "ext_test", json!({"package": "greeter"})),
                call(5, "ext_check", json!({"package": "../"})),
                call(6, "ext_check", json!({"package": "linked"})),
                call(7, "ext_check", json!({"package": "/etc"})),
                call(8, "ext_snap", json!({"package": "greeter", "fixture": "../../"})),
                call(9, "ext_test", json!({"package": "greeter", "update": true})),
                call(10, "ext_check", json!({"package": "greeter/fixtures"})),
            ]
            .concat(),
            10,
        );
        replies.sort_by_key(|reply| reply["id"].as_u64());
        let names: Vec<&str> = replies[0]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(&names[5..], ["ext_check", "ext_run", "ext_test", "ext_snap"]);
        let result = |n: usize| &replies[n]["result"];
        assert_eq!(result(1)["structuredContent"]["keyword"], "greeter");
        assert_eq!(
            result(2)["structuredContent"]["items"][0]["view"]["title"],
            "Hello, Dupin!"
        );
        assert_eq!(result(3)["structuredContent"]["passed"], true);
        for n in [4, 5, 7] {
            assert_eq!(result(n)["isError"], true, "{}", replies[n]);
            assert!(
                result(n)["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("outside the workspace"),
                "{}",
                replies[n]
            );
        }
        assert_eq!(
            result(6)["content"][0]["text"],
            "package must be a path relative to the workspace"
        );
        assert_eq!(result(8)["content"][0]["text"], "unexpected argument: update");
        let missing: Value = serde_json::from_str(result(9)["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(missing["diagnostic"]["code"], "missing-manifest");
        assert_eq!(std::fs::read(&fixture).unwrap(), before);
    }

    #[test]
    fn closing_input_stops_an_extension_tool_whose_package_never_returns() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let package = root.join("spinner");
        dupin_ext_sdk::scaffold(&package, dupin_ext_sdk::Template::JavaScript).unwrap();
        std::fs::write(
            package.join("main.js"),
            "export function search() { return [{ title: 'Spin', actions: [{ title: 'Spin', run: 'spin' }] }]; }\n\
             export const actions = { spin() { for (;;) {} } };\n",
        )
        .unwrap();
        std::fs::write(
            package.join("fixtures/search.json"),
            json!({ "fixture": 1, "steps": [{ "query": "x" }, { "activate": "Spin" }] }).to_string(),
        )
        .unwrap();
        let host = Fake {
            workspace: Some(root),
            ..Fake::default()
        };
        let (mut client, server) = UnixStream::pair().unwrap();
        let started = Instant::now();
        std::thread::scope(|scope| {
            let serving = scope.spawn(|| serve(BufReader::new(server), &mut Vec::new(), &host).unwrap());
            client
                .write_all(call(1, "ext_test", json!({"package": "spinner"})).as_bytes())
                .unwrap();
            std::thread::sleep(Duration::from_secs(2));
            drop(client);
            serving.join().unwrap();
        });
        assert!(
            started.elapsed() < crate::extensions::ACTION_DEADLINE / 2,
            "serve waited {:?} for the action",
            started.elapsed()
        );
    }

    #[test]
    fn images_are_base64() {
        for (bytes, encoded) in [
            (&b""[..], ""),
            (b"a", "YQ=="),
            (b"ab", "YWI="),
            (b"abc", "YWJj"),
            (b"abcd", "YWJjZA=="),
            (&[0xff, 0xee, 0x80][..], "/+6A"),
        ] {
            assert_eq!(base64(bytes), encoded);
        }
    }

    #[test]
    fn a_cancelled_call_aborts_as_soon_as_it_registers() {
        let cancel = Cancel::default();
        cancel.abort(true);
        let (sender, receiver) = mpsc::channel();
        cancel.on_cancel(move || sender.send(()).unwrap());
        receiver.try_recv().unwrap();
    }
}
