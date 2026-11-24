//! A broker bridge for one fixture: the package installed at its revision with the fixture's
//! consent, behind host services that answer from the fixture's `replies` alone. Requests go
//! through the same decoding and broker checks as the launcher's; only the services differ, and
//! none of them reaches the network, a program, the clipboard, calendars or Dupin's storage.
//! A `files` scope is bound to a scratch folder holding the fixture's files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use dupin_ext_sdk::fixture::{Consent, FilesReply, Replies};
use dupin_extension_bridge::services::{self, Services, Subscription};
use dupin_extension_bridge::{
    Base, Bridge, Broker, Failure, FilesKind, Local, OsAccess, OsPermission, Scope,
};
use dupin_extension_http as http;
use dupin_host_services::exec;
use dupin_js_runtime::Diagnostic;
use serde_json::{Map, Value};

use super::super::Extension;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Bound to every executable profile; the fixture's exec service never runs it.
const UNRUN_PROGRAM: &str = "/usr/bin/false";

/// What the package did through the services during one step.
#[derive(Default)]
pub(super) struct Effects {
    pub clipboard: Vec<String>,
    pub storage_changed: bool,
}

pub(super) struct Fixture {
    pub bridge: Bridge,
    state: Arc<State>,
    /// The `files` scopes' contents, removed when the fixture ends.
    _scratch: tempfile::TempDir,
}

struct State {
    replies: Replies,
    now: i64,
    storage: Mutex<Map<String, Value>>,
    effects: Mutex<Effects>,
}

impl Fixture {
    /// `name` is the fixture's path in the package, for diagnostics.
    pub fn open(extension: &Extension, name: &str, replies: &Replies, now: i64) -> Result<Self, Diagnostic> {
        let invalid = |message: String, field: &str| {
            Diagnostic::new("invalid-fixture", message)
                .file(name)
                .field(field.to_owned())
        };
        let scratch = tempfile::Builder::new()
            .prefix("dupin-fixture-")
            .tempdir()
            .map_err(|error| {
                Diagnostic::new(
                    "fixtures-unreadable",
                    format!("couldn't make a scratch folder: {error}"),
                )
            })?;
        let state = Arc::new(State {
            replies: replies.clone(),
            now,
            storage: Mutex::new(replies.storage.clone()),
            effects: Mutex::default(),
        });
        let services = Services {
            http: Box::new(Shared(state.clone())),
            exec: Box::new(Shared(state.clone())),
            storage: Box::new(Shared(state.clone())),
            clipboard: Box::new(Shared(state.clone())),
            dictionary: Box::new(Shared(state.clone())),
            calendar: Box::new(Shared(state.clone())),
        };
        let bridge = Bridge::new(Broker::new(Base::default(), Local::default()), None, services);
        let id = &extension.package;
        bridge.install(id, extension.revision.clone(), extension.capabilities.clone());
        for permission in OsPermission::ALL {
            bridge.set_os_access(permission, OsAccess::Allowed);
        }
        let refused = |error: dupin_extension_bridge::SaveError| {
            Diagnostic::new(
                "invalid-fixture",
                format!("the broker refused the fixture's consent: {error}"),
            )
            .file(name)
        };
        bridge.set_enabled(id, true).map_err(refused)?;
        let scopes = bridge.scopes(id);
        let declared: Vec<String> = scopes.iter().map(|entry| entry.scope.key().to_string()).collect();
        if let Some(key) = replies.consent.keys().find(|key| !declared.contains(key)) {
            return Err(invalid(
                format!("the package doesn't declare {key}"),
                &format!("replies.consent.{key}"),
            )
            .hint(format!("Name a declared scope: {}.", declared.join(", "))));
        }
        let files_scopes: Vec<String> = scopes
            .iter()
            .filter_map(|entry| match &entry.scope {
                Scope::Files { name, .. } => Some(name.as_str().to_owned()),
                _ => None,
            })
            .collect();
        if let Some(scope) = replies.files.keys().find(|scope| !files_scopes.contains(scope)) {
            return Err(invalid(
                format!("the package declares no files scope {scope:?}"),
                &format!("replies.files.{scope}"),
            ));
        }
        let revision = extension.revision.clone();
        for entry in scopes {
            let key = entry.scope.key();
            let consent = replies
                .consent
                .get(&key.to_string())
                .copied()
                .unwrap_or(Consent::Allow);
            if consent == Consent::Pending {
                continue;
            }
            let binding = match &entry.scope {
                Scope::Files { name, kind, .. } => Some(
                    materialize(
                        scratch.path(),
                        name.as_str(),
                        *kind,
                        replies.files.get(name.as_str()),
                    )
                    .map_err(|error| {
                        Diagnostic::new(
                            "fixtures-unreadable",
                            format!("couldn't write the fixture's files: {error}"),
                        )
                        .file(name.as_str().to_owned())
                    })?,
                ),
                scope if scope.needs_binding() => Some(PathBuf::from(UNRUN_PROGRAM)),
                _ => None,
            };
            bridge
                .allow(id, &revision, &entry.scope, binding)
                .map_err(refused)?;
            if consent == Consent::Deny {
                bridge.revoke(id, &key).map_err(refused)?;
            }
        }
        Ok(Self {
            bridge,
            state,
            _scratch: scratch,
        })
    }

