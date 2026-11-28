//! `dupin ext test` and `dupin ext render`: a package's fixtures replayed through the same
//! extension host, bytecode and checks the launcher uses, with the fixture's clock, seed and time
//! zone instead of the machine's. A query step answers as the launcher's keyword does: what
//! `search` computes, then the `items()` catalogue matched natively, held across steps and
//! updated as actions and events invalidate it. `render` draws a step's panel with the launcher
//! itself, headless, in a scratch home that holds only the package, answering its host requests
//! from the fixture's `replies` as `test` does.

use dupin_spawn::{Command, Stdio};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dupin_core::catalogue::Entry;
use dupin_ext_sdk::fixture::{self, Difference, Fixture};
use dupin_extension_bridge::Call;
use dupin_extension_host::Host;
use dupin_js_runtime::row::{Row, RowEffect};
use dupin_js_runtime::{Diagnostic, Preferences, RunError};
use serde_json::{Map, Value, json};

use super::catalogue::{Asking, Cached, Convert};
use super::{Extension, QUERY_DEADLINE, bundled, helper_command, load_folder, manifest};

mod replay;
mod services;

use replay::replay;

pub(super) use services::headless;

/// Passes a fixture's clock and seed to the headless launcher `render` starts.
pub(super) const HEADLESS_ENVIRONMENT: &str = "DUPIN_HEADLESS_EXTENSION_ENVIRONMENT";
const ZONEINFO: &str = "/usr/share/zoneinfo";
/// Each headless run's own limit; a realtime run of the longest fixture takes about a minute.
const RENDER_DEADLINE: Duration = Duration::from_secs(120);
/// Realtime seconds before the first key, for the launcher to load the package.
const FIRST_KEY: f64 = 1.0;

/// How quickly a replay presses keys, in seconds.
#[derive(Clone, Copy)]
struct Pace {
    /// Between the characters of typed text.
    typing: f64,
    /// Between the keys of one step.
    press: f64,
    /// After each step, for it to answer.
    step: f64,
}

/// A still only needs each step to have answered.
const QUICK: Pace = Pace {
    typing: 0.035,
    press: 0.05,
    step: 0.75,
};

/// A recording is watched, so it presses keys at a pace a reader can follow.
const READABLE: Pace = Pace {
    typing: 0.1,
    press: 0.45,
    step: 1.2,
};

/// What `dupin ext test` found: every fixture and step, as JSON and as text.
pub struct Report {
    pub passed: bool,
    pub json: Value,
    pub text: String,
}

/// The package in `folder` as the launcher runs it: one in the folder this Dupin's bundled
/// extensions are in as that bundled package, so its fixtures replay what it may do that a
/// third-party package may not, and any other as itself. It goes by where the folder is
/// rather than by the build's seal, which every edit to a fixture would break until the next
/// build.
fn load_as_launched(folder: &Path) -> Result<Arc<Extension>, Diagnostic> {
    let canonical = |path: &Path| path.canonicalize().ok();
    let shipped = bundled::directory().as_deref().and_then(canonical);
    if shipped.is_some() && canonical(folder).as_deref().and_then(Path::parent) == shipped.as_deref() {
        return super::load(folder, None, true).map(|loaded| loaded.extension);
    }
    load_folder(folder)
}

/// Replays every fixture in `folder/fixtures`. With `update`, a fixture whose steps all ran is
/// rewritten to expect what they answered. Once `cancelled` answers true, the running call is
/// stopped and no further fixture starts.
pub fn test(folder: &Path, update: bool, cancelled: &dyn Fn() -> bool) -> Result<Report, Diagnostic> {
    let extension = load_as_launched(folder)?;
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
    Ok(Host::replaying(launch, Some(guest_environment(fixture))))
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
    /// What a person presses for the step, which `render` replays in the headless launcher.
    presses: Vec<Press>,
}

/// A key or text a person gives the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Press {
    Type(String),
    Key { name: &'static str, cmd: bool },
}

impl Press {
    fn key(name: &'static str) -> Self {
        Self::Key { name, cmd: false }
    }

    fn command(name: &'static str) -> Self {
        Self::command_key(name)
    }

