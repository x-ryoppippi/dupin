//! A broker bridge for one fixture: the package installed at its revision with the fixture's
//! consent, behind host services that answer from the fixture's `replies` alone. Requests go
//! through the same decoding and broker checks as the launcher's; only the services differ, and
//! none of them reaches the network, a program, the clipboard, calendars or Dupin's storage.
//! A `files` scope is bound to a scratch folder holding the fixture's files, and `storage` to
//! a scratch store sealed with a throwaway key, seeded with the fixture's records.
//!
//! The headless launcher `dupin ext snap` starts answers from the same services, so the panel
//! it renders is the one `dupin ext test` replays.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use dupin_ext_sdk::fixture::{self, Consent, FilesReply, Replies};
use dupin_extension_bridge::services::{self, Services, Subscription};
use dupin_extension_bridge::storage::Namespace;
use dupin_extension_bridge::{
    Base, Bridge, Broker, Failure, FilesKind, Local, OsAccess, OsPermission, PackageId, PackageStorage,
    Revision, Scope,
};
use dupin_extension_http as http;
use dupin_host_services::{calendar, exec};
use dupin_js_runtime::Diagnostic;
use dupin_storage::{Limits, MemoryKey, NewRecord, Store};
use serde_json::Value;

use super::super::Extension;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Under the scratch folder; the dot keeps it apart from every `files` scope's name.
const STORAGE_DIR: &str = ".storage";
/// Bound to every executable profile; the fixture's exec service never runs it.
const UNRUN_PROGRAM: &str = "/usr/bin/false";

/// What the package did through the services during one step.
#[derive(Default)]
pub(super) struct Effects {
    pub clipboard: Vec<String>,
    /// The whole store, when the step changed it.
    pub storage: Option<Value>,
}

pub(super) struct Fixture {
    pub bridge: Bridge,
    replier: Replier,
}

/// The services a fixture's replies answer, and the scratch folder its `files` scopes and
/// storage live in.
struct Replier {
    state: Arc<State>,
    /// The `files` scopes' contents, removed when the fixture ends.
    scratch: tempfile::TempDir,
}

struct State {
    replies: Replies,
    now: i64,
    storage: PackageStorage,
    /// The package's namespace once a call has opened and seeded it.
    opened: Mutex<Option<Arc<Namespace>>>,
    /// The store as the last step left it.
    stored: Mutex<Vec<Value>>,
    effects: Mutex<Effects>,
}

impl Fixture {
    /// `name` is the fixture's path in the package, for diagnostics.
    pub fn open(extension: &Extension, name: &str, replies: &Replies, now: i64) -> Result<Self, Diagnostic> {
        let replier = Replier::new(replies, now)?;
        let bridge = Bridge::new(
            Broker::new(Base::default(), Local::default()),
            None,
            replier.services(),
        );
        bridge.install(
            &extension.package,
            extension.revision.clone(),
            extension.capabilities.clone(),
        );
        replier.grant(&bridge, &extension.package, &extension.revision, name)?;
        Ok(Self { bridge, replier })
    }

    /// What the package did since the last call.
    pub fn take_effects(&self) -> Effects {
        self.replier.take_effects()
    }
}

impl Replier {
    fn new(replies: &Replies, now: i64) -> Result<Self, Diagnostic> {
        let scratch = tempfile::Builder::new()
            .prefix("dupin-fixture-")
            .tempdir()
            .map_err(|error| {
                Diagnostic::new(
                    "fixtures-unreadable",
                    format!("couldn't make a scratch folder: {error}"),
                )
            })?;
        // A fixed key, so records stored under a key, as the fixture's own are, get the same IDs
        // on every run and a fixture can name them. The store is thrown away with the run.
        let key = dupin_storage::DataKey::from_slice(&[0x5a; 32]).expect("32 bytes make a key");
        let storage = dupin_storage::Storage::new(scratch.path().join(STORAGE_DIR), MemoryKey::new(key));
        let state = Arc::new(State {
            replies: replies.clone(),
            now,
            storage: PackageStorage::new(storage),
            opened: Mutex::default(),
            stored: Mutex::new(replies.storage.clone()),
            effects: Mutex::default(),
        });
        Ok(Self { state, scratch })
    }

    fn services(&self) -> Services {
        let shared = || Box::new(Shared(self.state.clone()));
        Services {
            http: shared(),
            exec: shared(),
            storage: shared(),
            clipboard: shared(),
            dictionary: shared(),
            calendar: shared(),
        }
    }

    /// Turns on package `id`, which `bridge` holds at `revision`, as macOS and the fixture's
    /// consent allow it. `name` is the fixture's path in the package, for diagnostics.
    fn grant(
        &self,
        bridge: &Bridge,
        id: &PackageId,
        revision: &Revision,
        name: &str,
    ) -> Result<(), Diagnostic> {
        let replies = &self.state.replies;
        let scratch = &self.scratch;
        let invalid = |message: String, field: &str| {
            Diagnostic::new("invalid-fixture", message)
                .file(name)
                .field(field.to_owned())
        };
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
                .allow(id, revision, &entry.scope, binding)
                .map_err(refused)?;
            if consent == Consent::Deny {
                bridge.revoke(id, &key).map_err(refused)?;
            }
        }
        Ok(())
    }

    fn take_effects(&self) -> Effects {
        let mut effects = std::mem::take(&mut *lock(&self.state.effects));
        let Some(namespace) = lock(&self.state.opened).clone() else {
            return effects;
        };
        let now = match namespace.with(records) {
            Ok(records) => records,
            Err(error) => vec![Value::from(format!("the store couldn't be read: {error}"))],
        };
        let mut stored = lock(&self.state.stored);
        if *stored != now {
            effects.storage = Some(Value::from(now.clone()));
            *stored = now;
        }
        effects
    }
}

