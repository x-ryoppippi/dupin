//! A broker bridge for one fixture: the package installed at its revision with the fixture's
//! consent, behind host services that answer from the fixture's `replies` alone. Requests go
//! through the same decoding and broker checks as the launcher's; only the services differ, and
//! none of them reaches the network, a program, the clipboard, calendars, windows or Dupin's
//! storage.
//! A `files` scope is bound to a scratch folder holding the fixture's files, and `storage` to
//! a scratch store sealed with a throwaway key, seeded with the fixture's records.
//!
//! The headless launcher `dupin ext render` starts answers from the same services, so the panel
//! it renders is the one `dupin ext test` replays.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use dupin_ext_sdk::fixture::{self, Consent, FilesReply, Replies};
use dupin_extension_bridge::services::{self, Services, Subscription};
use dupin_extension_bridge::storage::Namespace;
use dupin_extension_bridge::{
    Base, Bridge, Broker, Failure, FilesKind, Local, OsAccess, OsPermission, PackageId, PackageStorage,
    Revision, Scope,
};
use dupin_extension_http as http;
use dupin_host_services::{calendar, context, exec, windows};
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
    /// Each frame the focused window was moved to.
    pub windows: Vec<windows::Frame>,
    /// The whole store, when the step changed it.
    pub storage: Option<Value>,
}

pub(super) struct Fixture {
    pub bridge: Bridge,
    replier: Replier,
}

/// The fixture's clock, in milliseconds since the epoch, which never moves.
struct FixtureClock(i64);

impl dupin_storage::Clock for FixtureClock {
    fn now(&self) -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_millis(self.0.max(0) as u64)
    }
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
    windows: Option<windows::Service<windows::Fixture>>,
    context: Option<context::Service<context::Fixture>>,
    /// The snippets the fixture hands the package, which its `snippets` calls change.
    snippets: Mutex<Snippets>,
    /// What watches the clipboard, by subscription, which a headless launcher hands the
    /// fixture's copies.
    watchers: Mutex<Vec<(u64, Watcher)>>,
    subscriptions: AtomicU64,
}

type Watcher = Arc<dyn Fn(Value) + Send + Sync>;

