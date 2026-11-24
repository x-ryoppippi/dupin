//! Fixtures: JSON files in a package's `fixtures/` folder that replay input against the package
//! and state what it should answer. `dupin ext test` runs every one through the extension host;
//! `dupin ext snap` renders one step's panel. A fixture fixes everything a run could otherwise
//! take from the machine: the clock, `Math.random`'s seed, the locale, the time zone, the theme
//! and the window. Each host service an extension asks for is answered from the fixture's
//! `replies`, never from the machine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dupin_js_runtime::Diagnostic;
use dupin_js_runtime::row::{Row, RowEffect};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The format version a fixture states in `"fixture"`.
const VERSION: u32 = 1;
const FOLDER: &str = "fixtures";
const MAX_FIXTURE_BYTES: usize = 1024 * 1024;
const MAX_FIXTURES: usize = 64;
pub const MAX_STEPS: usize = 64;
pub const MAX_QUERY_BYTES: usize = 4096;
/// What a step may expect, as keys of its outcome.
const EXPECTATIONS: [&str; 10] = [
    "rows",
    "selected",
    "effect",
    "toasts",
    "clipboard",
    "error",
    "sections",
    "detail",
    "storage",
    "invalidated",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    fixture: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default, skip_serializing_if = "Replies::is_empty")]
    pub replies: Replies,
    /// What `items` sees as `ctx.preferences`.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub preferences: Map<String, Value>,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Environment {
    /// A UTC time, `YYYY-MM-DDTHH:MM:SSZ` with optional milliseconds, at which the clock stands.
    pub now: String,
    pub seed: u32,
    pub locale: String,
    pub time_zone: String,
    pub theme: Theme,
    pub window: Window,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            now: "2026-01-01T09:00:00Z".into(),
            seed: 1,
            locale: "en-GB".into(),
            time_zone: "UTC".into(),
            theme: Theme::Dark,
            window: Window::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Window {
    /// The panel's width in points, as `[theme] panel_width` sets it.
    pub width: u16,
    /// Points; the panel's own height, fitted to its content, when left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u16>,
    pub scale: u8,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            width: 680,
            height: None,
            scale: 2,
        }
    }
}

/// One thing the user does. Exactly one of `query`, `select` and `activate` is given.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Typed after the keyword, replacing the query; the step waits for the answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    query: Option<String>,
    /// Moves the selection to this row, counting from 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    select: Option<usize>,
    /// Chooses the selected row's action with this title or id. A `run` action calls the
    /// package's handler as Enter would; any other effect is recorded, never carried out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activate: Option<String>,
    /// Calls the package's `events[event]` with `data`, as a watch Dupin keeps for it would.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action<'a> {
    Query(&'a str),
    Select(usize),
    Activate(&'a str),
    Event(&'a str, &'a Value),
}

impl Step {
    pub fn action(&self) -> Action<'_> {
        match (&self.query, self.select, &self.activate, &self.event) {
            (Some(query), None, None, None) => Action::Query(query),
            (None, Some(row), None, None) => Action::Select(row),
            (None, None, Some(action), None) => Action::Activate(action),
            (None, None, None, Some(event)) => {
                Action::Event(event, self.data.as_ref().unwrap_or(&Value::Null))
            }
            _ => unreachable!("validated by `parse`"),
        }
    }
}

impl Action<'_> {
    /// The step as the report names it, such as `query "Dupin"`.
    pub fn describe(&self) -> String {
        match self {
            Action::Query(query) => format!("query {}", Value::from(*query)),
            Action::Select(row) => format!("select {row}"),
            Action::Activate(action) => format!("activate {}", Value::from(*action)),
            Action::Event(event, _) => format!("event {}", Value::from(*event)),
        }
    }
}

