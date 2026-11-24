//! Fixtures: JSON files in a package's `fixtures/` folder that replay input against the package
//! and state what it should answer. `dupin ext test` runs every one through the extension host;
//! `dupin ext snap` renders one step's panel. A fixture fixes everything a run could otherwise
//! take from the machine: the clock, `Math.random`'s seed, the locale, the time zone, the theme
//! and the window. Each host service an extension asks for is answered from the fixture's
//! `replies`, never from the machine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dupin_js_runtime::Diagnostic;
use dupin_plugin_protocol::{Effect, PluginItem, View};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The format version a fixture states in `"fixture"`.
pub const VERSION: u32 = 1;
pub const FOLDER: &str = "fixtures";
pub const MAX_FIXTURE_BYTES: usize = 1024 * 1024;
pub const MAX_FIXTURES: usize = 64;
pub const MAX_STEPS: usize = 64;
pub const MAX_QUERY_BYTES: usize = 4096;
/// What a step may expect, as keys of its outcome.
pub const EXPECTATIONS: [&str; 7] = [
    "rows", "selected", "effect", "error", "sections", "detail", "toasts",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub fixture: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default, skip_serializing_if = "Replies::is_empty")]
    pub replies: Replies,
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
    pub query: Option<String>,
    /// Moves the selection to this row, counting from 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<usize>,
    /// Chooses the selected row's action with this title or id. Its effect is recorded, never
    /// carried out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action<'a> {
    Query(&'a str),
    Select(usize),
    Activate(&'a str),
}

impl Step {
    pub fn action(&self) -> Action<'_> {
        match (&self.query, self.select, &self.activate) {
            (Some(query), None, None) => Action::Query(query),
            (None, Some(row), None) => Action::Select(row),
            (None, None, Some(action)) => Action::Activate(action),
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
        }
    }
}

/// The host services' answers, by capability. A request no reply matches fails with
/// `unavailable`, so a fixture never falls through to the machine.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Replies {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub http: Vec<HttpReply>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileReply>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exec: Vec<ExecReply>,
    /// The extension's storage when the fixture starts, key by key.
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub storage: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<ClipboardReplies>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dictionary: Vec<DictionaryReply>,
    /// The events a calendar read sees, in the calendar service's shape.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub calendar: Vec<Value>,
    /// Your decision for each scope, named as the broker names grants, such as
    /// `http:https://api.github.com`. A declared scope left out is granted.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub consent: BTreeMap<String, Consent>,
}

