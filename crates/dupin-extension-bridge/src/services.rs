//! The host services the bridge calls once the broker allows an operation. Each is a narrow
//! trait, so the app supplies the macOS-backed ones and tests supply fixtures; none of them
//! checks authority, prompts or knows which package asked.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use dupin_extension_http as http;
use dupin_host_services::{calendar, exec};
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
    fn open(
        &self,
        namespace: &str,
        limits: dupin_storage::Limits,
    ) -> Result<Arc<crate::storage::Namespace>, Failure>;
    /// Deletes `namespace` and everything in it.
    fn discard(&self, namespace: &str) -> Result<(), Failure>;
}

impl<T: Storage> Storage for Arc<T> {
    fn open(
        &self,
        namespace: &str,
        limits: dupin_storage::Limits,
    ) -> Result<Arc<crate::storage::Namespace>, Failure> {
        T::open(self, namespace, limits)
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
    /// Calls `changed` with each copy, shaped as [`Clipboard::read`] returns it.
    fn watch(&self, changed: Box<dyn Fn(Value) + Send + Sync>) -> Result<Subscription, Failure>;
}

pub trait Dictionary: Send + Sync {
    /// The entries for `word`, as a JSON array.
    fn lookup(&self, word: &str) -> Result<Value, Failure>;
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
    pub calendar: Box<dyn Calendar>,
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
            calendar: Box::new(Unavailable("calendars aren't available")),
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! In-memory services for tests: nothing here touches the network, the clipboard, calendars
    //! or the file system.

    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;

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
