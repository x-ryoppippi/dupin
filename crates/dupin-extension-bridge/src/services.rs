//! The host services the bridge calls once the broker allows an operation. Each is a narrow
//! trait, so the app supplies the macOS-backed ones and tests supply fixtures; none of them
//! checks authority, prompts or knows which package asked.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use dupin_extension_http as http;
use dupin_host_services::{calendar, exec, windows};
use serde::Serialize;
use serde_json::Value;

/// Why a host request failed, as the guest's `DupinError` receives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub(crate) code: String,
    pub(crate) message: String,
    /// The scope at stake, as grant files name it, such as `clipboard:read`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scope: Option<String>,
    /// What granting the scope allows, as consent shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    /// The macOS access the scope also needs, as Settings → Permissions names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) permission: Option<String>,
}

impl Failure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            scope: None,
            description: None,
            permission: None,
        }
    }

    /// What went wrong, as `DupinError.code` names it, such as `needs-consent`.
    pub fn code(&self) -> &str {
        &self.code
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }

    pub(crate) fn scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub trait Http: Send + Sync {
    /// Fetches `request` within `scope` until `cancel` fires. The app's implementation is
    /// [`http::HttpService::fetch`].
    fn fetch(
        &self,
        request: http::Request,
        scope: &http::HttpScope,
        cancel: &http::CancelToken,
    ) -> Result<http::Response, http::HttpError>;
}

impl Http for http::HttpService {
    fn fetch(
        &self,
        request: http::Request,
        scope: &http::HttpScope,
        cancel: &http::CancelToken,
    ) -> Result<http::Response, http::HttpError> {
        http::HttpService::fetch(self, request, scope, cancel)
    }
}

pub trait Exec: Send + Sync {
    /// Runs `profile` with `values`, as [`exec::run`] does with the app's script catalogue
    /// and runner.
    fn run(
        &self,
        profile: &exec::Profile,
        values: &BTreeMap<String, exec::Value>,
        cancel: &AtomicBool,
    ) -> Result<exec::Outcome, exec::ExecError>;
}

/// Packages' encrypted namespaces. The app's and the tests' implementation is
/// [`crate::PackageStorage`]; the bridge picks the namespace and the limits from the permit.
pub trait Storage: Send + Sync {
    /// `namespace`, open with `limits`, which apply at once if they changed since it opened.
    /// With `synced`, the package holds `storage:sync` and its namespace syncs under that name
    /// while sync is on; without, it stays on this Mac.
    fn open(
        &self,
        namespace: &str,
        limits: dupin_storage::Limits,
        synced: Option<&str>,
    ) -> Result<Arc<crate::storage::Namespace>, Failure>;
    /// The name `package`'s namespace syncs under on every Mac, when it has one: only a
    /// package installed from Git, whose source names it the same everywhere.
    fn synced_name(&self, package: &crate::PackageId) -> Option<String> {
        let _ = package;
        None
    }
    /// Deletes `namespace` and everything in it.
    fn discard(&self, namespace: &str) -> Result<(), Failure>;
}

impl<T: Storage> Storage for Arc<T> {
    fn open(
        &self,
        namespace: &str,
        limits: dupin_storage::Limits,
        synced: Option<&str>,
    ) -> Result<Arc<crate::storage::Namespace>, Failure> {
        T::open(self, namespace, limits, synced)
    }

    fn synced_name(&self, package: &crate::PackageId) -> Option<String> {
        T::synced_name(self, package)
    }

    fn discard(&self, namespace: &str) -> Result<(), Failure> {
        T::discard(self, namespace)
    }
}

/// Keeps a watch going while it is held.
pub type Subscription = Box<dyn Send + Sync>;

pub trait Clipboard: Send + Sync {
    /// The current clipboard as `{ "type": "text", "text": … }` or another content type, or
    /// none when it is empty or withheld.
    fn read(&self) -> Result<Option<Value>, Failure>;
    fn write_text(&self, text: &str) -> Result<(), Failure>;
    /// Puts a PNG on the clipboard.
    fn write_image(&self, _png: &[u8]) -> Result<(), Failure> {
        Err(Failure::new("unavailable", "images can't be copied here"))
    }
    /// Puts files on the clipboard as Finder's Copy does. The bridge has checked that a watch
    /// delivered them to the package.
    fn write_files(&self, _paths: &[std::path::PathBuf]) -> Result<(), Failure> {
        Err(Failure::new("unavailable", "files can't be copied here"))
    }
    /// Calls `changed` with each copy, shaped as [`Clipboard::read`] returns it.
    fn watch(&self, changed: Box<dyn Fn(Value) + Send + Sync>) -> Result<Subscription, Failure>;
}