/// What the host services answer, by capability, in place of the network, files, programs,
/// storage, the clipboard, dictionaries and calendars. A request the fixture has no reply for
/// fails as `unavailable`; nothing falls through to this Mac.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Replies {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub http: Vec<HttpReply>,
    /// Each `files` scope's contents by name: the text of a `file` scope, or a `directory`
    /// scope's files by relative path.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub files: BTreeMap<String, FilesReply>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exec: Vec<ExecReply>,
    /// The data of each record the package's storage holds when the fixture starts, oldest
    /// first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub storage: Vec<Value>,
    /// What a clipboard read returns: content such as `{ "type": "text", "text": … }`, or
    /// `null` for an empty clipboard. Left out, reading fails as `unavailable`.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub clipboard: Option<Value>,
    /// Each term's entries, in the dictionary service's shape.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub dictionary: BTreeMap<String, Vec<Value>>,
    /// The events a calendar read sees, in the calendar service's shape.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub calendar: Vec<Value>,
    /// Your decision on each declared scope, named as grant files name it, such as
    /// `http:https://api.github.com` or `clipboard:read`. A scope left out is allowed.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub consent: BTreeMap<String, Consent>,
}

/// Keeps a `null` that is written apart from a key that is left out, which serde would
/// otherwise read alike.
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