/// Hands the headless launcher `dupin ext snap` starts the fixture it renders, as JSON:
/// `{ "fixture": <path>, "package": <package ID> }`.
pub(in crate::extensions) const HEADLESS_FIXTURE: &str = "DUPIN_HEADLESS_EXTENSION_FIXTURE";

#[derive(serde::Serialize, serde::Deserialize)]
pub(in crate::extensions) struct HeadlessFixture {
    pub fixture: PathBuf,
    pub package: String,
}

/// The fixture a headless launcher renders: its replies answer every host request, and its
/// consent is the replayed package's once the launcher installs it.
pub(in crate::extensions) struct Headless {
    package: PackageId,
    replier: Replier,
    /// The revision consent was last given for.
    granted: Mutex<Option<Revision>>,
}

/// The fixture `dupin ext snap` handed this headless launcher, if it handed one. A launcher
/// with a window never reads it.
pub(in crate::extensions) fn headless() -> Option<&'static Headless> {
    static HEADLESS: OnceLock<Option<Headless>> = OnceLock::new();
    if !rune::tooling::is_headless() {
        return None;
    }
    HEADLESS
        .get_or_init(|| {
            let handed: HeadlessFixture =
                serde_json::from_str(&std::env::var(HEADLESS_FIXTURE).ok()?).ok()?;
            Headless::open(handed)
                .inspect_err(|diagnostic| crate::log::warn(&format!("snap fixture: {diagnostic}")))
                .ok()
        })
        .as_ref()
}

impl Headless {
    pub fn open(handed: HeadlessFixture) -> Result<Self, Diagnostic> {
        let name = handed.fixture.display().to_string();
        let parsed = fixture::read(&name, &handed.fixture)?;
        let now = fixture::parse_time(&parsed.environment.now).expect("validated by `fixture::parse`");
        let package = PackageId::new(handed.package)
            .map_err(|error| Diagnostic::new("invalid-package", error.to_string()))?;
        Ok(Self {
            package,
            replier: Replier::new(&parsed.replies, now)?,
            granted: Mutex::default(),
        })
    }

    pub fn services(&self) -> Services {
        self.replier.services()
    }

    /// Gives `extension` the fixture's consent the first time `bridge` holds the replayed
    /// package at its revision.
    pub fn grant(&self, bridge: &Bridge, extension: &Extension) {
        if extension.package != self.package {
            return;
        }
        let mut granted = lock(&self.granted);
        if granted.as_ref() == Some(&extension.revision) {
            return;
        }
        let name = self.package.to_string();
        match self
            .replier
            .grant(bridge, &extension.package, &extension.revision, &name)
        {
            Ok(()) => *granted = Some(extension.revision.clone()),
            Err(diagnostic) => crate::log::warn(&format!("fixture for {name}: {diagnostic}")),
        }
    }
}

/// Each record's data, oldest first; blobs are left out.
fn records(store: &Store) -> Result<Vec<Value>, dupin_storage::Error> {
    let mut records = Vec::new();
    let mut before = None;
    loop {
        let page = store.list(before.as_ref(), usize::MAX)?;
        records.extend(page.records.into_iter().map(|record| record.data));
        match page.next {
            Some(next) => before = Some(next),
            None => break,
        }
    }
    records.reverse();
    Ok(records)
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
    /// Seeds the namespace with the fixture's records the first time a call opens it.
    fn open(&self, namespace: &str, limits: Limits) -> Result<Arc<Namespace>, Failure> {
        let mut opened = lock(&self.0.opened);
        let store = self.0.storage.open(namespace, limits)?;
        if opened.is_none() {
            let seeded = self
                .0
                .replies
                .storage
                .iter()
                .enumerate()
                .try_for_each(|(index, data)| {
                    store
                        .with(|store| {
                            store.upsert_within_limits(
                                &format!("fixture record {index}"),
                                NewRecord {
                                    data: data.clone(),
                                    blobs: Vec::new(),
                                },
                            )
                        })
                        .map(drop)
                });
            if let Err(failure) = seeded {
                self.0.storage.discard(namespace)?;
                return Err(failure);
            }
            *opened = Some(store.clone());
        }
        Ok(store)
    }

    fn discard(&self, namespace: &str) -> Result<(), Failure> {
        self.0.storage.discard(namespace)
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

    /// The fixture's events overlapping the range whose title or location holds every word of
    /// `query`, in `order`, as the calendar service matches them.
    fn events(
        &self,
        from: i64,
        to: i64,
        query: Option<&str>,
        limit: usize,
        order: calendar::Order,
        _scope: &Scope,
    ) -> Result<Value, Failure> {
        let at = |event: &Value, key: &str| event.get(key).and_then(Value::as_i64);
        let text = |event: &Value, key: &str| {
            event
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase()
        };
        let query = query.unwrap_or_default().to_lowercase();
        let terms: Vec<&str> = query.split_whitespace().collect();
        let mut events: Vec<&Value> = self
            .0
            .replies
            .calendar
            .iter()
            .filter(|event| {
                at(event, "start").is_some_and(|start| start < to)
                    && at(event, "end").is_some_and(|end| end > from)
            })
            .filter(|event| {
                let (title, location) = (text(event, "title"), text(event, "location"));
                terms
                    .iter()
                    .all(|term| title.contains(term) || location.contains(term))
            })
            .collect();
        events.sort_by_key(|event| at(event, "start"));
        if order == calendar::Order::Latest {
            events.reverse();
        }
        Ok(Value::from(
            events.into_iter().take(limit).cloned().collect::<Vec<_>>(),
        ))
    }
}
