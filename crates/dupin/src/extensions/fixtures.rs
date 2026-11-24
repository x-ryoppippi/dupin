//! `dupin ext test` and `dupin ext snap`: a package's fixtures replayed through the same
//! extension host, bytecode and checks the launcher uses, with the fixture's clock, seed and time
//! zone instead of the machine's. A query step answers as the launcher's keyword does: what
//! `search` computes, then the `items()` catalogue matched natively, held across steps and
//! updated as actions and events invalidate it. `snap` renders a step's panel with the launcher
//! itself, headless, in a scratch home that holds only the package, answering its host requests
//! from the fixture's `replies` as `test` does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use dupin_core::catalogue::Entry;
use dupin_ext_sdk::fixture::{self, Action, Difference, Fixture};
use dupin_extension_bridge::{Call, Trigger};
use dupin_extension_host::{ActionEffect, Host};
use dupin_js_runtime::row::{Row, RowEffect};
use dupin_js_runtime::{Diagnostic, RunError};
use serde_json::{Map, Value, json};

use super::catalogue::{Cached, Convert};
use super::provider::row_item;
use super::{ACTION_DEADLINE, Extension, QUERY_DEADLINE, helper_command, load_folder, manifest};

mod services;

pub(super) use services::headless;

/// Passes a fixture's clock and seed to the headless launcher `snap` starts.
pub(super) const HEADLESS_ENVIRONMENT: &str = "DUPIN_HEADLESS_EXTENSION_ENVIRONMENT";
const ZONEINFO: &str = "/usr/share/zoneinfo";
/// Each headless run's own limit; a realtime run of the longest fixture takes about a minute.
const RENDER_DEADLINE: Duration = Duration::from_secs(120);
/// Realtime seconds before the first key, for the launcher to load the package.
const FIRST_KEY: f64 = 1.0;
const STEP_SECONDS: f64 = 0.75;

/// What `dupin ext test` found: every fixture and step, as JSON and as text.
pub struct Report {
    pub passed: bool,
    pub json: Value,
    pub text: String,
}

/// Replays every fixture in `folder/fixtures`. With `update`, a fixture whose steps all ran is
/// rewritten to expect what they answered. Once `cancelled` answers true, the running call is
/// stopped and no further fixture starts.
pub fn test(folder: &Path, update: bool, cancelled: &dyn Fn() -> bool) -> Result<Report, Diagnostic> {
    let extension = load_folder(folder)?;
    let found = fixture::discover(folder)?;
    let mut fixtures = Vec::new();
    let mut text = String::new();
    let mut failed = 0;
    for (name, path) in &found {
        if cancelled() {
            return Err(Diagnostic::new("cancelled", "the run was cancelled"));
        }
        let result = fixture::read(name, path).and_then(|parsed| {
            let replayed = replay(&extension, name, &parsed, cancelled)?;
            Ok(replayed.finish(parsed, name, path, update))
        });
        let outcome = result.unwrap_or_else(|diagnostic| FixtureResult::invalid(name, diagnostic));
        if !outcome.passed {
            failed += 1;
        }
        outcome.describe(&mut text);
        fixtures.push(outcome.json);
    }
    let passed = failed == 0;
    text.push_str(&format!(
        "{} {}: {} passed, {failed} failed\n",
        found.len(),
        if found.len() == 1 { "fixture" } else { "fixtures" },
        found.len() - failed,
    ));
    Ok(Report {
        passed,
        json: json!({ "package": extension.id, "passed": passed, "fixtures": fixtures }),
        text,
    })
}

/// A host of the fixture's own, whose helper runs in its time zone, which must be one this Mac
/// has, and whose guests see its clock and seed.
fn fixture_host(name: &str, fixture: &Fixture) -> Result<Host, Diagnostic> {
    let zone = fixture.environment.time_zone.clone();
    if !Path::new(ZONEINFO).join(&zone).is_file() {
        return Err(
            Diagnostic::new("invalid-fixture", format!("this Mac has no time zone {zone:?}"))
                .file(name)
                .field("environment.timeZone")
                .hint("Use a name from the tz database, such as `UTC` or `Europe/London`."),
        );
    }
    let launch = Box::new(move || {
        let mut command = helper_command()?;
        command.env("TZ", &zone);
        Ok(command)
    });
    Ok(Host::fixed(launch, Some(guest_environment(fixture))))
}

