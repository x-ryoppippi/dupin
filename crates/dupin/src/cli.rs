use crate::control::{self, Command, Request, exchange};
use serde_json::Value;
use std::io::{self, Read};
const HELP: &str = "dupin open [--query QUERY] [--json]\ndupin pick [--input-json] [--json]\ndupin snapshots list|close [--json]\ndupin snapshots open [--id ID] [--json]\ndupin config get [--json]\ndupin config set BASE.json DESIRED.json [--json]\ndupin snap VIEW.json OUTPUT.png [--json]\ndupin ext check DIR [--json]\ndupin ext run DIR --query QUERY [--json]\ndupin ext test DIR [--update] [--json]\ndupin ext snap DIR FIXTURE.json OUTPUT.png [--step N] [--json]\ndupin ext revision DIR [--json]\ndupin ext new DIR [--ts | --bundle] [--json]\ndupin ext build DIR [--sourcemap] [--json]\ndupin ext dev DIR [--sourcemap] [--json]\ndupin ext search [QUERY] [--json]\ndupin ext install github:OWNER/REPO[@REF]|URL[@REF]|DIR [--json]\ndupin ext list|sync [--json]\ndupin ext update [NAME] [--json]\ndupin ext remove NAME [--json]\ndupin ext eject NAME [--json]\ndupin mcp [--workspace DIR]\ndupin --licenses [--json]\n\nNo subcommand starts the resident launcher. Existing Rune tooling flags remain available.";
/// The usage lines of `dupin ext`, taken from [`HELP`] so the two can't disagree.
fn ext_help() -> String {
    HELP.lines()
        .filter(|line| line.starts_with("dupin ext "))
        .collect::<Vec<_>>()
        .join("\n")
}
#[derive(Debug, PartialEq)]
enum Action {
    Help,
    ExtHelp,
    Version,
    /// Print the third-party notices Dupin.app carries.
    Licenses,
    Open(Option<String>),
    Pick(bool),
    Get,
    SnapshotsList,
    SnapshotsOpen(Option<String>),
    SnapshotsClose,
    Set(String, String),
    Snap(String, String),
    /// Check an extension folder.
    ExtCheck(String),
    /// Ask an extension folder for a query.
    ExtRun(String, String),
    /// Replay an extension folder's fixtures, rewriting their expectations with `true`.
    ExtTest(String, bool),
    /// Render a fixture's step: the folder, the fixture, the PNG and the step.
    ExtSnap(String, String, String, Option<usize>),
    /// Print the revision declared grants name for an extension folder.
    ExtRevision(String),
    /// Install, list, update, remove or restore packages in the extensions folder.
    ExtPackages(crate::extension_install::Command),
    ExtNew(String, dupin_ext_sdk::Template),
    ExtBuild(String, dupin_ext_sdk::BuildOptions),
    ExtDev(String, dupin_ext_sdk::BuildOptions),
    /// Copy a bundled extension into the extensions folder.
    ExtEject(String),
    /// List the GitHub repositories tagged `dupin-extension` that match the words.
    ExtSearch(Option<String>),
    Mcp(Option<String>),
}