impl Replies {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpReply {
    pub request: HttpRequest,
    pub response: HttpResponse,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpRequest {
    #[serde(default = "get")]
    pub method: String,
    pub url: String,
}

fn get() -> String {
    "GET".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpResponse {
    #[serde(default = "ok")]
    pub status: u16,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// The body as text; `json` gives it as JSON instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    json: Option<Value>,
}

fn ok() -> u16 {
    200
}

impl HttpResponse {
    pub fn body(&self) -> Vec<u8> {
        match (&self.json, &self.body) {
            (Some(json), _) => json.to_string().into_bytes(),
            (None, Some(body)) => body.clone().into_bytes(),
            (None, None) => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FilesReply {
    File(String),
    Directory(BTreeMap<String, String>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecReply {
    /// The profile's name as `dupin.capabilities.exec` declares it, `@script:` included.
    pub profile: String,
    /// The arguments the run must have, after its inputs fill the template. Matches any when
    /// left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub status: i32,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Consent {
    Allow,
    /// Granted, then revoked.
    Deny,
    /// Not answered yet, so a request needs consent.
    Pending,
}

/// A relative path inside a files scope: plain names joined by `/`.
fn is_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains('\0'))
}

/// Reads and checks the fixture in `bytes`. `name` is the file's path inside the package.
pub fn parse(name: &str, bytes: &[u8]) -> Result<Fixture, Diagnostic> {
    let invalid = |message: String| Diagnostic::new("invalid-fixture", message).file(name);
    if bytes.len() > MAX_FIXTURE_BYTES {
        return Err(
            invalid(format!("is larger than {} KiB", MAX_FIXTURE_BYTES / 1024))
                .hint("Split it into several fixtures."),
        );
    }
    let fixture: Fixture = serde_json::from_slice(bytes).map_err(|error| {
        let (line, column) = (error.line() as u32, error.column() as u32);
        let message = error.to_string();
        let message = message
            .rsplit_once(" at line ")
            .map_or(message.as_str(), |(message, _)| message);
        invalid(message.to_owned())
            .at(line, column)
            .hint("See Fixtures in docs/features/extensions.md for the format.")
    })?;
    if fixture.fixture != VERSION {
        return Err(invalid(format!(
            "states fixture format {}, but this Dupin reads format {VERSION}",
            fixture.fixture
        ))
        .field("fixture")
        .hint(format!("Set `\"fixture\": {VERSION}`.")));
    }
    let environment = &fixture.environment;
    let field = |field: &str, message: String| invalid(message).field(format!("environment.{field}"));
    if parse_time(&environment.now).is_none() {
        return Err(field("now", format!("{:?} is not a UTC time", environment.now))
            .hint("Write it as `2026-01-01T09:00:00Z`."));
    }
    if !is_locale(&environment.locale) {
        return Err(field(
            "locale",
            format!("{:?} is not a language tag", environment.locale),
        )
        .hint("Use a tag such as `en-GB`."));
    }
    if !is_time_zone(&environment.time_zone) {
        return Err(field(
            "timeZone",
            format!("{:?} is not a time zone name", environment.time_zone),
        )
        .hint("Use a name from the tz database, such as `UTC` or `Europe/London`."));
    }
    let window = environment.window;
    if !(400..=1200).contains(&window.width) {
        return Err(field("window.width", "must be 400–1200 points".into()));
    }
    if window.height.is_some_and(|height| !(60..=2000).contains(&height)) {
        return Err(field("window.height", "must be 60–2000 points".into()));
    }
    if !(1..=3).contains(&window.scale) {
        return Err(field("window.scale", "must be 1, 2 or 3".into()));
    }
    if fixture.steps.is_empty() || fixture.steps.len() > MAX_STEPS {
        return Err(invalid(format!("must have 1–{MAX_STEPS} steps")).field("steps"));
    }
    if fixture.steps[0].query.is_none() {
        return Err(invalid("must start with a `query` step".into())
            .field("steps[0]")
            .hint("Type a query first: `{ \"query\": \"…\" }`."));
    }
    for (index, step) in fixture.steps.iter().enumerate() {
        let at = |message: String| invalid(message).field(format!("steps[{index}]"));
        let given = [
            step.query.is_some(),
            step.select.is_some(),
            step.activate.is_some(),
            step.event.is_some(),
        ];
        if given.iter().filter(|given| **given).count() != 1 {
            return Err(at(
                "must have exactly one of `query`, `select`, `activate` and `event`".into(),
            ));
        }
        if step.data.is_some() && step.event.is_none() {
            return Err(at("`data` goes with an `event`".into()));
        }
        if step
            .query
            .as_ref()
            .is_some_and(|query| query.len() > MAX_QUERY_BYTES || query.contains('\n'))
        {
            return Err(at(format!(
                "`query` must be one line of at most {MAX_QUERY_BYTES} bytes"
            )));
        }
        if let Some(key) = step
            .expect
            .iter()
            .flat_map(Map::keys)
            .find(|key| !EXPECTATIONS.contains(&key.as_str()))
        {
            return Err(at(format!("`expect` has no `{key}`"))
                .hint(format!("Expect any of {}.", EXPECTATIONS.join(", "))));
        }
    }
    for (scope, reply) in &fixture.replies.files {
        let paths: Vec<&String> = match reply {
            FilesReply::File(_) => Vec::new(),
            FilesReply::Directory(files) => files.keys().collect(),
        };
        if let Some(path) = paths.into_iter().find(|path| !is_relative_path(path)) {
            return Err(
                invalid(format!("{path:?} is not a relative path inside the scope"))
                    .field(format!("replies.files.{scope}"))
                    .hint("Name files such as `notes/today.md`, without `..` or a leading `/`."),
            );
        }
    }
    Ok(fixture)
}

/// `"2026-01-01T09:00:00Z"`, optionally with `.sss`, as milliseconds since the epoch.
pub fn parse_time(text: &str) -> Option<i64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let (time, millis) = match time.split_once('.') {
        Some((time, millis)) if millis.len() == 3 => (time, millis.parse::<i64>().ok()?),
        Some(_) => return None,
        None => (time, 0),
    };
    let numbers = |text: &str, lengths: &[usize]| -> Option<Vec<i64>> {
        let parts: Vec<&str> = text.split(if text.contains('-') { '-' } else { ':' }).collect();
        (parts.len() == lengths.len()
            && parts
                .iter()
                .zip(lengths)
                .all(|(part, len)| part.len() == *len && part.bytes().all(|b| b.is_ascii_digit())))
        .then(|| {
            parts
                .iter()
                .map(|part| part.parse().unwrap_or_default())
                .collect()
        })
    };
    let date = numbers(date, &[4, 2, 2])?;
    let time = numbers(time, &[2, 2, 2])?;
    let (year, month, day) = (date[0], date[1], date[2]);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&month)
        || !(1..=days_in_month[month as usize - 1]).contains(&day)
        || time[0] > 23
        || time[1] > 59
        || time[2] > 59
    {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + time[0]) * 60 + time[1]) * 60_000 + time[2] * 1000 + millis)
}

fn is_locale(tag: &str) -> bool {
    let mut parts = tag.split('-');
    parts.next().is_some_and(|language| {
        (2..=3).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_lowercase())
    }) && parts.all(|part| (2..=8).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_alphanumeric()))
        && tag.len() <= 35
}