fn guest_environment(fixture: &Fixture) -> dupin_js_runtime::Environment {
    dupin_js_runtime::Environment {
        now: fixture::parse_time(&fixture.environment.now).expect("validated by `fixture::parse`"),
        seed: fixture.environment.seed,
    }
}

struct Replay {
    steps: Vec<StepResult>,
}

struct StepResult {
    action: String,
    outcome: Map<String, Value>,
    differences: Vec<Difference>,
    /// Why the step couldn't run, such as selecting a row the answer doesn't have.
    problem: Option<String>,
}

/// The catalogue a fixture holds across its steps, as the launcher holds one per package, with
/// each row as the package sent it, so a step reports and activates catalogue rows as it does
/// `search`'s.
struct Held<'a> {
    extension: &'a Extension,
    cached: Cached,
    rows: RefCell<HashMap<String, Row>>,
}

impl Convert for Held<'_> {
    fn entry(&self, row: Row) -> Entry {
        self.rows
            .borrow_mut()
            .insert(self.extension.entry_id(&row.id), row.clone());
        self.extension.entry(row)
    }

    fn entry_id(&self, id: &str) -> String {
        self.extension.entry_id(id)
    }
}

impl<'a> Held<'a> {
    fn new(extension: &'a Extension) -> Self {
        Self {
            extension,
            cached: Cached::default(),
            rows: RefCell::default(),
        }
    }