    fn command_key(name: &'static str) -> Self {
        Self::Key { name, cmd: true }
    }
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
    /// Rows' times are measured against `now`, the fixture's clock in Unix milliseconds.
    fn new(extension: &'a Extension, now: i64) -> Self {
        Self {
            extension,
            cached: Cached::with_clock(extension.matching, super::Clock::fixed(now)),
            rows: RefCell::default(),
        }
    }

    /// The catalogue's matches for `query`, as the launcher ranks and groups them, then its web
    /// searches filled with the query, asking the package through `host` with `preferences`
    /// first when the catalogue is missing, invalidated, expired or asked for with others.
    fn search(
        &self,
        host: &Host,
        query: &str,
        limit: usize,
        preferences: &Arc<Preferences>,
        call: &Call,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<Row>, RunError> {
        let deadline = host.now() + QUERY_DEADLINE;
        let mut fetch = self.extension.fetch(
            host,
            super::Fetching::Resident,
            deadline,
            cancelled,
            &call.services,
        );
        let asking = Asking {
            preferences,
            generation: call.generation,
            role: super::Role::Keyword,
        };
        let batches = self
            .cached
            .search(query, limit, asking, &mut fetch, self)
            .map_err(|error| self.extension.located(error))?;
        let searches = self
            .cached
            .web_searches(asking, &mut fetch, self)
            .map_err(|error| self.extension.located(error))?;
        let held = self.rows.borrow();
        let mut rows: Vec<Row> = batches
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
            .collect();
        rows.extend(
            searches
                .iter()
                .filter_map(|(shown, search)| crate::web::item(shown, search, query.trim(), false))
                .map(|item| item_row(self.extension, &item)),
        );
        rows.truncate(limit);
        Ok(rows)
    }
}

/// What the launcher's keyword answers `query` with: `search`'s rows for a query that isn't
/// blank, or for any query when the manifest sets `emptyQuery`, then the catalogue's matches and
/// web searches.
fn query_rows(
    host: &Host,
    extension: &Extension,
    held: &Held<'_>,
    query: &str,
    preferences: &Arc<Preferences>,
    call: &Call,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Row>, RunError> {
    let exports = host.check(&extension.compiled, host.now() + QUERY_DEADLINE, cancelled)?;
    let limit = dupin_js_runtime::MAX_ROWS;
    let mut rows = Vec::new();
    if exports.search && (extension.empty_query || !query.trim().is_empty()) {
        rows = host.search(
            &extension.compiled,
            query,
            limit,
            host.now() + QUERY_DEADLINE,
            cancelled,
            &call.services,
        )?;
    }
    if exports.items {
        let left = limit.saturating_sub(rows.len());
        rows.extend(held.search(host, query, left, preferences, call, cancelled)?);
    }
    Ok(rows)
}

/// A web search's row as the launcher fills it, as fixtures state rows: the package's own ID
/// and what each action does.
fn item_row(extension: &Extension, item: &dupin_core::Item) -> Row {
    let prefix = format!("extension:{}/", extension.id);
    let subtitle = match &item.view {
        dupin_core::View::Text { subtitle, .. } => subtitle.clone(),
        _ => String::new(),
    };
    let actions = item
        .actions
        .iter()
        .filter_map(|action| {
            let effect = match item.action(&action.id)? {
                dupin_core::Action::Copy(text) => RowEffect::Copy { text: text.clone() },
                dupin_core::Action::OpenUrl(url) => RowEffect::OpenUrl { url: url.clone() },
                dupin_core::Action::Extension {
                    request: dupin_core::ExtensionRequest::Run { action, data },
                    ..
                } => RowEffect::Run {
                    handler: action.clone(),
                    data: data.clone(),
                },
                _ => return None,
            };
            Some(dupin_js_runtime::row::RowAction::new(
                action.title.clone(),
                effect,
            ))
        })
        .collect();
    Row {
        subtitle,
        actions,
        ..Row::new(
            item.id.strip_prefix(&prefix).unwrap_or(&item.id),
            item.display_title(),
        )
    }
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

/// What `dupin ext render` rendered.
pub struct Rendered {
    pub png: PathBuf,
    pub step: usize,
    pub width: u16,
    pub height: u32,
    pub scale: u8,
    /// Every text the panel shows, in the order it lays them out.
    pub shown: Vec<String>,
}

/// Renders the panel after step `step` (counting from 1; the last by default) of the fixture at
/// `fixture_path`, with the launcher's own view in a headless run of this executable, to
/// `output`. Activations aren't replayed: their effects never show in the panel, and the panel
/// stays as it was when the action was chosen. Once `cancelled` answers true, the headless run
/// is stopped.
pub fn render(
    folder: &Path,
    fixture_path: &Path,
    step: Option<usize>,
    output: &Path,
    frames: Option<&Path>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Rendered, Diagnostic> {
    let extension = load_as_launched(folder)?;
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
            .hint("`render` runs this Dupin headless; a build without Rune's `tooling` feature can't.")
    };
    let scratch = tempfile::Builder::new()
        .prefix("dupin-render-")
        .tempdir()
        .map_err(|error| render_failed(format!("couldn't make a scratch folder: {error}")))?;
    let root = scratch.path();
    let config = root.join("config/dupin");
    let extensions = config.join("extensions");
    std::fs::create_dir_all(&extensions)
        .and_then(|()| {
            std::fs::write(
                config.join("config.toml"),
                scratch_config(&extension, environment.window.width),
            )
        })
        .and_then(|()| std::fs::create_dir_all(root.join("data")))
        .and_then(|()| std::fs::create_dir_all(root.join("tmp")))
        .map_err(|error| render_failed(format!("couldn't prepare the scratch home: {error}")))?;
    copy_package(folder, &extensions.join(&extension.id))?;
    let script = root.join("script.json");
    // The replay works out what to press for each step: which action sits where in its row's
    // panel, and where each field of a form is.
    let replayed = replay(&extension, &name, &parsed, cancelled)?;
    let steps = &replayed.steps[..last.min(replayed.steps.len())];
    if let Some((index, problem)) = steps
        .iter()
        .enumerate()
        .find_map(|(index, step)| Some((index, step.problem.as_ref()?)))
    {
        return Err(
            Diagnostic::new("invalid-step", format!("step {} can't run: {problem}", index + 1))
                .file(name)
                .hint("Run `dupin ext test` on the package to see each step's outcome."),
        );
    }
    let clipboard_events: Vec<bool> = parsed
        .steps
        .iter()
        .map(|step| {
            matches!(
                step.action(),
                fixture::Action::Event(services::CLIPBOARD_EVENT, _)
            )
        })
        .collect();
    let pace = if frames.is_some() { READABLE } else { QUICK };
    let (events, played) = script_for(steps, &clipboard_events, pace);
    std::fs::write(&script, events.to_string())
        .map_err(|error| render_failed(format!("couldn't write the input script: {error}")))?;
    let handed = root.join("fixture.json");
    std::fs::write(&handed, fixture::to_json(&parsed))
        .map_err(|error| render_failed(format!("couldn't write the fixture: {error}")))?;
    let handed = serde_json::to_string(&services::HeadlessFixture {
        fixture: handed,
        name: extension.id.clone(),
        revision: extension.revision.as_str().to_owned(),
        events: played,
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
            .env(crate::features::apps::HEADLESS_APPS, "")
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
    let drawn = root.join("drawn.json");
    launcher(&[
        OsStr::new("--snap"),
        output.as_os_str(),
        OsStr::new("--size"),
        OsStr::new(&size),
        OsStr::new("--scale"),
        OsStr::new(&scale),
        OsStr::new("--tree"),
        drawn.as_os_str(),
    ])?;
    if !output.is_file() {
        return Err(render_failed("the headless run wrote no PNG".into()));
    }
    let shown = shown_text(&drawn).map_err(render_failed)?;
    if let Some(frames) = frames {
        record_frames(
            frames,
            &script,
            &launcher,
            environment.window.width,
            height,
            &scale,
        )?;
    }
    Ok(Rendered {
        png: output.to_owned(),
        step: last,
        width: environment.window.width,
        height,
        scale: environment.window.scale,
        shown,
    })
}

/// How many frames a second `--record` records.
const FRAMES_PER_SECOND: u8 = 10;
/// How tall `--record` draws the panel at least, in points, so it has room to grow at a step
/// before the last.
const FRAMES_HEIGHT: u32 = 720;

/// Records the whole replay into `frames` as `frame_0000.png` and on, with the input script
/// beside them as `script.json`, so a recording can caption the keys it shows.
fn record_frames(
    frames: &Path,
    script: &Path,
    launcher: &dyn Fn(&[&OsStr]) -> Result<(), Diagnostic>,
    width: u16,
    height: u32,
    scale: &str,
) -> Result<(), Diagnostic> {
    std::fs::create_dir_all(frames)
        .and_then(|()| std::fs::copy(script, frames.join("script.json")).map(|_| ()))
        .map_err(|error| {
            Diagnostic::new(
                "render-failed",
                format!("couldn't prepare {}: {error}", frames.display()),
            )
        })?;
    let size = format!("{width}x{}", height.max(FRAMES_HEIGHT));
    let fps = FRAMES_PER_SECOND.to_string();
    launcher(&[
        OsStr::new("--out"),
        frames.as_os_str(),
        OsStr::new("--size"),
        OsStr::new(&size),
        OsStr::new("--scale"),
        OsStr::new(scale),
        OsStr::new("--fps"),
        OsStr::new(&fps),
    ])
}

/// The scratch home's config.toml: Extensions on, the panel `width` points wide, and, for a
/// package a built-in ships as, that built-in on, since only then does the package answer.
fn scratch_config(extension: &Extension, width: u16) -> String {
    let mut config = dupin_core::Config::parse(
        "hotkey = \"\"\nauto_update = false\ninput_source = \"keep\"\nweb_searches = {}\nextensions = true\n",
    )
    .expect("the scratch config parses");
    config.theme.panel_width = width;
    crate::features::turn_on_ported(&extension.id, &mut config);
    toml::to_string(&config).expect("a config serializes")
}

/// The package's manifest, modules and own icon, which are all the launcher reads, copied so
/// the render never sees the author's other files.
fn copy_package(folder: &Path, into: &Path) -> Result<(), Diagnostic> {
    let manifest = manifest::read(&manifest::Stamps::read(folder)?)?;
    let entry = manifest.entry;
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
    if let Some(manifest::PackageIcon::File(name)) = &manifest.icon {
        let image = dupin_js_runtime::package_file(folder, name)
            .map_err(|diagnostic| diagnostic.file(manifest::PACKAGE_JSON).field("dupin.icon"))?;
        let target = into.join(name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(unwritable)?;
        }
        std::fs::copy(image, target).map_err(unwritable)?;
    }
    Ok(())
}

/// The keys a person would press for `steps`, as the replay worked them out, a step's keys
/// close together and each step given time to answer, and when each step that
/// `clipboard_events` marks, by index, copies, which the launcher is handed at that time.
fn script_for(steps: &[StepResult], clipboard_events: &[bool], pace: Pace) -> (Value, Vec<(usize, f64)>) {
    let mut events = Vec::new();
    let mut copies = Vec::new();
    let mut at = FIRST_KEY;
    for (index, step) in steps.iter().enumerate() {
        if clipboard_events.get(index).copied().unwrap_or_default() {
            copies.push((index, at));
            at += pace.step;
            continue;
        }
        if step.presses.is_empty() {
            continue;
        }
        for press in &step.presses {
            events.push(match press {
                Press::Type(text) => {
                    let typed = json!({ "t": at, "type": "type", "text": text, "every": pace.typing });
                    // The headless run types one character at a time.
                    at += pace.typing * text.chars().count() as f64;
                    typed
                }
                Press::Key { name, cmd: true } => {
                    json!({ "t": at, "type": "key", "name": name, "cmd": true })
                }
                Press::Key { name, cmd: false } => json!({ "t": at, "type": "key", "name": name }),
            });
            at += pace.press;
        }
        at += pace.step;
    }
    events.push(json!({ "t": at + pace.step, "type": "mark", "name": "end" }));
    (json!({ "events": events }), copies)
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

/// Every text the panel shows, in the order it lays them out, from a `--tree` dump.
fn shown_text(tree: &Path) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(tree).map_err(|error| format!("couldn't read the element tree: {error}"))?;
    let tree: Value = serde_json::from_slice(&bytes).map_err(|_| "the element tree isn't JSON".to_owned())?;
    Ok(tree["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|element| element["text"].as_str())
        .map(str::to_owned)
        .collect())
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