pub trait Dictionary: Send + Sync {
    /// The entries for `word`, as a JSON array.
    fn lookup(&self, word: &str) -> Result<Value, Failure>;
}

pub trait Calculator: Send + Sync {
    /// `expression` worked out, as a JSON object, or `null` when it isn't a calculation the
    /// calculator answers in time. `cancelled` turns true once nobody waits for the answer.
    fn evaluate(&self, expression: &str, cancelled: &AtomicBool) -> Result<Value, Failure>;
}

/// A snippet a package asks Dupin to store, as the guest sent it, before Dupin checks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnippetDraft {
    pub keyword: String,
    pub name: Option<String>,
    pub text: String,
}

/// The snippets the user stores in Dupin, which only the user's own actions change.
pub trait Snippets: Send + Sync {
    /// Stores `draft` in place of the stored snippet with the keyword `replaces`, or as a new
    /// one without it. Returns what is wrong with each field, by field name, as a JSON object
    /// that is empty when the snippet was stored.
    fn save(&self, draft: SnippetDraft, replaces: Option<&str>) -> Result<Value, Failure>;
    /// Deletes the stored snippet `keyword`, and says whether there was one. config.toml's
    /// snippets are not stored and stay.
    fn delete(&self, keyword: &str) -> Result<bool, Failure>;
}

pub trait Calendar: Send + Sync {
    /// The whole local days before and after today that `[from, to)` (milliseconds since the
    /// epoch) reaches, which the broker checks against the declared window.
    fn window(&self, from: i64, to: i64) -> Result<(u32, u32), Failure>;
    /// Events overlapping `[from, to)` whose title or location holds every word of `query`, at
    /// most `limit` from the end `order` names, inside `scope`'s window, as a JSON array whose
    /// `start` and `end` are milliseconds since the epoch.
    fn events(
        &self,
        from: i64,
        to: i64,
        query: Option<&str>,
        limit: usize,
        order: calendar::Order,
        scope: &dupin_broker::Scope,
    ) -> Result<Value, Failure>;
}

pub trait Windows: Send + Sync {
    /// Every display, the primary one first, as a JSON array.
    fn displays(&self) -> Result<Value, Failure>;
    /// The focused window of the app in front of Dupin and that app, or `null`.
    fn focused(&self) -> Result<Value, Failure>;
    /// Moves and resizes that window to `frame`, returning the frame it took.
    fn set_frame(&self, frame: windows::Frame) -> Result<Value, Failure>;
}

pub trait Context: Send + Sync {
    /// The app in front when the panel opened, or `null`.
    fn frontmost(&self) -> Result<Value, Failure>;
    /// The file the front app's focused window showed then, or `null`.
    fn document(&self) -> Result<Value, Failure>;
}