    /// The catalogue's matches for `query`, as the launcher ranks and groups them, asking the
    /// package through `host` first when the catalogue is missing, invalidated or expired.
    fn search(
        &self,
        host: &Host,
        query: &str,
        limit: usize,
        call: &Call,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<Row>, RunError> {
        let deadline = Instant::now() + QUERY_DEADLINE;
        let mut fetch = |cursor: Option<&str>, reason| {
            host.items(
                &self.extension.compiled,
                cursor,
                reason,
                deadline,
                cancelled,
                &call.services,
            )
        };
        let batches = self
            .cached
            .search(query, limit, &mut fetch, self, call.generation)
            .map_err(|error| self.extension.located(error))?;
        let held = self.rows.borrow();
        Ok(batches
            .into_iter()
            .flat_map(|batch| {
                let section = batch.section;
                batch
                    .items
                    .into_iter()
                    .filter_map(|item| held.get(&item.id).cloned())
                    .map(move |row| Row {
                        // A ranked list drops the headings, as the launcher's does.
                        section: section.clone(),
                        ..row
                    })
                    .collect::<Vec<_>>()
            })
            .collect())
    }
}

/// What the launcher's keyword answers `query` with: `search`'s rows for a query that isn't
/// blank, then the catalogue's matches.
fn query_rows(
    host: &Host,
    extension: &Extension,
    held: &Held<'_>,
    query: &str,
    call: &Call,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Row>, RunError> {
    let exports = host.check(&extension.compiled, Instant::now() + QUERY_DEADLINE, cancelled)?;
    let limit = dupin_js_runtime::MAX_ROWS;
    let mut rows = Vec::new();
    if exports.search && !query.trim().is_empty() {
        rows = host.search(
            &extension.compiled,
            query,
            limit,
            Instant::now() + QUERY_DEADLINE,
            cancelled,
            &call.services,
        )?;
    }
    if exports.items {
        let left = limit.saturating_sub(rows.len());
        rows.extend(held.search(host, query, left, call, cancelled)?);
    }
    Ok(rows)
}

/// Runs the steps in order through the fixture's host and broker bridge, stopping at the first
/// that can't run.
fn replay(
    extension: &Extension,
    name: &str,
    fixture: &Fixture,
    cancelled: &dyn Fn() -> bool,
) -> Result<Replay, Diagnostic> {
    let host = fixture_host(name, fixture)?;
    let world = services::Fixture::open(extension, name, &fixture.replies, guest_environment(fixture).now)?;
    let call = |trigger| world.bridge.call(&extension.package, trigger);
    let held = Held::new(extension);
    let mut rows: Vec<Row> = Vec::new();
    let mut selected: Option<usize> = None;
    let mut steps = Vec::new();
    for step in &fixture.steps {
        let action = step.action();
        let mut outcome = Map::new();
        let mut problem = None;
        match action {
            Action::Query(query) => {
                let answer = query_rows(&host, extension, &held, query, &call(Trigger::Query), cancelled);
                rows = match answer {
                    Ok(items) => items,
                    Err(error) => {
                        problem = record(error, &mut outcome);
                        Vec::new()
                    }
                };
                selected = (!rows.is_empty()).then_some(0);
                outcome.insert("rows".into(), fixture::rows(&rows));
                if let Some(index) = selected {
                    outcome.insert("selected".into(), index.into());
                }
            }
            Action::Select(index) if index < rows.len() => {
                selected = Some(index);
                outcome.insert("selected".into(), index.into());
            }
            Action::Select(index) => {
                problem = Some(format!(
                    "there is no row {index} to select; the answer has {} {}",
                    rows.len(),
                    if rows.len() == 1 { "row" } else { "rows" }
                ));
            }
            Action::Activate(name) => {
                let row = selected.and_then(|index| rows.get(index));
                // The IDs the launcher gives the row's actions, which a step may name instead of
                // a title.
                let chosen = row.and_then(|row| {
                    let (item, ..) = row_item(extension, row.clone(), false);
                    row.actions
                        .iter()
                        .zip(&item.actions)
                        .find(|(action, shown)| action.title == name || shown.id == name)
                        .map(|(action, _)| action)
                });
                match chosen {
                    Some(action) => {
                        outcome.insert("effect".into(), Value::Object(fixture::effect(&action.effect)));
                        if let RowEffect::Run { handler, data } = &action.effect {
                            let acted = host.action(
                                &extension.compiled,
                                handler,
                                data.as_ref().unwrap_or(&Value::Null),
                                Instant::now() + ACTION_DEADLINE,
                                cancelled,
                                &call(Trigger::Activation).services,
                            );
                            if let Ok(acted) = &acted
                                && acted.invalidated
                            {
                                held.cached.invalidate();
                                outcome.insert("invalidated".into(), true.into());
                            }
                            match acted.map(|acted| acted.effect) {
                                Ok(Some(ActionEffect::Toast { text })) => {
                                    outcome.insert("toasts".into(), json!([text]));
                                }
                                Ok(None) => {}
                                Err(error) => problem = record(error, &mut outcome),
                            }
                        }
                    }
                    None => {
                        let offered: Vec<&str> = row
                            .map(|row| row.actions.iter().map(|action| action.title.as_str()).collect())
                            .unwrap_or_default();
                        problem = Some(if offered.is_empty() {
                            format!("the selected row has no action {}", Value::from(name))
                        } else {
                            format!(
                                "the selected row has no action {}; it offers {}",
                                Value::from(name),
                                offered.join(", ")
                            )
                        });
                    }
                }
            }
            Action::Event(event, data) => {
                let handled = host.event(
                    &extension.compiled,
                    event,
                    data,
                    Instant::now() + ACTION_DEADLINE,
                    cancelled,
                    &call(Trigger::Background).services,
                );
                match handled {
                    Ok(handled) if handled.invalidated => {
                        held.cached.invalidate();
                        outcome.insert("invalidated".into(), true.into());
                    }
                    Ok(_) => {}
                    Err(error) => problem = record(error, &mut outcome),
                }
            }
        }
        let effects = world.take_effects();
        if !effects.clipboard.is_empty() {
            outcome.insert("clipboard".into(), effects.clipboard.into());
        }
        if let Some(storage) = effects.storage {
            outcome.insert("storage".into(), storage);
        }
        let differences = match (&problem, &step.expect) {
            (None, Some(expected)) => fixture::compare(expected, &outcome),
            _ => Vec::new(),
        };
        let stop = problem.is_some();
        steps.push(StepResult {
            action: action.describe(),
            outcome,
            differences,
            problem,
        });
        if stop {
            break;
        }
    }
    Ok(Replay { steps })
}

/// A failed call is the step's outcome; a cancelled one means the step never ran.
fn record(error: RunError, outcome: &mut Map<String, Value>) -> Option<String> {
    match error {
        RunError::Failed(diagnostic) => {
            outcome.insert("error".into(), fixture::error(&diagnostic));
            None
        }
        RunError::Cancelled => Some("the call was cancelled".into()),
    }
}

struct FixtureResult {
    passed: bool,
    json: Value,
    name: String,
    lines: Vec<String>,
    updated: bool,
}

impl Replay {
    fn finish(self, mut parsed: Fixture, name: &str, path: &Path, update: bool) -> FixtureResult {
        let complete =
            self.steps.len() == parsed.steps.len() && self.steps.iter().all(|step| step.problem.is_none());
        let mut updated = false;
        if update && complete {
            for (step, result) in parsed.steps.iter_mut().zip(&self.steps) {
                step.expect = Some(result.outcome.clone());
            }
            let rewritten = fixture::to_json(&parsed);
            if std::fs::read_to_string(path).ok().as_deref() != Some(rewritten.as_str()) {
                if let Err(error) = std::fs::write(path, rewritten) {
                    return FixtureResult::invalid(
                        name,
                        Diagnostic::new("unwritable", format!("Couldn't write it: {error}")).file(name),
                    );
                }
                updated = true;
            }
        }
        let passed = complete && (update || self.steps.iter().all(|step| step.differences.is_empty()));
        let mut lines = Vec::new();
        let steps: Vec<Value> = self
            .steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let step_passed = step.problem.is_none() && (update || step.differences.is_empty());
                if !step_passed {
                    lines.push(format!("  step {}, {}:", index + 1, step.action));
                    lines.extend(step.problem.iter().map(|problem| format!("    {problem}")));
                    if !update {
                        lines.extend(
                            step.differences
                                .iter()
                                .map(|difference| format!("    {difference}")),
                        );
                    }
                }
                let mut entry = json!({
                    "step": index + 1,
                    "action": step.action,
                    "passed": step_passed,
                    "outcome": step.outcome,
                });
                if !update && !step.differences.is_empty() {
                    entry["differences"] =
                        serde_json::to_value(&step.differences).expect("differences serialize");
                }
                if let Some(problem) = &step.problem {
                    entry["problem"] = problem.clone().into();
                }
                entry
            })
            .collect();
        FixtureResult {
            passed,
            json: json!({ "file": name, "passed": passed, "updated": updated, "steps": steps }),
            name: name.to_owned(),
            lines,
            updated,
        }
    }
}