impl Fixture {
    /// `name` is the fixture's path in the package, for diagnostics.
    pub fn open(
        extension: &Extension,
        name: &str,
        replies: &Replies,
        preferences: &serde_json::Map<String, Value>,
        now: i64,
    ) -> Result<Self, Diagnostic> {
        let replier = Replier::new(replies, preferences, now)?;
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

    /// The `snippets` and `paste` preferences, when a `snippets` call changed the snippets
    /// since the last call, as Dupin hands them to the bundled package after a change.
    pub fn take_snippets(&self) -> Option<serde_json::Map<String, Value>> {
        self.replier.take_snippets()
    }
}

/// The fixture's snippets: `ctx.preferences.snippets` split by where each comes from, which a
/// `snippets` call checks and changes as Dupin's own library does, without storage.
#[derive(Default)]
struct Snippets {
    declared: BTreeMap<String, dupin_core::config::Snippet>,
    stored: Vec<dupin_core::snippets::Snippet>,
    paste: bool,
    changed: bool,
}

impl Snippets {
    fn seeded(preferences: &serde_json::Map<String, Value>) -> Self {
        let mut snippets = Self {
            paste: preferences
                .get("paste")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            ..Self::default()
        };
        let listed = preferences.get("snippets").and_then(Value::as_array);
        for entry in listed.into_iter().flatten() {
            let Ok(snippet) = serde_json::from_value::<dupin_core::snippets::Snippet>(entry.clone()) else {
                continue;
            };
            if entry.get("source").and_then(Value::as_str) == Some("config") {
                snippets.declared.insert(
                    snippet.keyword,
                    dupin_core::config::Snippet {
                        name: snippet.name,
                        text: snippet.text,
                    },
                );
            } else {
                snippets.stored.push(snippet);
            }
        }
        snippets
    }
}

impl Replier {
    fn new(
        replies: &Replies,
        preferences: &serde_json::Map<String, Value>,
        now: i64,
    ) -> Result<Self, Diagnostic> {
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
        // Records are stamped with the fixture's clock, so the times a package shows of them are the
        // same on every run.
        let storage = dupin_storage::Storage::new(scratch.path().join(STORAGE_DIR), MemoryKey::new(key))
            .with_clock(FixtureClock(now));
        let windows = replies.windows.as_ref().map(window_fixture).transpose()?;
        let context = replies.context.as_ref().map(context_fixture).transpose()?;
        let state = Arc::new(State {
            replies: replies.clone(),
            now,
            storage: PackageStorage::new(storage),
            opened: Mutex::default(),
            stored: Mutex::new(replies.storage.clone()),
            effects: Mutex::default(),
            windows,
            context,
            snippets: Mutex::new(Snippets::seeded(preferences)),
            watchers: Mutex::default(),
            subscriptions: AtomicU64::default(),
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
            calculator: shared(),
            snippets: shared(),
            calendar: shared(),
            windows: shared(),
            context: shared(),
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

    fn take_snippets(&self) -> Option<serde_json::Map<String, Value>> {
        let mut snippets = lock(&self.state.snippets);
        std::mem::take(&mut snippets.changed).then(|| {
            dupin_core::snippets::preferences(&snippets.declared, &snippets.stored, snippets.paste).0
        })
    }

    fn take_effects(&self) -> Effects {
        let mut effects = std::mem::take(&mut *lock(&self.state.effects));
        if let Some(windows) = &self.state.windows {
            effects.windows = std::mem::take(&mut *lock(&windows.backend().moves));
        }
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

/// Hands the headless launcher `dupin ext render` starts the fixture it renders, as JSON:
/// `{ "fixture": <path>, "name": <package name>, "revision": <revision> }`.
pub(in crate::extensions) const HEADLESS_FIXTURE: &str = "DUPIN_HEADLESS_EXTENSION_FIXTURE";

#[derive(serde::Serialize, serde::Deserialize)]
pub(in crate::extensions) struct HeadlessFixture {
    pub fixture: PathBuf,
    /// The `name` in the package's package.json.
    pub name: String,
    pub revision: String,
    /// Each clipboard `event` step, by its index among the steps, and the seconds into the
    /// script it comes at.
    pub events: Vec<(usize, f64)>,
}

/// What a headless run that nobody handed a fixture finds in its extensions folder, such as a
/// latency scenario's: `{ "name": <bundled package name>, "fixture": <file in that folder> }`.
/// A file whose name starts with a dot is no extension, so the folder loads nothing else.
const PLACED_FIXTURE: &str = ".headless-fixture.json";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Placed {
    name: String,
    fixture: PathBuf,
}

/// The fixture placed in the extensions folder, for the bundled package it names at the
/// revision this build ships.
fn placed() -> Option<HeadlessFixture> {
    let folder = super::super::directory()?;
    let placed: Placed = serde_json::from_str(&std::fs::read_to_string(folder.join(PLACED_FIXTURE)).ok()?)
        .inspect_err(|error| crate::log::warn(&format!("{PLACED_FIXTURE}: {error}")))
        .ok()?;
    let package = super::super::bundled::find(&super::super::bundled::directory()?, &placed.name)?;
    let (_, revision, _) = super::super::declaration_of(&package).ok()?;
    Some(HeadlessFixture {
        fixture: folder.join(placed.fixture),
        name: placed.name,
        revision: revision.as_str().to_owned(),
        events: Vec::new(),
    })
}

/// The fixture a headless launcher renders: its replies answer every host request, and its
/// consent and preferences are the replayed package's once the launcher installs it.
pub(in crate::extensions) struct Headless {
    name: String,
    revision: Revision,
    replier: Replier,
    /// Whether the fixture's consent was given.
    granted: Mutex<bool>,
    /// The fixture's preferences, with the snippets its `snippets` calls changed: the same
    /// value until they change again.
    preferences: Mutex<Arc<dupin_js_runtime::Preferences>>,
    /// The copies the clipboard `event` steps make, and the seconds into the script each
    /// comes at.
    events: Vec<(f64, Value)>,
    played: AtomicBool,
}

/// The fixture `dupin ext render` handed this headless launcher, if it handed one. A launcher
/// with a window never reads it.
pub(in crate::extensions) fn headless() -> Option<&'static Headless> {
    static HEADLESS: OnceLock<Option<Headless>> = OnceLock::new();
    if !rune::tooling::is_headless() {
        return None;
    }
    HEADLESS
        .get_or_init(|| {
            let handed: HeadlessFixture = match std::env::var(HEADLESS_FIXTURE) {
                Ok(handed) => serde_json::from_str(&handed).ok()?,
                Err(_) => placed()?,
            };
            Headless::open(handed)
                .inspect_err(|diagnostic| crate::log::warn(&format!("render fixture: {diagnostic}")))
                .ok()
        })
        .as_ref()
}

impl Headless {
    pub fn open(handed: HeadlessFixture) -> Result<Self, Diagnostic> {
        let name = handed.fixture.display().to_string();
        let parsed = fixture::read(&name, &handed.fixture)?;
        let now = fixture::parse_time(&parsed.environment.now).expect("validated by `fixture::parse`");
        let revision =
            Revision::new(handed.revision).map_err(|error| Diagnostic::new("invalid-package", error))?;
        Ok(Self {
            name: handed.name,
            revision,
            replier: Replier::new(&parsed.replies, &parsed.preferences, now)?,
            granted: Mutex::default(),
            preferences: Mutex::new(Arc::new(parsed.preferences.clone())),
            events: handed
                .events
                .iter()
                .filter_map(|(index, at)| match parsed.steps.get(*index)?.action() {
                    fixture::Action::Event(CLIPBOARD_EVENT, data) => Some((*at, data.clone())),
                    _ => None,
                })
                .collect(),
            played: AtomicBool::new(false),
        })
    }

    pub fn services(&self) -> Services {
        self.replier.services()
    }

    /// Whether `extension` is the replayed package: its name at its revision. A copy byte for
    /// byte the shipped package is the bundled package, so it is that one.
    pub fn renders(&self, extension: &Extension) -> bool {
        extension.id == self.name && extension.revision == self.revision
    }

    /// Gives the replayed package the fixture's consent, and nothing more, the first time
    /// `bridge` holds it.
    pub fn grant(&self, bridge: &Bridge, extension: &Extension) {
        if !self.renders(extension) {
            return;
        }
        let mut granted = lock(&self.granted);
        if *granted {
            return;
        }
        match self
            .replier
            .grant(bridge, &extension.package, &extension.revision, &self.name)
        {
            Ok(()) => *granted = true,
            Err(diagnostic) => crate::log::warn(&format!("fixture for {}: {diagnostic}", self.name)),
        }
    }

    /// What the replayed package is handed as `ctx.preferences` beside its declared ones, in
    /// place of its feature's settings, as `dupin ext test` hands it the fixture's.
    pub fn preferences(&self, extension: &Extension) -> Option<Arc<dupin_js_runtime::Preferences>> {
        if !self.renders(extension) {
            return None;
        }
        let mut held = lock(&self.preferences);
        if let Some(snippets) = self.replier.take_snippets() {
            let mut preferences = (**held).clone();
            preferences.extend(snippets);
            *held = Arc::new(preferences);
        }
        Some(held.clone())
    }

    /// Hands what watches the clipboard each copy the fixture's clipboard `event` steps make,
    /// at its time in the script, counted from now: the first frame, where the script starts.
    pub fn play_events(&'static self) {
        if self.events.is_empty() || self.played.swap(true, Ordering::Relaxed) {
            return;
        }
        let start = Instant::now();
        std::thread::spawn(move || {
            for (at, copy) in &self.events {
                let due = start + Duration::from_secs_f64(*at);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
                let watchers: Vec<Watcher> = lock(&self.replier.state.watchers)
                    .iter()
                    .map(|(_, watcher)| watcher.clone())
                    .collect();
                for watcher in watchers {
                    watcher(copy.clone());
                }
            }
        });
    }
}

/// The event a fixture names for a copy the package watches the clipboard for.
pub(in crate::extensions) const CLIPBOARD_EVENT: &str = "clipboard";

/// The windows service a fixture's `replies.windows` describes, read as the service's own shapes.
fn window_fixture(reply: &fixture::WindowsReply) -> Result<windows::Service<windows::Fixture>, Diagnostic> {
    let invalid = |field: String, error: serde_json::Error| {
        Diagnostic::new("invalid-fixture", error.to_string()).field(field)
    };
    let displays = reply
        .displays
        .iter()
        .enumerate()
        .map(|(index, display)| {
            serde_json::from_value(display.clone())
                .map_err(|error| invalid(format!("replies.windows.displays[{index}]"), error))
        })
        .collect::<Result<_, _>>()?;
    let focused = serde_json::from_value(reply.focused.clone())
        .map_err(|error| invalid("replies.windows.focused".into(), error))?;
    Ok(windows::Service::new(windows::Fixture::new(displays, focused)))
}

/// The context service a fixture's `replies.context` describes, read as the service's own
/// shapes.
fn context_fixture(reply: &fixture::ContextReply) -> Result<context::Service<context::Fixture>, Diagnostic> {
    let invalid = |field: &str, error: serde_json::Error| {
        Diagnostic::new("invalid-fixture", error.to_string()).field(format!("replies.context.{field}"))
    };
    Ok(context::Service::new(context::Fixture {
        frontmost: serde_json::from_value(reply.frontmost.clone())
            .map_err(|error| invalid("frontmost", error))?,
        document: serde_json::from_value(reply.document.clone())
            .map_err(|error| invalid("document", error))?,
    }))
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
    fn open(&self, namespace: &str, limits: Limits, synced: Option<&str>) -> Result<Arc<Namespace>, Failure> {
        let mut opened = lock(&self.0.opened);
        let store = self.0.storage.open(namespace, limits, synced)?;
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

    /// The fixture's `event` steps are the copies watched: a replay hands them to the package
    /// itself, and a headless launcher to what watches here.
    fn watch(&self, changed: Box<dyn Fn(Value) + Send + Sync>) -> Result<Subscription, Failure> {
        let id = self.0.subscriptions.fetch_add(1, Ordering::Relaxed);
        lock(&self.0.watchers).push((id, Arc::from(changed)));
        Ok(Box::new(Watching {
            state: Arc::downgrade(&self.0),
            id,
        }))
    }
}

/// Keeps a clipboard watch while held.
struct Watching {
    state: Weak<State>,
    id: u64,
}

impl Drop for Watching {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            lock(&state.watchers).retain(|(id, _)| *id != self.id);
        }
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

impl services::Calculator for Shared {
    fn evaluate(&self, expression: &str, _cancelled: &AtomicBool) -> Result<Value, Failure> {
        self.0
            .replies
            .calculator
            .get(expression)
            .cloned()
            .ok_or_else(|| missing(format!("working out {}", Value::from(expression))))
    }
}

impl services::Snippets for Shared {
    fn save(&self, draft: services::SnippetDraft, replaces: Option<&str>) -> Result<Value, Failure> {
        let draft = dupin_core::snippets::Snippet {
            keyword: draft.keyword,
            name: draft.name,
            text: draft.text,
        }
        .normalized();
        let mut snippets = lock(&self.0.snippets);
        let problems =
            dupin_core::snippets::check_save(&snippets.declared, &snippets.stored, &draft, replaces);
        if problems.is_empty() {
            snippets.stored.retain(|snippet| {
                Some(snippet.keyword.as_str()) != replaces && snippet.keyword != draft.keyword
            });
            snippets.stored.push(draft);
            snippets.stored.sort_by(|a, b| a.keyword.cmp(&b.keyword));
            snippets.changed = true;
        }
        serde_json::to_value(problems).map_err(|error| Failure::new("failed", error.to_string()))
    }

    fn delete(&self, keyword: &str) -> Result<bool, Failure> {
        let mut snippets = lock(&self.0.snippets);
        let before = snippets.stored.len();
        snippets.stored.retain(|snippet| snippet.keyword != keyword);
        let deleted = snippets.stored.len() != before;
        snippets.changed |= deleted;
        Ok(deleted)
    }
}

impl Shared {
    fn windows(&self) -> Result<&windows::Service<windows::Fixture>, Failure> {
        self.0
            .windows
            .as_ref()
            .ok_or_else(|| missing("reading or moving windows".into()))
    }

    fn context(&self) -> Result<&context::Service<context::Fixture>, Failure> {
        self.0
            .context
            .as_ref()
            .ok_or_else(|| missing("reading what you were using".into()))
    }
}

fn encoded(value: impl serde::Serialize) -> Result<Value, Failure> {
    serde_json::to_value(value).map_err(|error| Failure::new("failed", error.to_string()))
}

impl services::Windows for Shared {
    fn displays(&self) -> Result<Value, Failure> {
        encoded(self.windows()?.displays()?)
    }

    fn focused(&self) -> Result<Value, Failure> {
        encoded(self.windows()?.focused()?)
    }

    fn set_frame(&self, frame: windows::Frame) -> Result<Value, Failure> {
        encoded(self.windows()?.set_frame(frame)?)
    }
}

impl services::Context for Shared {
    fn frontmost(&self) -> Result<Value, Failure> {
        encoded(self.context()?.frontmost())
    }

    fn document(&self) -> Result<Value, Failure> {
        encoded(self.context()?.document())
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