    /// What the package did since the last call.
    pub fn take_effects(&self) -> Effects {
        std::mem::take(&mut *lock(&self.state.effects))
    }

    pub fn storage(&self) -> Value {
        Value::Object(lock(&self.state.storage).clone())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Writes a files scope's fixture contents under `root`, or an empty file or folder when the
/// fixture has none, and returns the path to bind it to.
fn materialize(
    root: &Path,
    scope: &str,
    kind: FilesKind,
    reply: Option<&FilesReply>,
) -> std::io::Result<PathBuf> {
    let path = root.join(scope);
    match (kind, reply) {
        (FilesKind::File, reply) => {
            let text = match reply {
                Some(FilesReply::File(text)) => text.as_str(),
                _ => "",
            };
            std::fs::write(&path, text)?;
        }
        (FilesKind::Directory, reply) => {
            std::fs::create_dir_all(&path)?;
            if let Some(FilesReply::Directory(files)) = reply {
                for (relative, text) in files {
                    let file = path.join(relative);
                    if let Some(parent) = file.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(file, text)?;
                }
            }
        }
    }
    Ok(path)
}

struct Shared(Arc<State>);

fn missing(what: String) -> Failure {
    Failure::new("unavailable", format!("the fixture has no reply for {what}"))
}

impl services::Http for Shared {
    fn fetch(
        &self,
        request: http::Request,
        _scope: &http::HttpScope,
        _cancel: &http::CancelToken,
    ) -> Result<http::Response, http::HttpError> {
        let method = request.method.as_str();
        let reply = self
            .0
            .replies
            .http
            .iter()
            .find(|reply| {
                reply.request.method.eq_ignore_ascii_case(method) && reply.request.url == request.url
            })
            .ok_or_else(|| {
                http::HttpError::Network(format!("the fixture has no reply for {method} {}", request.url))
            })?;
        Ok(http::Response {
            status: reply.response.status,
            url: request.url,
            redirected: false,
            headers: reply
                .response
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            body: reply.response.body(),
        })
    }
}

impl services::Exec for Shared {
    fn run(
        &self,
        profile: &exec::Profile,
        values: &BTreeMap<String, exec::Value>,
        _cancel: &AtomicBool,
    ) -> Result<exec::Outcome, exec::ExecError> {
        let args = profile.bind(values)?;
        let names = match profile.target() {
            exec::Target::Script(script) => vec![profile.name().to_owned(), format!("@script:{script}")],
            exec::Target::Executable(_) => vec![profile.name().to_owned()],
        };
        let reply = self
            .0
            .replies
            .exec
            .iter()
            .find(|reply| {
                names.contains(&reply.profile) && reply.args.as_ref().is_none_or(|want| *want == args)
            })
            .ok_or_else(|| {
                exec::ExecError::Unavailable(format!(
                    "the fixture has no reply for running {}",
                    profile.name()
                ))
            })?;
        Ok(exec::Outcome {
            exit: exec::Exit::Code(reply.status),
            output: profile.options().return_output.then(|| exec::Output {
                stdout: reply.stdout.clone(),
                stderr: reply.stderr.clone(),
                truncated: false,
            }),
        })
    }
}

impl services::Storage for Shared {
    fn get(&self, _package: &str, key: &str) -> Result<Option<Value>, Failure> {
        Ok(lock(&self.0.storage).get(key).cloned())
    }

    fn set(
        &self,
        _package: &str,
        key: &str,
        value: &Value,
        _retention_days: Option<u32>,
    ) -> Result<(), Failure> {
        lock(&self.0.storage).insert(key.into(), value.clone());
        lock(&self.0.effects).storage_changed = true;
        Ok(())
    }

    fn delete(&self, _package: &str, key: &str) -> Result<(), Failure> {
        if lock(&self.0.storage).remove(key).is_some() {
            lock(&self.0.effects).storage_changed = true;
        }
        Ok(())
    }

    fn keys(&self, _package: &str) -> Result<Vec<String>, Failure> {
        Ok(lock(&self.0.storage).keys().cloned().collect())
    }

    fn size(&self, _package: &str) -> Result<u64, Failure> {
        Ok(lock(&self.0.storage)
            .iter()
            .map(|(key, value)| (key.len() + value.to_string().len()) as u64)
            .sum())
    }
}

impl services::Clipboard for Shared {
    fn read(&self) -> Result<Option<Value>, Failure> {
        match &self.0.replies.clipboard {
            Some(Value::Null) => Ok(None),
            Some(content) => Ok(Some(content.clone())),
            None => Err(missing("reading the clipboard".into())),
        }
    }

    fn write_text(&self, text: &str) -> Result<(), Failure> {
        lock(&self.0.effects).clipboard.push(text.into());
        Ok(())
    }

    /// Watched copies arrive as the fixture's `event` steps instead.
    fn watch(&self, _changed: Box<dyn Fn(Value) + Send + Sync>) -> Result<Subscription, Failure> {
        Ok(Box::new(()))
    }
}

impl services::Dictionary for Shared {
    fn lookup(&self, word: &str) -> Result<Value, Failure> {
        self.0
            .replies
            .dictionary
            .get(word)
            .map(|entries| Value::from(entries.clone()))
            .ok_or_else(|| missing(format!("looking up {}", Value::from(word))))
    }
}

impl services::Calendar for Shared {
    /// Whole days from the fixture's clock, counted in UTC.
    fn window(&self, from: i64, to: i64) -> Result<(u32, u32), Failure> {
        let today = self.0.now.div_euclid(DAY_MS) * DAY_MS;
        let days = |span: i64| (span.max(0) + DAY_MS - 1) / DAY_MS;
        let past = days(today - from);
        let future = days(to - (today + DAY_MS));
        Ok((
            past.min(u32::MAX as i64) as u32,
            future.min(u32::MAX as i64) as u32,
        ))
    }

    fn events(&self, from: i64, to: i64, limit: usize, _scope: &Scope) -> Result<Value, Failure> {
        let at = |event: &Value, key: &str| event.get(key).and_then(Value::as_i64);
        Ok(Value::from(
            self.0
                .replies
                .calendar
                .iter()
                .filter(|event| {
                    at(event, "start").is_some_and(|start| start < to)
                        && at(event, "end").is_some_and(|end| end > from)
                })
                .take(limit)
                .cloned()
                .collect::<Vec<_>>(),
        ))
    }
}