/// Why a command failed: a plain message, or a diagnostic about an extension package.
#[derive(Debug, PartialEq)]
enum Failure {
    Message(String),
    Diagnostic(dupin_js_runtime::Diagnostic),
    /// Fixtures that didn't pass: the report as JSON and as text.
    Report(Value, String),
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
    let bundled = parser.contains("--bundle");
    let options = dupin_ext_sdk::BuildOptions {
        sourcemap: parser.contains("--sourcemap"),
    };
    let update = parser.contains("--update");
    let step: Option<usize> = parser
        .opt_value_from_str("--step")
        .map_err(|_| "--step takes a step number, counting from 1".to_owned())?;
    let workspace: Option<String> = parser
        .opt_value_from_str("--workspace")
        .map_err(|_| "--workspace takes a folder".to_owned())?;
    let build_flags = typescript || bundled || options.sourcemap;
    let other_flags = update || step.is_some() || workspace.is_some();
    let rest = parser.finish();
    let args: Vec<_> = rest
        .iter()
        .map(|s| s.to_str().ok_or_else(|| "arguments must be UTF-8".to_owned()))
        .collect::<Result<_, _>>()?;
    match args.as_slice() {
        _ if build_flags && other_flags => Err("invalid arguments; use dupin --help".into()),
        ["ext", "new", folder] if !folder.starts_with('-') && !options.sourcemap && !other_flags => {
            let template = match (typescript, bundled) {
                (_, true) => dupin_ext_sdk::Template::Bundled,
                (true, false) => dupin_ext_sdk::Template::TypeScript,
                (false, false) => dupin_ext_sdk::Template::JavaScript,
            };
            Ok(Action::ExtNew(folder.to_string(), template))
        }
        ["ext", "build", folder] if !folder.starts_with('-') && !typescript && !bundled && !other_flags => {
            Ok(Action::ExtBuild(folder.to_string(), options))
        }
        ["ext", "dev", folder] if !folder.starts_with('-') && !typescript && !bundled && !other_flags => {
            Ok(Action::ExtDev(folder.to_string(), options))
        }
        ["ext", "test", folder]
            if !folder.starts_with('-') && !build_flags && step.is_none() && workspace.is_none() =>
        {
            Ok(Action::ExtTest(folder.to_string(), update))
        }
        ["ext", "snap", folder, fixture, output]
            if [folder, fixture, output].iter().all(|arg| !arg.starts_with('-'))
                && step.is_none_or(|step| step > 0)
                && !update
                && !build_flags
                && workspace.is_none() =>
        {
            Ok(Action::ExtSnap(
                folder.to_string(),
                fixture.to_string(),
                output.to_string(),
                step,
            ))
        }
        ["mcp"] if !json && !build_flags && !update && step.is_none() => Ok(Action::Mcp(workspace)),
        _ if build_flags || other_flags => Err("invalid arguments; use dupin --help".into()),
        ["--help" | "-h"] => Ok(Action::Help),
        ["ext", "--help" | "-h" | "help"] => Ok(Action::ExtHelp),
        ["--version" | "-V"] => Ok(Action::Version),
        ["--licenses"] => Ok(Action::Licenses),
        ["open"] => Ok(Action::Open(None)),
        ["open", "--query", q] => Ok(Action::Open(Some(q.to_string()))),
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
        ["ext", "search", words @ ..] if words.iter().all(|word| !word.starts_with('-')) => {
            Ok(Action::ExtSearch((!words.is_empty()).then(|| words.join(" "))))
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
        ["ext", "eject", name] if !name.starts_with('-') => Ok(Action::ExtEject(name.to_string())),
        ["ext", "check", folder] if !folder.starts_with('-') => Ok(Action::ExtCheck(folder.to_string())),
        ["ext", "revision", folder] if !folder.starts_with('-') => {
            Ok(Action::ExtRevision(folder.to_string()))
        }
        ["ext", "run", folder, "--query", query] if !folder.starts_with('-') => {
            Ok(Action::ExtRun(folder.to_string(), query.to_string()))
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
            Action::Snap(_, output) if json => Some(output.clone()),
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
        Err(Failure::Report(report, text)) => {
            if json {
                println!("{report}");
            } else {
                print!("{text}");
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
        !first.starts_with('-')
            || matches!(
                first.as_str(),
                "--help" | "-h" | "--version" | "-V" | "--licenses" | "--json"
            )
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
        Action::ExtHelp => {
            let help = ext_help();
            if json {
                return Ok(serde_json::json!({"help": help}));
            }
            println!("{help}");
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
        Action::Licenses => {
            let (path, notices) = crate::notices::read()?;
            if json {
                Ok(serde_json::json!({"path": path, "notices": notices}))
            } else {
                // A reader such as `head` that stops early isn't a failure.
                let _ = std::io::Write::write_all(&mut io::stdout().lock(), notices.as_bytes());
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
        Action::ExtCheck(folder) => {
            let extension = crate::extensions::check_folder(std::path::Path::new(&folder))?;
            // Kept from the check above, so this asks the package nothing.
            let exports = extension.check(std::time::Instant::now() + crate::extensions::QUERY_DEADLINE)?;
            let mut exported: Vec<&str> = Vec::new();
            if exports.search {
                exported.push("search");
            }
            if exports.items {
                exported.push("items");
            }
            if json {
                return Ok(serde_json::json!({
                    "id": extension.id,
                    "name": extension.name,
                    "version": extension.version,
                    "keyword": extension.keyword,
                    "exports": exported,
                    "defaultSearch": extension.default_search,
                }));
            }
            println!(
                "{} {} answers `{}` with {}{}",
                extension.name,
                extension.version,
                extension.keyword,
                exported.join(" and "),
                if extension.default_search {
                    ", and the main search with its items"
                } else {
                    ""
                }
            );
            Ok(Value::Null)
        }
        Action::ExtRevision(folder) => {
            let (package, revision, capabilities) =
                crate::extensions::declaration_of(std::path::Path::new(&folder))?;
            if json {
                let scopes: Vec<_> = capabilities
                    .scopes()
                    .map(|scope| scope.key().to_string())
                    .collect();
                return Ok(serde_json::json!({
                    "package": package.to_string(),
                    "revision": revision.to_string(),
                    "scopes": scopes,
                }));
            }
            println!("{revision}");
            Ok(Value::Null)
        }
        Action::ExtRun(folder, query) => {
            let extension = crate::extensions::load_folder(std::path::Path::new(&folder))?;
            let batches = crate::extensions::answer(
                &extension,
                &query,
                None,
                dupin_js_runtime::MAX_ROWS,
                std::time::Instant::now() + crate::extensions::QUERY_DEADLINE,
                || false,
                &dupin_extension_bridge::Call::none(),
            )?;
            if json {
                return Ok(answer_json(&batches));
            }
            print!("{}", describe_answer(&batches));
            Ok(Value::Null)
        }
        Action::ExtTest(folder, update) => {
            let report = crate::extensions::test(std::path::Path::new(&folder), update, &|| false)?;
            if !report.passed {
                return Err(Failure::Report(report.json, report.text));
            }
            if json {
                return Ok(report.json);
            }
            print!("{}", report.text);
            Ok(Value::Null)
        }
        Action::ExtSnap(folder, fixture, output, step) => {
            let snapped = crate::extensions::snap(
                std::path::Path::new(&folder),
                std::path::Path::new(&fixture),
                step,
                std::path::Path::new(&output),
                &|| false,
            )?;
            if json {
                return Ok(snapped_json(&snapped));
            }
            println!(
                "Rendered step {} of {fixture} to {output}: {}×{} points at {}×",
                snapped.step, snapped.width, snapped.height, snapped.scale
            );
            Ok(Value::Null)
        }
        Action::ExtPackages(command) => Ok(crate::extension_install::run(command, json)?),
        Action::ExtSearch(query) => Ok(crate::extension_install::search(query.as_deref(), json)?),
        Action::ExtEject(name) => {
            let copy = crate::extension_install::eject(&name)?;
            let (config, ..) = crate::settings::load(&crate::settings::Paths::discover());
            if json {
                return Ok(serde_json::json!({
                    "name": name,
                    "path": copy,
                    "extensions": config.extensions,
                }));
            }
            println!(
                "Copied the bundled {name} to {}. It replaces the bundled one while the extensions folder is on, without the bundled grants once you change it; delete it to go back.",
                copy.display()
            );
            if !config.extensions {
                println!(
                    "The extensions folder is off: turn it on in Settings → Extensions to use your copy."
                );
            }
            Ok(Value::Null)
        }
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
                    "{} runs as written, needs no build and passes the check",
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
        Action::Mcp(workspace) => crate::mcp::run(workspace.map(std::path::PathBuf::from))
            .map(|()| Value::Null)
            .map_err(|e| e.to_string().into()),
    }
}

pub(crate) fn snapped_json(snapped: &crate::extensions::Snapped) -> Value {
    serde_json::json!({
        "png": snapped.png,
        "step": snapped.step,
        "width": snapped.width,
        "height": snapped.height,
        "scale": snapped.scale,
    })
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

/// An action's effect as `dupin ext run --json` shows it: the protocol's effects, and `run` for
/// a package's own handler.
fn effect_json(action: &dupin_core::Action) -> Value {
    use dupin_core::Action;
    match action {
        Action::Copy(text) => serde_json::json!({"kind": "copy", "text": text}),
        Action::OpenUrl(url) => serde_json::json!({"kind": "open_url", "url": url}),
        Action::Push(page) => serde_json::json!({
            "kind": "push", "view": page.view, "title": page.title, "context": page.context,
        }),
        Action::Extension {
            request: dupin_core::ExtensionRequest::Run { action, data },
            ..
        } => serde_json::json!({"kind": "run", "handler": action, "data": data}),
        Action::Extension {
            request: dupin_core::ExtensionRequest::Paste { text, template },
            ..
        } => serde_json::json!({"kind": "paste", "text": text, "template": template}),
        other => serde_json::json!({"kind": format!("{other:?}")}),
    }
}

/// The rows an extension answered with, in the protocol's `results` shape, with each row's
/// section and detail beside it.
pub(crate) fn answer_json(batches: &[dupin_core::Batch]) -> Value {
    let items: Vec<Value> = batches
        .iter()
        .flat_map(|batch| batch.items.iter().map(move |item| (batch, item)))
        .map(|(batch, item)| {
            let mut row = serde_json::json!({
                "id": item.id,
                "view": item.view,
                "actions": item.actions.iter().map(|action| serde_json::json!({
                    "id": action.id,
                    "title": action.title,
                    "effect": effect_json(item.action(&action.id).expect("listed")),
                })).collect::<Vec<_>>(),
            });
            if let Some(section) = &batch.section {
                row["section"] = section.as_str().into();
            }
            match &item.icon {
                Some(dupin_core::Icon::Host(image)) => row["image"] = host_image_json(image),
                Some(dupin_core::Icon::Symbol(name)) => {
                    row["icon"] = format!("{}{name}", dupin_core::item::SYMBOL_PREFIX).into();
                }
                Some(icon) => {
                    if let Some(path) = icon.path() {
                        row["icon"] = path.to_string_lossy().into_owned().into();
                    }
                }
                None => {}
            }
            if let Some(detail) = batch.details.get(&item.id) {
                row["detail"] = serde_json::json!({
                    "title": detail.title,
                    "metadata": detail.metadata.iter().map(|entry| serde_json::json!({
                        "label": entry.label, "text": entry.value,
                    })).collect::<Vec<_>>(),
                    "blocks": detail.body.blocks.len(),
                });
                if let Some(image) = &detail.image {
                    row["detail"]["image"] = host_image_json(image);
                }
            }
            row
        })
        .collect();
    serde_json::json!({"kind": "results", "items": items})
}

/// An image a row names, as its package gave it.
fn host_image_json(image: &dupin_core::HostImage) -> Value {
    match &image.source {
        dupin_core::HostImageSource::Blob(handle) => handle.as_str().into(),
        dupin_core::HostImageSource::File { path, handle } => {
            serde_json::json!({ "path": path, "handle": handle })
        }
    }
}

/// An answer as plain text: each section's heading, each row's text, then its actions,
/// indented.
fn describe_answer(batches: &[dupin_core::Batch]) -> String {
    use dupin_core::View;
    let mut out = String::new();
    let mut section = None;
    for batch in batches {
        if batch.section.is_some() && batch.section != section {
            out.push_str(&format!("# {}\n", batch.section.as_deref().unwrap_or_default()));
        }
        section.clone_from(&batch.section);
        for item in &batch.items {
            match &item.view {
                View::Text { title, subtitle } if subtitle.is_empty() => out.push_str(&format!("{title}\n")),
                View::Text { title, subtitle } => out.push_str(&format!("{title}\n  {subtitle}\n")),
                View::Detail { title, body } => out.push_str(&format!("{title}\n  {body}\n")),
                View::Pair { source, result } => out.push_str(&format!("{source} → {result}\n")),
            }
            for action in &item.actions {
                let target = match item.action(&action.id).expect("listed") {
                    dupin_core::Action::Copy(text) => text.clone(),
                    dupin_core::Action::OpenUrl(url) => url.clone(),
                    dupin_core::Action::Push(page) => format!("view {}", page.view),
                    dupin_core::Action::Extension {
                        request: dupin_core::ExtensionRequest::Run { action, .. },
                        ..
                    } => format!("runs {action}"),
                    dupin_core::Action::Extension {
                        request: dupin_core::ExtensionRequest::Paste { text, .. },
                        ..
                    } => format!("paste {text}"),
                    other => format!("{other:?}"),
                };
                out.push_str(&format!("  {}: {target}\n", action.title));
            }
        }
    }
    if out.is_empty() {
        out.push_str("No results\n");
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
            execute(Action::ExtCheck(word_count.clone()), true).unwrap()["keyword"],
            "wc"
        );
        let answer = execute(Action::ExtRun(word_count, "two words".into()), true).unwrap();
        assert_eq!(answer["items"][0]["view"]["title"], "2 words");
        assert_eq!(
            answer["items"][0]["actions"][0]["effect"],
            serde_json::json!({"kind": "copy", "text": "2"})
        );

        let leftover = tempfile::tempdir().unwrap();
        std::fs::write(leftover.path().join("extension.toml"), "entry = \"guest.wasm\"\n").unwrap();
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
            (
                Action::ExtCheck(leftover.path().to_string_lossy().into_owned()),
                "wasm-removed",
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
        assert_eq!(loaded.id, "greeter");
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
                "invalid-type",
            ),
            (r#""keyword": "greeter", "colour": "x""#, "invalid-manifest"),
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
    fn typescript_packages_check_run_and_publish_without_a_build() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let root = tempfile::tempdir().unwrap();
        let extensions = tempfile::tempdir().unwrap();
        let package = root.path().join("greeter");
        dupin_ext_sdk::scaffold(&package, dupin_ext_sdk::Template::TypeScript).unwrap();
        let folder = package.to_string_lossy().into_owned();
        assert_eq!(
            execute(Action::ExtCheck(folder.clone()), true).unwrap()["id"],
            "greeter"
        );
        assert_eq!(
            execute(Action::ExtRun(folder.clone(), "Dupin".into()), true).unwrap()["items"][0]["view"]["title"],
            "Hello, Dupin!"
        );
        assert_eq!(
            execute(Action::ExtBuild(folder.clone(), Default::default()), true).unwrap()["rolldown"],
            Value::Null
        );

        let published = dupin_ext_sdk::dev::DevCopy {
            package_dir: &package,
            extensions: extensions.path(),
            rolldown: None,
            options: dupin_ext_sdk::BuildOptions::default(),
            validate: &check_package,
        };
        let mut outcome = None;
        published.watch(std::time::Duration::ZERO, &|| false, &mut |published| {
            outcome = Some(published)
        });
        assert_eq!(outcome.unwrap().unwrap().revision, 1);
        let link = extensions.path().join("greeter");
        assert!(link.join("src/main.ts").is_file());
        crate::extensions::check_folder(&link).unwrap();

        let main = package.join("src/main.ts");
        let source = std::fs::read_to_string(&main).unwrap();
        std::fs::write(&main, format!("{source}\nenum Mode {{\n\tOn,\n}}\n")).unwrap();
        let line = source.lines().count() as u32 + 2;
        match execute(Action::ExtCheck(folder.clone()), true) {
            Err(Failure::Diagnostic(diagnostic)) => assert_eq!(
                (
                    diagnostic.code.as_ref(),
                    diagnostic.file.as_deref(),
                    diagnostic.line
                ),
                ("typescript-needs-build", Some("src/main.ts"), Some(line)),
                "{diagnostic:?}"
            ),
            other => panic!("expected the enum to be rejected, got {other:?}"),
        }

        std::fs::write(
            &main,
            source.replace(
                "if (!name) return [];",
                "if (!name) return [];\n\tif (name === 'throw') throw new Error(name as string);",
            ),
        )
        .unwrap();
        let throw_line = source
            .lines()
            .position(|line| line.contains("if (!name)"))
            .unwrap() as u32
            + 2;
        match execute(Action::ExtRun(folder, "throw".into()), true) {
            Err(Failure::Diagnostic(diagnostic)) => assert_eq!(
                (
                    diagnostic.code.as_ref(),
                    diagnostic.file.as_deref(),
                    diagnostic.line
                ),
                ("search-threw", Some("src/main.ts"), Some(throw_line)),
                "{diagnostic:?}"
            ),
            other => panic!("expected search to throw, got {other:?}"),
        }
    }

    #[test]
    fn plain_text_answers_list_sections_rows_and_actions() {
        use dupin_core::{Action, Batch, ExtensionRequest, Item, ItemAction, Payload, View};
        let item = |title: &str, actions: Vec<ItemAction>| Item {
            id: title.into(),
            view: View::Text {
                title: title.into(),
                subtitle: "Read them".into(),
            },
            payload: Payload::default(),
            record_usage: false,
            icon: None,
            matched: Vec::new(),
            completion: None,
            score: None,
            primary: None,
            alternate: None,
            actions,
        };
        let batches = [
            Batch::from(vec![item(
                "Docs",
                vec![
                    ItemAction::new("open", "Open URL", Action::OpenUrl("https://example.com/".into())),
                    ItemAction::new("copy", "Copy", Action::Copy("docs".into())),
                ],
            )]),
            Batch::section(
                "Recent",
                vec![item(
                    "Notes",
                    vec![ItemAction::new(
                        "action-0",
                        "Forget",
                        Action::Extension {
                            extension: "notes".into(),
                            request: ExtensionRequest::Run {
                                action: "forget".into(),
                                data: None,
                            },
                        },
                    )],
                )],
            ),
        ];
        assert_eq!(
            describe_answer(&batches),
            "Docs\n  Read them\n  Open URL: https://example.com/\n  Copy: docs\n# Recent\nNotes\n  Read them\n  Forget: runs forget\n"
        );
        let json = answer_json(&batches);
        assert_eq!(json["items"][1]["section"], "Recent");
        assert_eq!(
            json["items"][1]["actions"][0]["effect"],
            serde_json::json!({"kind": "run", "handler": "forget", "data": null})
        );
        assert_eq!(describe_answer(&[]), "No results\n");
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
    fn ext_help_prints_the_ext_usage() {
        for help in ["--help", "-h", "help"] {
            assert_eq!(
                parse(&["ext".into(), help.into()]).unwrap(),
                Action::ExtHelp,
                "dupin ext {help}"
            );
        }
        let usage = ext_help();
        assert!(usage.starts_with("dupin ext check DIR"), "{usage}");
        assert!(usage.ends_with("dupin ext eject NAME [--json]"), "{usage}");
        assert!(
            usage.lines().all(|line| line.starts_with("dupin ext ")),
            "{usage}"
        );
        assert_eq!(
            execute(Action::ExtHelp, true).unwrap(),
            serde_json::json!({ "help": usage })
        );
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
        assert!(owns_arguments(&["--licenses".into()]));
        assert_eq!(parse(&["--licenses".into()]).unwrap(), Action::Licenses);
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
        assert!(parse(&args(&["ext", "run", "word-count"])).is_err());
        assert_eq!(
            parse(&args(&["ext", "revision", "word-count"])).unwrap(),
            Action::ExtRevision("word-count".into())
        );
        assert_eq!(
            parse(&args(&["ext", "new", "--ts", "greeter"])).unwrap(),
            Action::ExtNew("greeter".into(), dupin_ext_sdk::Template::TypeScript)
        );
        assert_eq!(
            parse(&args(&["ext", "new", "greeter"])).unwrap(),
            Action::ExtNew("greeter".into(), dupin_ext_sdk::Template::JavaScript)
        );
        for bundled in [
            &["ext", "new", "--bundle", "greeter"][..],
            &["ext", "new", "--ts", "--bundle", "greeter"],
        ] {
            assert_eq!(
                parse(&args(bundled)).unwrap(),
                Action::ExtNew("greeter".into(), dupin_ext_sdk::Template::Bundled)
            );
        }
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
            &["ext", "dev", "greeter", "--bundle"],
            &["ext", "dev"],
            &["ext", "check", "guest.wasm", "request.json"],
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
        assert_eq!(
            parse(&args(&["ext", "eject", "dupin", "--json"])).unwrap(),
            Action::ExtEject("dupin".into())
        );
        assert!(parse(&args(&["ext", "eject"])).is_err());
        assert_eq!(parse(&args(&["ext", "search"])).unwrap(), Action::ExtSearch(None));
        assert_eq!(
            parse(&args(&["ext", "search", "clip", "board", "--json"])).unwrap(),
            Action::ExtSearch(Some("clip board".into()))
        );
        assert!(parse(&args(&["ext", "search", "--limit"])).is_err());
        assert_eq!(parse(&["mcp".into()]).unwrap(), Action::Mcp(None));
        assert_eq!(
            parse(&args(&["mcp", "--workspace", "packages"])).unwrap(),
            Action::Mcp(Some("packages".into()))
        );
        assert_eq!(
            parse(&args(&["ext", "test", "greeter", "--update", "--json"])).unwrap(),
            Action::ExtTest("greeter".into(), true)
        );
        assert_eq!(
            parse(&args(&["ext", "test", "greeter"])).unwrap(),
            Action::ExtTest("greeter".into(), false)
        );
        assert_eq!(
            parse(&args(&[
                "ext",
                "snap",
                "greeter",
                "fixtures/search.json",
                "out.png",
                "--step",
                "2"
            ]))
            .unwrap(),
            Action::ExtSnap(
                "greeter".into(),
                "fixtures/search.json".into(),
                "out.png".into(),
                Some(2)
            )
        );
        for invalid in [
            &[
                "ext",
                "snap",
                "greeter",
                "fixtures/search.json",
                "out.png",
                "--step",
                "0",
            ][..],
            &[
                "ext",
                "snap",
                "greeter",
                "fixtures/search.json",
                "out.png",
                "--step",
                "x",
            ],
            &[
                "ext",
                "snap",
                "greeter",
                "fixtures/search.json",
                "out.png",
                "--update",
            ],
            &["ext", "snap", "greeter", "fixtures/search.json"],
            &["ext", "test", "greeter", "--step", "1"],
            &["ext", "run", "greeter", "--query", "x", "--update"],
            &["ext", "check", "greeter", "--workspace", "x"],
            &["mcp", "--update"],
            &["mcp", "--ts"],
            &["ext", "test", "greeter", "--ts"],
            &["ext", "new", "greeter", "--update"],
        ] {
            assert!(parse(&args(invalid)).is_err(), "{invalid:?}");
        }
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