impl Replies {
    pub fn is_empty(&self) -> bool {
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
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
}

fn ok() -> u16 {
    200
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileReply {
    /// The `files` scope's name, as `dupin.capabilities.files` declares it.
    pub scope: String,
    /// Relative to the scope's root; `""` is the root.
    #[serde(default)]
    pub path: String,
    /// What a listing returns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<Value>>,
    /// What a read returns, as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecReply {
    /// The `exec` profile or `@script:` target.
    pub target: String,
    /// Matches any arguments when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub status: i32,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClipboardReplies {
    /// What a read returns: `{ "change": …, "content": … }`, or `null` for an empty clipboard.
    pub read: Value,
    /// The copies a watch delivers, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DictionaryReply {
    pub term: String,
    pub entries: Vec<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Consent {
    Allow,
    Deny,
    /// Not answered yet: the request fails with `needs-consent`.
    Pending,
}

/// A host service request, as the broker bridge decodes it from the guest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request<'a> {
    Http { method: &'a str, url: &'a str },
    FilesList { scope: &'a str, path: &'a str },
    FilesRead { scope: &'a str, path: &'a str },
    Exec { target: &'a str, args: &'a [String] },
    StorageGet { key: &'a str },
    ClipboardRead,
    DictionaryLookup { term: &'a str },
    CalendarRead { start: i64, end: i64 },
}

/// The error a guest's call gets, with the codes the host services use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReplyError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl Replies {
    /// Answers `request`, which the package's grant `scope` covers, from the fixture alone.
    pub fn answer(&self, scope: &str, request: &Request<'_>) -> Result<Value, ReplyError> {
        match self.consent.get(scope) {
            Some(Consent::Deny) => {
                return Err(ReplyError {
                    code: "denied",
                    message: format!("the fixture denies {scope}"),
                    scope: Some(scope.into()),
                });
            }
            Some(Consent::Pending) => {
                return Err(ReplyError {
                    code: "needs-consent",
                    message: format!("{scope} is waiting for your decision"),
                    scope: Some(scope.into()),
                });
            }
            Some(Consent::Allow) | None => {}
        }
        let missing = |what: String| ReplyError {
            code: "unavailable",
            message: format!("the fixture has no reply for {what}"),
            scope: None,
        };
        match *request {
            Request::Http { method, url } => self
                .http
                .iter()
                .find(|reply| reply.request.method.eq_ignore_ascii_case(method) && reply.request.url == url)
                .map(|reply| {
                    let response = &reply.response;
                    let body = match (&response.json, &response.body) {
                        (Some(json), _) => json.to_string(),
                        (None, Some(body)) => body.clone(),
                        (None, None) => String::new(),
                    };
                    json!({
                        "status": response.status,
                        "ok": (200..300).contains(&response.status),
                        "headers": response.headers,
                        "body": body,
                    })
                })
                .ok_or_else(|| missing(format!("{} {url}", method.to_ascii_uppercase()))),
            Request::FilesList { scope, path } | Request::FilesRead { scope, path } => {
                let listing = matches!(request, Request::FilesList { .. });
                self.files
                    .iter()
                    .filter(|reply| reply.scope == scope && reply.path == path)
                    .find_map(|reply| {
                        if listing {
                            reply.entries.clone().map(Value::from)
                        } else {
                            reply.contents.clone().map(Value::from)
                        }
                    })
                    .ok_or_else(|| {
                        let verb = if listing { "listing" } else { "reading" };
                        missing(format!("{verb} {scope}:{path}"))
                    })
            }
            Request::Exec { target, args } => self
                .exec
                .iter()
                .find(|reply| reply.target == target && reply.args.as_deref().is_none_or(|want| want == args))
                .map(|reply| json!({"status": reply.status, "stdout": reply.stdout, "stderr": reply.stderr}))
                .ok_or_else(|| missing(format!("running {target}"))),
            Request::StorageGet { key } => Ok(self.storage.get(key).cloned().unwrap_or(Value::Null)),
            Request::ClipboardRead => self
                .clipboard
                .as_ref()
                .map(|clipboard| clipboard.read.clone())
                .ok_or_else(|| missing("reading the clipboard".into())),
            Request::DictionaryLookup { term } => self
                .dictionary
                .iter()
                .find(|reply| reply.term == term)
                .map(|reply| Value::from(reply.entries.clone()))
                .ok_or_else(|| missing(format!("looking up {}", Value::from(term)))),
            Request::CalendarRead { start, end } => Ok(Value::from(
                self.calendar
                    .iter()
                    .filter(|event| {
                        let at = |key| event.get(key).and_then(Value::as_i64);
                        at("start").is_some_and(|from| from < end) && at("end").is_some_and(|to| to > start)
                    })
                    .cloned()
                    .collect::<Vec<_>>(),
            )),
        }
    }
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
        ];
        if given.iter().filter(|given| **given).count() != 1 {
            return Err(at(
                "must have exactly one of `query`, `select` and `activate`".into()
            ));
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
pub fn rows(items: &[PluginItem]) -> Value {
    items
        .iter()
        .map(|item| {
            let mut row = Map::new();
            row.insert("id".into(), item.id.clone().into());
            match &item.view {
                View::Text { title, subtitle } => {
                    row.insert("title".into(), title.clone().into());
                    if !subtitle.is_empty() {
                        row.insert("subtitle".into(), subtitle.clone().into());
                    }
                }
                View::Detail { title, body } => {
                    row.insert("title".into(), title.clone().into());
                    row.insert("detail".into(), body.clone().into());
                }
                View::Pair { source, result } => {
                    row.insert("source".into(), source.clone().into());
                    row.insert("result".into(), result.clone().into());
                }
            }
            let actions: Vec<Value> = item
                .actions
                .iter()
                .map(|action| {
                    let mut entry = effect(&action.effect);
                    entry.insert("title".into(), action.title.clone().into());
                    Value::Object(entry)
                })
                .collect();
            row.insert("actions".into(), actions.into());
            Value::Object(row)
        })
        .collect()
}

/// What choosing an action would do, as `{ "copy": … }`, `{ "url": … }` or `{ "push": … }`.
pub fn effect(effect: &Effect) -> Map<String, Value> {
    let (key, value) = match effect {
        Effect::Copy { text } => ("copy", Value::from(text.clone())),
        Effect::OpenUrl { url } => ("url", Value::from(url.clone())),
        Effect::Push { view, title, .. } => ("push", json!({"view": view, "title": title})),
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
    pub path: String,
    /// Absent when the outcome has something the fixture doesn't expect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    /// Absent when the outcome lacks something the fixture expects.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
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