/// What a host service is missing, such as one a headless run has no use for.
pub struct Unavailable(pub &'static str);

impl Unavailable {
    fn failure(&self) -> Failure {
        Failure::new("unavailable", self.0)
    }
}

impl Http for Unavailable {
    fn fetch(
        &self,
        _request: http::Request,
        _scope: &http::HttpScope,
        _cancel: &http::CancelToken,
    ) -> Result<http::Response, http::HttpError> {
        Err(http::HttpError::Network(self.0.into()))
    }
}

impl Exec for Unavailable {
    fn run(
        &self,
        _profile: &exec::Profile,
        _values: &BTreeMap<String, exec::Value>,
        _cancel: &AtomicBool,
    ) -> Result<exec::Outcome, exec::ExecError> {
        Err(exec::ExecError::Unavailable(self.0.into()))
    }
}

impl Storage for Unavailable {
    fn open(
        &self,
        _namespace: &str,
        _limits: dupin_storage::Limits,
        _synced: Option<&str>,
    ) -> Result<Arc<crate::storage::Namespace>, Failure> {
        Err(self.failure())
    }
    fn discard(&self, _namespace: &str) -> Result<(), Failure> {
        Err(self.failure())
    }
}

impl Clipboard for Unavailable {
    fn read(&self) -> Result<Option<Value>, Failure> {
        Err(self.failure())
    }
    fn write_text(&self, _text: &str) -> Result<(), Failure> {
        Err(self.failure())
    }
    fn watch(&self, _changed: Box<dyn Fn(Value) + Send + Sync>) -> Result<Subscription, Failure> {
        Err(self.failure())
    }
}

impl Dictionary for Unavailable {
    fn lookup(&self, _word: &str) -> Result<Value, Failure> {
        Err(self.failure())
    }
}

impl Calculator for Unavailable {
    fn evaluate(&self, _expression: &str, _cancelled: &AtomicBool) -> Result<Value, Failure> {
        Err(self.failure())
    }
}

impl Snippets for Unavailable {
    fn save(&self, _draft: SnippetDraft, _replaces: Option<&str>) -> Result<Value, Failure> {
        Err(self.failure())
    }
    fn delete(&self, _keyword: &str) -> Result<bool, Failure> {
        Err(self.failure())
    }
}

impl Calendar for Unavailable {
    fn window(&self, _from: i64, _to: i64) -> Result<(u32, u32), Failure> {
        Err(self.failure())
    }
    fn events(
        &self,
        _from: i64,
        _to: i64,
        _query: Option<&str>,
        _limit: usize,
        _order: calendar::Order,
        _scope: &dupin_broker::Scope,
    ) -> Result<Value, Failure> {
        Err(self.failure())
    }
}

impl Windows for Unavailable {
    fn displays(&self) -> Result<Value, Failure> {
        Err(self.failure())
    }
    fn focused(&self) -> Result<Value, Failure> {
        Err(self.failure())
    }
    fn set_frame(&self, _frame: windows::Frame) -> Result<Value, Failure> {
        Err(self.failure())
    }
}

impl Context for Unavailable {
    fn frontmost(&self) -> Result<Value, Failure> {
        Err(self.failure())
    }
    fn document(&self) -> Result<Value, Failure> {
        Err(self.failure())
    }
}

impl From<dupin_host_services::ServiceError> for Failure {
    fn from(error: dupin_host_services::ServiceError) -> Self {
        Failure {
            permission: error.permission().map(|permission| permission.name().to_owned()),
            ..Failure::new(error.code(), error.to_string())
        }
    }
}

/// Every service the bridge can reach.
pub struct Services {
    pub http: Box<dyn Http>,
    pub exec: Box<dyn Exec>,
    pub storage: Box<dyn Storage>,
    pub clipboard: Box<dyn Clipboard>,
    pub dictionary: Box<dyn Dictionary>,
    pub calculator: Box<dyn Calculator>,
    pub snippets: Box<dyn Snippets>,
    pub calendar: Box<dyn Calendar>,
    pub windows: Box<dyn Windows>,
    pub context: Box<dyn Context>,
}

impl Services {
    /// No service at all: every allowed operation then fails as unavailable.
    pub fn none() -> Self {
        Self {
            http: Box::new(Unavailable("network access isn't available")),
            exec: Box::new(Unavailable("running programs isn't available")),
            storage: Box::new(Unavailable("storage isn't available")),
            clipboard: Box::new(Unavailable("the clipboard isn't available")),
            dictionary: Box::new(Unavailable("the dictionary isn't available")),
            calculator: Box::new(Unavailable("the calculator isn't available")),
            snippets: Box::new(Unavailable("snippets aren't available")),
            calendar: Box::new(Unavailable("calendars aren't available")),
            windows: Box::new(Unavailable("windows aren't available")),
            context: Box::new(Unavailable("what you were using isn't available")),
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! In-memory services for tests: nothing here touches the network, the clipboard, calendars,
    //! windows or the file system.

    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;

    /// Snippets by keyword, refusing a keyword with a space as Dupin does.
    #[derive(Clone, Default)]
    pub(crate) struct Snippets(pub(crate) Arc<Mutex<BTreeMap<String, SnippetDraft>>>);

    impl super::Snippets for Snippets {
        fn save(&self, draft: SnippetDraft, replaces: Option<&str>) -> Result<Value, Failure> {
            if draft.keyword.contains(' ') {
                return Ok(serde_json::json!({ "keyword": "A keyword has no spaces" }));
            }
            let mut stored = self.0.lock().unwrap();
            if let Some(replaced) = replaces {
                stored.remove(replaced);
            }
            stored.insert(draft.keyword.clone(), draft);
            Ok(serde_json::json!({}))
        }
        fn delete(&self, keyword: &str) -> Result<bool, Failure> {
            Ok(self.0.lock().unwrap().remove(keyword).is_some())
        }
    }

    /// Answers every fetch with `body` after `delay`, recording each request.
    #[derive(Default)]
    pub(crate) struct Http {
        pub(crate) body: String,
        pub(crate) delay: Mutex<Duration>,
        pub(crate) requests: Mutex<Vec<http::Request>>,
        pub(crate) finished: AtomicUsize,
    }

    impl super::Http for Arc<Http> {
        fn fetch(
            &self,
            request: http::Request,
            _scope: &http::HttpScope,
            cancel: &http::CancelToken,
        ) -> Result<http::Response, http::HttpError> {
            self.requests.lock().unwrap().push(request.clone());
            let delay = *self.delay.lock().unwrap();
            let until = std::time::Instant::now() + delay;
            while std::time::Instant::now() < until {
                if cancel.is_cancelled() {
                    return Err(http::HttpError::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            self.finished.fetch_add(1, Ordering::Relaxed);
            Ok(http::Response {
                status: 200,
                url: request.url,
                redirected: false,
                headers: vec![("content-type".into(), "application/json".into())],
                body: self.body.clone().into_bytes(),
            })
        }
    }

    /// Records each profile it is asked to run and exits 0 with `stdout`.
    #[derive(Default)]
    pub(crate) struct Exec {
        pub(crate) stdout: String,
        pub(crate) runs: Mutex<Vec<(String, BTreeMap<String, exec::Value>)>>,
        /// The program and the timeout of each run, in the same order.
        pub(crate) launches: Mutex<Vec<(exec::Target, std::time::Duration)>>,
    }

    impl super::Exec for Arc<Exec> {
        fn run(
            &self,
            profile: &exec::Profile,
            values: &BTreeMap<String, exec::Value>,
            _cancel: &AtomicBool,
        ) -> Result<exec::Outcome, exec::ExecError> {
            profile.bind(values)?;
            self.runs
                .lock()
                .unwrap()
                .push((profile.name().into(), values.clone()));
            self.launches
                .lock()
                .unwrap()
                .push((profile.target().clone(), profile.options().timeout));
            Ok(exec::Outcome {
                exit: exec::Exit::Code(0),
                output: profile.options().return_output.then(|| exec::Output {
                    stdout: self.stdout.clone(),
                    stderr: String::new(),
                    truncated: false,
                }),
            })
        }
    }

    /// A clipboard holding `text`, whose watchers [`Clipboard::copy`] notifies.
    #[derive(Default)]
    pub(crate) struct Clipboard {
        pub(crate) text: Mutex<Option<String>>,
        #[allow(clippy::type_complexity)]
        watchers: Mutex<Vec<(u64, Arc<dyn Fn(Value) + Send + Sync>)>>,
        next: AtomicUsize,
    }

    impl Clipboard {
        /// Copies `text`, as another app would.
        pub(crate) fn copy(&self, text: &str) {
            *self.text.lock().unwrap() = Some(text.into());
            let watchers: Vec<_> = self
                .watchers
                .lock()
                .unwrap()
                .iter()
                .map(|(_, watcher)| watcher.clone())
                .collect();
            for watcher in watchers {
                watcher(serde_json::json!({ "type": "text", "text": text }));
            }
        }

        pub(crate) fn watchers(&self) -> usize {
            self.watchers.lock().unwrap().len()
        }
    }

    struct Unwatch(Arc<Clipboard>, u64);

    impl Drop for Unwatch {
        fn drop(&mut self) {
            self.0.watchers.lock().unwrap().retain(|(id, _)| *id != self.1);
        }
    }

    impl super::Clipboard for Arc<Clipboard> {
        fn read(&self) -> Result<Option<Value>, Failure> {
            Ok(self
                .text
                .lock()
                .unwrap()
                .clone()
                .map(|text| serde_json::json!({ "type": "text", "text": text })))
        }
        fn write_text(&self, text: &str) -> Result<(), Failure> {
            *self.text.lock().unwrap() = Some(text.into());
            Ok(())
        }
        fn write_image(&self, png: &[u8]) -> Result<(), Failure> {
            *self.text.lock().unwrap() = Some(format!("image of {} bytes", png.len()));
            Ok(())
        }
        fn write_files(&self, paths: &[std::path::PathBuf]) -> Result<(), Failure> {
            let paths: Vec<_> = paths.iter().map(|path| path.display().to_string()).collect();
            *self.text.lock().unwrap() = Some(paths.join("\n"));
            Ok(())
        }
        fn watch(&self, changed: Box<dyn Fn(Value) + Send + Sync>) -> Result<Subscription, Failure> {
            let id = self.next.fetch_add(1, Ordering::Relaxed) as u64;
            self.watchers.lock().unwrap().push((id, Arc::from(changed)));
            Ok(Box::new(Unwatch(self.clone(), id)))
        }
    }

    pub(crate) struct Dictionary;

    impl super::Dictionary for Dictionary {
        fn lookup(&self, word: &str) -> Result<Value, Failure> {
            Ok(serde_json::json!([{
                "headword": word,
                "dictionary": "Fixture",
                "pronunciation": null,
                "groups": [{ "partOfSpeech": "noun", "note": null, "senses": [{ "definition": format!("the word {word}"), "examples": [], "senses": [] }] }],
                "sections": [],
                "truncated": false,
            }]))
        }
    }

    /// Answers `1+1`, and nothing else.
    pub(crate) struct Calculator;

    impl super::Calculator for Calculator {
        fn evaluate(&self, expression: &str, _cancelled: &AtomicBool) -> Result<Value, Failure> {
            Ok(if expression == "1+1" {
                serde_json::json!({ "result": "2", "number": "2", "parts": [{ "kind": "number", "text": "2" }] })
            } else {
                Value::Null
            })
        }
    }

    /// One display, and a focused window that each move records.
    #[derive(Default)]
    pub(crate) struct Windows {
        pub(crate) moves: Mutex<Vec<super::windows::Frame>>,
    }

    impl super::Windows for Arc<Windows> {
        fn displays(&self) -> Result<Value, Failure> {
            Ok(serde_json::json!([{
                "frame": { "x": 0, "y": 0, "width": 1440, "height": 900 },
                "visibleFrame": { "x": 0, "y": 25, "width": 1440, "height": 875 },
                "scale": 2,
            }]))
        }
        fn focused(&self) -> Result<Value, Failure> {
            Ok(serde_json::json!({
                "frame": { "x": 10, "y": 40, "width": 300, "height": 200 },
                "app": { "name": "Notes", "bundleId": "com.apple.Notes" },
            }))
        }
        fn set_frame(&self, frame: super::windows::Frame) -> Result<Value, Failure> {
            self.moves.lock().unwrap().push(frame);
            Ok(serde_json::to_value(frame).unwrap())
        }
    }

    /// An editor in front, showing a note.
    pub(crate) struct Context;

    impl super::Context for Context {
        fn frontmost(&self) -> Result<Value, Failure> {
            Ok(serde_json::json!({ "name": "Editor", "bundleId": "com.example.editor", "pid": 4242 }))
        }
        fn document(&self) -> Result<Value, Failure> {
            Ok(serde_json::json!({ "path": "/Users/me/notes.md" }))
        }
    }

    /// Today starts at `today`; one event a day at noon.
    pub(crate) struct Calendar {
        pub(crate) today: i64,
    }

    const DAY: i64 = 24 * 60 * 60 * 1000;

    impl super::Calendar for Calendar {
        fn window(&self, from: i64, to: i64) -> Result<(u32, u32), Failure> {
            let offset = |at: i64| (at - self.today).div_euclid(DAY);
            let days = |count: i64| u32::try_from(count.max(0)).unwrap_or(u32::MAX);
            Ok((days(-offset(from)), days(offset(to - 1))))
        }
        fn events(
            &self,
            from: i64,
            to: i64,
            query: Option<&str>,
            limit: usize,
            _order: calendar::Order,
            _scope: &dupin_broker::Scope,
        ) -> Result<Value, Failure> {
            if query.is_some_and(|query| !"standup".contains(&query.trim().to_lowercase())) {
                return Ok(Value::Array(Vec::new()));
            }
            let first = (from - self.today).div_euclid(DAY);
            let events: Vec<_> = (first..)
                .map(|day| self.today + day * DAY + DAY / 2)
                .take_while(|start| *start < to)
                .filter(|start| *start >= from)
                .take(limit)
                .map(|start| {
                    serde_json::json!({ "id": format!("standup-{start}"), "title": "Standup", "start": start, "end": start + DAY / 24, "allDay": false, "calendar": { "name": "Work", "color": null }, "location": null, "url": null, "link": format!("ical://ekevent/standup-{start}?method=show&options=more") })
                })
                .collect();
            Ok(Value::Array(events))
        }
    }
}