/// A tz database name: words of letters, digits, `_`, `-` and `+` joined by `/`, never `..`.
fn is_time_zone(name: &str) -> bool {
    name.len() <= 64
        && !name.is_empty()
        && name.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+'))
        })
}

/// The `.json` files in the package's `fixtures/` folder, by name. Symbolic links are refused, so
/// a fixture can't make the runner read outside the package.
pub fn discover(package: &Path) -> Result<Vec<(String, PathBuf)>, Diagnostic> {
    let folder = package.join(FOLDER);
    let unreadable = |error: std::io::Error| {
        Diagnostic::new("fixtures-unreadable", format!("Couldn't read it: {error}")).file(FOLDER)
    };
    let none = || {
        Diagnostic::new("no-fixtures", "has no fixtures to run")
            .file(FOLDER)
            .hint("Add a fixture such as `fixtures/search.json`; `dupin ext new` writes one.")
    };
    match std::fs::symlink_metadata(&folder) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(
                Diagnostic::new("fixtures-unreadable", "must be a folder, not a link or a file").file(FOLDER),
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(none()),
        Err(error) => return Err(unreadable(error)),
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&folder).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        let relative = format!("{FOLDER}/{name}");
        if !entry.file_type().map_err(unreadable)?.is_file() {
            return Err(
                Diagnostic::new("invalid-fixture", "must be a file, not a link or a folder")
                    .file(relative)
                    .hint("Copy the fixture into the package instead of linking it."),
            );
        }
        found.push((relative, entry.path()));
    }
    if found.is_empty() {
        return Err(none());
    }
    if found.len() > MAX_FIXTURES {
        return Err(Diagnostic::new(
            "too-many-fixtures",
            format!("has more than {MAX_FIXTURES} fixtures"),
        )
        .file(FOLDER));
    }
    found.sort();
    Ok(found)
}

/// Reads the fixture at `path`, at most [`MAX_FIXTURE_BYTES`] of it.
pub fn read(name: &str, path: &Path) -> Result<Fixture, Diagnostic> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_FIXTURE_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|error| {
            Diagnostic::new("fixtures-unreadable", format!("Couldn't read it: {error}")).file(name)
        })?;
    parse(name, &bytes)
}

/// The fixture as `--update` writes it: tab-indented JSON, keys in the format's order.
pub fn to_json(fixture: &Fixture) -> String {
    let mut out = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(
        &mut out,
        serde_json::ser::PrettyFormatter::with_indent(b"\t"),
    );
    fixture
        .serialize(&mut serializer)
        .expect("a fixture always serializes");
    out.push(b'\n');
    String::from_utf8(out).expect("JSON is UTF-8")
}

/// The rows of an answer as fixtures state them: what Dupin shows and what each action does.
pub fn rows(rows: &[Row]) -> Value {
    rows.iter()
        .map(|row| {
            let mut out = Map::new();
            out.insert("id".into(), row.id.clone().into());
            out.insert("title".into(), row.title.clone().into());
            if !row.subtitle.is_empty() {
                out.insert("subtitle".into(), row.subtitle.clone().into());
            }
            if !row.keywords.is_empty() {
                out.insert("keywords".into(), row.keywords.clone().into());
            }
            if let Some(icon) = &row.icon {
                out.insert("icon".into(), icon.clone().into());
            }
            if let Some(section) = &row.section {
                out.insert("section".into(), section.clone().into());
            }
            if let Some(detail) = &row.detail {
                out.insert(
                    "detail".into(),
                    serde_json::to_value(detail).expect("a detail always serializes"),
                );
            }
            let actions: Vec<Value> = row
                .actions
                .iter()
                .map(|action| {
                    let mut entry = effect(&action.effect);
                    entry.insert("title".into(), action.title.clone().into());
                    Value::Object(entry)
                })
                .collect();
            out.insert("actions".into(), actions.into());
            Value::Object(out)
        })
        .collect()
}