impl FixtureResult {
    fn invalid(name: &str, diagnostic: Diagnostic) -> Self {
        let mut lines = vec![format!("  {diagnostic}")];
        lines.extend(diagnostic.hint.as_ref().map(|hint| format!("  hint: {hint}")));
        Self {
            passed: false,
            json: json!({ "file": name, "passed": false, "error": diagnostic.to_string(), "diagnostic": diagnostic }),
            name: name.to_owned(),
            lines,
            updated: false,
        }
    }

    fn describe(&self, out: &mut String) {
        let status = match (self.passed, self.updated) {
            (false, _) => "FAIL",
            (true, true) => "updated",
            (true, false) => "ok",
        };
        out.push_str(&format!("{status} {}\n", self.name));
        for line in &self.lines {
            out.push_str(line);
            out.push('\n');
        }
    }
}

/// What `dupin ext snap` rendered.
pub struct Snapped {
    pub png: PathBuf,
    pub step: usize,
    pub width: u16,
    pub height: u32,
    pub scale: u8,
}

/// Renders the panel after step `step` (counting from 1; the last by default) of the fixture at
/// `fixture_path`, with the launcher's own view in a headless run of this executable, to
/// `output`. Activations aren't replayed: their effects never show in the panel, and the panel
/// stays as it was when the action was chosen. Once `cancelled` answers true, the headless run
/// is stopped.
pub fn snap(
    folder: &Path,
    fixture_path: &Path,
    step: Option<usize>,
    output: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Snapped, Diagnostic> {
    let extension = load_folder(folder)?;
    let name = fixture_path.display().to_string();
    let parsed = fixture::read(&name, fixture_path)?;
    let last = step.unwrap_or(parsed.steps.len());
    if !(1..=parsed.steps.len()).contains(&last) {
        return Err(Diagnostic::new(
            "invalid-step",
            format!("has {} steps, so there is no step {last}", parsed.steps.len()),
        )
        .file(name)
        .hint("Name a step from 1 to the fixture's number of steps."));
    }
    let environment = &parsed.environment;
    if !Path::new(ZONEINFO).join(&environment.time_zone).is_file() {
        return Err(Diagnostic::new(
            "invalid-fixture",
            format!("this Mac has no time zone {:?}", environment.time_zone),
        )
        .file(name)
        .field("environment.timeZone"));
    }
    // The headless launcher runs in the scratch home, so a relative path would land there.
    let output = &std::path::absolute(output).map_err(|error| {
        Diagnostic::new(
            "render-failed",
            format!("couldn't resolve the output path: {error}"),
        )
    })?;
    let render_failed = |message: String| {
        Diagnostic::new("render-failed", message)
            .hint("`snap` runs this Dupin headless; a build without Rune's `tooling` feature can't.")
    };
    let scratch = tempfile::Builder::new()
        .prefix("dupin-snap-")
        .tempdir()
        .map_err(|error| render_failed(format!("couldn't make a scratch folder: {error}")))?;
    let root = scratch.path();
    let config = root.join("config/dupin");
    let extensions = config.join("extensions");
    std::fs::create_dir_all(&extensions)
        .and_then(|()| {
            std::fs::write(
                config.join("config.toml"),
                format!(
                    "hotkey = \"\"\nauto_update = false\ninput_source = \"keep\"\nweb_searches = {{}}\nextensions = true\n\n[theme]\npanel_width = {}\n",
                    environment.window.width
                ),
            )
        })
        .and_then(|()| std::fs::create_dir_all(root.join("data")))
        .and_then(|()| std::fs::create_dir_all(root.join("tmp")))
        .map_err(|error| render_failed(format!("couldn't prepare the scratch home: {error}")))?;
    copy_package(folder, &extensions.join(&extension.id))?;
    let script = root.join("script.json");
    std::fs::write(&script, script_for(&extension.keyword, &parsed, last).to_string())
        .map_err(|error| render_failed(format!("couldn't write the input script: {error}")))?;
    let handed = root.join("fixture.json");
    std::fs::write(&handed, fixture::to_json(&parsed))
        .map_err(|error| render_failed(format!("couldn't write the fixture: {error}")))?;
    let handed = serde_json::to_string(&services::HeadlessFixture {
        fixture: handed,
        package: extension.package.to_string(),
    })
    .unwrap_or_default();

    let exe =
        std::env::current_exe().map_err(|error| render_failed(format!("couldn't find Dupin: {error}")))?;
    let launcher = |args: &[&OsStr]| {
        let mut command = Command::new(&exe);
        let lang = format!("{}.UTF-8", environment.locale.replacen('-', "_", 1));
        command
            .env_clear()
            .current_dir(root)
            .env("HOME", root)
            .env("TMPDIR", root.join("tmp"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("PATH", "/usr/bin:/bin")
            .env("TZ", &environment.time_zone)
            .env("LANG", &lang)
            .env("LC_ALL", &lang)
            .env(
                HEADLESS_ENVIRONMENT,
                serde_json::to_string(&guest_environment(&parsed)).unwrap_or_default(),
            )
            .env(services::HEADLESS_FIXTURE, &handed)
            .arg("--script")
            .arg(&script)
            .args([
                "--realtime",
                "--at",
                "mark:end",
                "--theme",
                environment.theme.as_str(),
            ])
            .args(args);
        run_bounded(command, cancelled).map_err(render_failed)
    };
    let height = match environment.window.height {
        Some(height) => u32::from(height),
        None => {
            let tree = root.join("tree.json");
            launcher(&[OsStr::new("--tree"), tree.as_os_str()])?;
            panel_height(&tree).map_err(render_failed)?
        }
    };
    let size = format!("{}x{height}", environment.window.width);
    let scale = environment.window.scale.to_string();
    launcher(&[
        OsStr::new("--snap"),
        output.as_os_str(),
        OsStr::new("--size"),
        OsStr::new(&size),
        OsStr::new("--scale"),
        OsStr::new(&scale),
    ])?;
    if !output.is_file() {
        return Err(render_failed("the headless run wrote no PNG".into()));
    }
    Ok(Snapped {
        png: output.to_owned(),
        step: last,
        width: environment.window.width,
        height,
        scale: environment.window.scale,
    })
}

/// The package's manifest and modules, which are all the launcher reads, copied so the render
/// never sees the author's other files.
fn copy_package(folder: &Path, into: &Path) -> Result<(), Diagnostic> {
    let entry = manifest::read(&manifest::Stamps::read(folder)?)?.entry;
    let unwritable = |error: std::io::Error| {
        Diagnostic::new("render-failed", format!("couldn't copy the package: {error}"))
    };
    std::fs::create_dir_all(into).map_err(unwritable)?;
    std::fs::copy(
        folder.join(manifest::PACKAGE_JSON),
        into.join(manifest::PACKAGE_JSON),
    )
    .map_err(unwritable)?;
    for (name, path) in dupin_js_runtime::source_files(folder, &entry)? {
        let target = into.join(&name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(unwritable)?;
        }
        std::fs::copy(path, target).map_err(unwritable)?;
    }
    Ok(())
}

/// The keys a person would press for steps `1..=last`: the keyword and the first query, then
/// each later query in place of the last, and arrow keys to each selected row. The keyword
/// becomes a chip once typed, so a later query replaces only the text after it; an empty field
/// is left alone, since ⌫ there would leave the keyword.
fn script_for(keyword: &str, fixture: &Fixture, last: usize) -> Value {
    let mut events = Vec::new();
    let mut at = FIRST_KEY;
    let mut selected = 0;
    let mut typed: Option<&str> = None;
    for step in &fixture.steps[..last] {
        match step.action() {
            Action::Query(query) => {
                match typed {
                    None => {
                        events.push(json!({ "t": at, "type": "type", "text": format!("{keyword} {query}") }));
                    }
                    Some(before) => {
                        if !before.is_empty() {
                            events.push(json!({ "t": at, "type": "key", "name": "a", "cmd": true }));
                            at += 0.05;
                            events.push(json!({ "t": at, "type": "key", "name": "Backspace" }));
                            at += 0.05;
                        }
                        if !query.is_empty() {
                            events.push(json!({ "t": at, "type": "type", "text": query }));
                        }
                    }
                }
                typed = Some(query);
                selected = 0;
            }
            Action::Select(index) => {
                let (key, presses) = if index >= selected {
                    ("Down", index - selected)
                } else {
                    ("Up", selected - index)
                };
                for _ in 0..presses {
                    events.push(json!({ "t": at, "type": "key", "name": key }));
                    at += 0.05;
                }
                selected = index;
            }
            Action::Activate(_) | Action::Event(..) => continue,
        }
        at += STEP_SECONDS;
    }
    events.push(json!({ "t": at + STEP_SECONDS, "type": "mark", "name": "end" }));
    json!({ "events": events })
}

/// Runs `command` to completion within [`RENDER_DEADLINE`] unless `cancelled`, with the last
/// line it wrote to stderr as the error when it fails.
fn run_bounded(mut command: Command, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("couldn't start Dupin headless: {error}"))?;
    let stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut stderr) = stderr {
            let _ =
                std::io::Read::read_to_string(&mut std::io::Read::take(&mut stderr, 64 * 1024), &mut text);
        }
        text
    });
    let deadline = Instant::now() + RENDER_DEADLINE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if cancelled() => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("the headless run was cancelled".to_owned());
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("the headless run passed its deadline".to_owned());
            }
            Err(error) => break Err(format!("couldn't wait for the headless run: {error}")),
        }
    }?;
    let stderr = reader.join().unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    let last = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("no output");
    Err(format!("the headless run failed ({status}): {last}"))
}

/// The panel's laid-out height in points, rounded up, from a `--tree` dump.
fn panel_height(tree: &Path) -> Result<u32, String> {
    let bytes = std::fs::read(tree).map_err(|error| format!("couldn't read the element tree: {error}"))?;
    let tree: Value = serde_json::from_slice(&bytes).map_err(|_| "the element tree isn't JSON".to_owned())?;
    tree["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|element| element["key"] == "panel" && element["rect"].is_array())
        .and_then(|panel| panel["rect"][3].as_f64())
        .map(|height| height.ceil() as u32)
        .ok_or_else(|| "the element tree has no laid-out panel".to_owned())
}

#[cfg(test)]
mod tests;