/// What choosing an action does, as `{ "copy": … }`, `{ "paste": … }`, `{ "url": … }` or
/// `{ "run": … }`.
pub fn effect(effect: &RowEffect) -> Map<String, Value> {
    let (key, value) = match effect {
        RowEffect::Copy { text } => ("copy", Value::from(text.clone())),
        RowEffect::Paste { text } => ("paste", Value::from(text.clone())),
        RowEffect::OpenUrl { url } => ("url", Value::from(url.clone())),
        RowEffect::Run { handler, data } => ("run", json!({"action": handler, "data": data})),
    };
    Map::from_iter([(key.to_owned(), value)])
}

/// The error an answer failed with, as fixtures state it.
pub fn error(diagnostic: &Diagnostic) -> Value {
    serde_json::to_value(diagnostic).expect("a diagnostic always serializes")
}

/// Where `actual` differs from `expected`, for the keys `expected` names. Each difference names
/// the JSON path, such as `rows[0].title`.
pub fn compare(expected: &Map<String, Value>, actual: &Map<String, Value>) -> Vec<Difference> {
    let mut differences = Vec::new();
    for (key, want) in expected {
        let got = actual.get(key).unwrap_or(&Value::Null);
        walk(key.clone(), want, got, &mut differences);
    }
    differences
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Difference {
    path: String,
    /// Absent when the outcome has something the fixture doesn't expect.
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<Value>,
    /// Absent when the outcome lacks something the fixture expects.
    #[serde(skip_serializing_if = "Option::is_none")]
    actual: Option<Value>,
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.expected, &self.actual) {
            (Some(expected), Some(actual)) => write!(f, "{}: expected {expected}, got {actual}", self.path),
            (Some(expected), None) => write!(f, "{}: expected {expected}, got nothing", self.path),
            (None, Some(actual)) => write!(f, "{}: unexpected {actual}", self.path),
            (None, None) => write!(f, "{}", self.path),
        }
    }
}

fn walk(path: String, expected: &Value, actual: &Value, out: &mut Vec<Difference>) {
    match (expected, actual) {
        (Value::Object(want), Value::Object(got)) => {
            for (key, value) in want {
                let at = format!("{path}.{key}");
                match got.get(key) {
                    Some(actual) => walk(at, value, actual, out),
                    None => out.push(Difference {
                        path: at,
                        expected: Some(value.clone()),
                        actual: None,
                    }),
                }
            }
            for (key, value) in got.iter().filter(|(key, _)| !want.contains_key(*key)) {
                out.push(Difference {
                    path: format!("{path}.{key}"),
                    expected: None,
                    actual: Some(value.clone()),
                });
            }
        }
        (Value::Array(want), Value::Array(got)) => {
            for (index, pair) in want.iter().zip(got).enumerate() {
                walk(format!("{path}[{index}]"), pair.0, pair.1, out);
            }
            for (index, value) in want.iter().enumerate().skip(got.len()) {
                out.push(Difference {
                    path: format!("{path}[{index}]"),
                    expected: Some(value.clone()),
                    actual: None,
                });
            }
            for (index, value) in got.iter().enumerate().skip(want.len()) {
                out.push(Difference {
                    path: format!("{path}[{index}]"),
                    expected: None,
                    actual: Some(value.clone()),
                });
            }
        }
        (want, got) if want == got => {}
        (want, got) => out.push(Difference {
            path,
            expected: Some(want.clone()),
            actual: (!got.is_null()).then(|| got.clone()),
        }),
    }
}

#[cfg(test)]
mod tests;
