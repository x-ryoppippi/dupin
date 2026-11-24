//! The broker bridge: every host request guest JavaScript makes reaches Dupin's host services
//! only through here. The extension host carries a request from the sandboxed helper; a
//! [`Session`] the app made for that one call decodes it, asks the [`Broker`] whether the
//! installed package, at its revision and generation and under the trigger the app saw, may
//! perform it, runs the service and checks the permit again before releasing the result.
//!
//! The bridge owns the one grant store: the read-only declarative base, which Home Manager and
//! the installer write, and the user's local grants, which it saves after every decision. It
//! never prompts. A request that needs consent fails with `needs-consent` and is kept as a
//! [`ConsentRequest`] for the panel and Settings to show; granting it runs nothing again.

mod call;
pub mod services;
mod watch;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

pub use dupin_broker::{
    Base, Broker, BrokerError, Capabilities, ClipboardAccess, ConsentReason, ConsentState, Denial, FilesKind,
    Generation, GrantSource, Local, OsAccess, OsConsumer, OsPermission, PackageId, Reconciliation, Revision,
    Scope, ScopeKey, StoreError, Trigger,
};
use dupin_extension_host::Services as HostServices;
use dupin_host_services::files;
pub use services::{Failure, Services};

/// The largest reply a guest receives, encoded; its heap is 16 MiB.
const MAX_REPLY_BYTES: usize = 4 * 1024 * 1024;
/// Consent requests kept for the panel and Settings, newest first.
const MAX_PENDING: usize = 32;

/// A request for access the bridge refused for want of consent, as the panel and Settings show
/// it until the user answers or the package changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsentRequest {
    pub package: PackageId,
    /// The revision and declared scope to pass back to [`Bridge::allow`].
    pub revision: Revision,
    pub scope: Scope,
    pub reason: ConsentReason,
    /// What granting allows, in a sentence that starts lowercase: "read the clipboard".
    pub description: String,
    /// The macOS access the scope also needs.
    pub permission: Option<OsPermission>,
}

impl ConsentRequest {
    pub fn key(&self) -> ScopeKey {
        self.scope.key()
    }

    /// Whether granting needs a file, folder or program the user picks first.
    pub fn needs_binding(&self) -> bool {
        self.scope.needs_binding()
    }
}

/// One declared scope of a package, for Settings → Extensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeEntry {
    pub scope: Scope,
    pub state: ConsentState,
    pub description: String,
    pub permission: Option<OsPermission>,
}

/// What changed, so the app can refresh what it shows. Never a prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// A call was refused for want of consent; [`Bridge::pending`] lists it.
    ConsentNeeded(PackageId),
    /// Grants, enablement or the generation of the package changed.
    Changed(PackageId),
}

type Listener = Arc<dyn Fn(Notice) + Send + Sync>;

/// One call's view of the bridge: the package, the revision and generation it started at, and
/// why it runs. The extension host hands it every request the call makes.
pub struct Session {
    bridge: Arc<Inner>,
    package: PackageId,
    revision: Revision,
    generation: Generation,
    trigger: Trigger,
}

impl HostServices for Session {
    fn call(&self, op: &str, args: &[u8], deadline: Instant, cancelled: &AtomicBool) -> Vec<u8> {
        let reply = match self.dispatch(op, args, deadline, cancelled) {
            Ok(value) => serde_json::json!({ "ok": value }),
            Err(failure) => serde_json::json!({ "error": failure }),
        };
        let bytes = reply.to_string().into_bytes();
        if bytes.len() <= MAX_REPLY_BYTES {
            return bytes;
        }
        let failure = Failure::new(
            "too-large",
            format!("the result is larger than {} MiB", MAX_REPLY_BYTES / 1024 / 1024),
        );
        serde_json::json!({ "error": failure }).to_string().into_bytes()
    }
}

/// Host services for one call, and the generation it started at: a result the call produces
/// is shown only while [`Bridge::is_current`] says that generation still holds.
pub struct Call {
    pub services: Arc<dyn HostServices>,
    pub generation: Option<Generation>,
}

#[derive(Clone)]
pub struct Bridge {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    services: Services,
    /// The local grant file decisions are saved to, if there is one.
    local: Option<PathBuf>,
    files: Mutex<HashMap<PackageId, FilesTable>>,
    watches: watch::Watches,
    listener: Mutex<Option<Listener>>,
}

type FilesTable = Arc<Mutex<Option<(Generation, files::Files)>>>;

struct State {
    broker: Broker,
    pending: Vec<ConsentRequest>,
    /// Packages whose request failed because they are off, so the panel can offer to turn
    /// them on.
    asked_while_off: HashSet<PackageId>,
    /// What the host last reported for each OS permission.
    os: HashMap<OsPermission, OsAccess>,
    /// Problems reading or saving grant files, for Settings.
    problems: Vec<String>,
}

impl Bridge {
    pub fn new(broker: Broker, local: Option<PathBuf>, services: Services) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    broker,
                    pending: Vec::new(),
                    asked_while_off: HashSet::new(),
                    os: HashMap::new(),
                    problems: Vec::new(),
                }),
                services,
                local,
                files: Mutex::default(),
                watches: watch::Watches::default(),
                listener: Mutex::default(),
            }),
        }
    }

    /// Reads the declarative base at `base` and the local grants at `local`. A missing file is
    /// empty. An unreadable local file grants nothing and is never overwritten; an unreadable
    /// base is reported and grants nothing.
    pub fn open(base: &Path, local: &Path, services: Services) -> Self {
        let mut problems = Vec::new();
        let base = match std::fs::read_to_string(base) {
            Ok(source) => Base::parse(&source).unwrap_or_else(|error| {
                problems.push(format!("{}: {error}", base.display()));
                Base::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Base::default(),
            Err(error) => {
                problems.push(format!("{}: {error}", base.display()));
                Base::default()
            }
        };
        let local_grants = match std::fs::read_to_string(local) {
            Ok(source) => Local::parse(&source).unwrap_or_else(|error| {
                problems.push(format!("{}: {error}", local.display()));
                Local::unreadable()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Local::default(),
            Err(error) => {
                problems.push(format!("{}: {error}", local.display()));
                Local::unreadable()
            }
        };
        let bridge = Self::new(Broker::new(base, local_grants), Some(local.to_owned()), services);
        bridge.inner.lock().problems = problems;
        bridge
    }

    /// Calls `listener` whenever grants change or a call needs consent, from whichever thread
    /// made the change.
    pub fn set_listener(&self, listener: impl Fn(Notice) + Send + Sync + 'static) {
        *self
            .inner
            .listener
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(listener));
    }

    /// Loads or updates a package's declaration at `revision`. It never grants anything; what
    /// a new revision widens needs consent again, and what it no longer declares is dropped.
    pub fn install(&self, id: &PackageId, revision: Revision, capabilities: Capabilities) -> Reconciliation {
        let mut state = self.inner.lock();
        let before = state.broker.generation(id);
        let report = state.broker.install(id.clone(), revision, capabilities);
        let changed = before != state.broker.generation(id);
        if changed {
            state.pending.retain(|request| request.package != *id);
        }
        if !report.dropped.is_empty() || !report.changed.is_empty() || !report.kept.is_empty() {
            self.inner.save(&mut state);
        }
        drop(state);
        if changed {
            self.inner.forget_files(id);
            self.inner.notify(Notice::Changed(id.clone()));
        }
        report
    }

    /// Host services for one call of `id`, as `trigger` describes why it runs. The app sets the
    /// trigger from what it saw: typing, a chosen action or a watched event.
    pub fn call(&self, id: &PackageId, trigger: Trigger) -> Call {
        let state = self.inner.lock();
        let (Some(revision), Some(generation)) = (state.broker.revision(id), state.broker.generation(id))
        else {
            return Call {
                services: Arc::new(dupin_extension_host::NoServices),
                generation: None,
            };
        };
        let session = Session {
            bridge: self.inner.clone(),
            package: id.clone(),
            revision: revision.clone(),
            generation,
            trigger,
        };
        Call {
            services: Arc::new(session),
            generation: Some(generation),
        }
    }

    /// Whether work stamped with `generation` may still be shown for `id`.
    pub fn is_current(&self, id: &PackageId, generation: Option<Generation>) -> bool {
        generation.is_none() || self.inner.lock().broker.generation(id) == generation
    }

    pub fn is_enabled(&self, id: &PackageId) -> bool {
        self.inner.lock().broker.is_enabled(id)
    }

    /// Turns a package's capabilities on or off. Turning off drops its in-flight results.
    pub fn set_enabled(&self, id: &PackageId, enabled: bool) -> Result<(), BrokerError> {
        self.change(id, |broker| broker.set_enabled(id, enabled))?;
        self.inner.lock().asked_while_off.remove(id);
        Ok(())
    }

    /// Whether a request of `id` failed because it is off since it was last turned on or off.
    pub fn asked_while_off(&self, id: &PackageId) -> bool {
        self.inner.lock().asked_while_off.contains(id)
    }

    /// Grants `scope` exactly as the user saw it for `revision`, with the path they picked
    /// for a files or exec scope. Turning the scope on also turns the package on, since the
    /// user chose to let it act. Nothing that failed for want of it runs again.
    pub fn allow(
        &self,
        id: &PackageId,
        revision: &Revision,
        scope: &Scope,
        binding: Option<PathBuf>,
    ) -> Result<(), BrokerError> {
        let key = scope.key();
        self.change(id, |broker| {
            broker.consent(id, revision, scope, binding)?;
            if !broker.is_enabled(id) {
                broker.set_enabled(id, true)?;
            }
            Ok(())
        })?;
        let mut state = self.inner.lock();
        state.asked_while_off.remove(id);
        state
            .pending
            .retain(|request| request.package != *id || request.key() != key);
        Ok(())
    }

    /// Takes `key` away at once, whoever granted it. In-flight results of the package are
    /// dropped, and the scope stays off until the user grants it again.
    pub fn revoke(&self, id: &PackageId, key: &ScopeKey) -> Result<(), BrokerError> {
        self.change(id, |broker| broker.revoke(id, key))?;
        self.inner
            .lock()
            .pending
            .retain(|request| request.package != *id || request.key() != *key);
        Ok(())
    }

    /// Reports what macOS allows, read without prompting.
    pub fn set_os_access(&self, permission: OsPermission, access: OsAccess) {
        let changed: Vec<_> = {
            let mut state = self.inner.lock();
            let before: Vec<_> = state
                .broker
                .os_consumers()
                .into_iter()
                .filter(|consumer| consumer.permission == permission)
                .map(|consumer| {
                    (
                        consumer.package.clone(),
                        state.broker.generation(&consumer.package),
                    )
                })
                .collect();
            state.broker.set_os_access(permission, access);
            state.os.insert(permission, access);
            before
                .into_iter()
                .filter(|(id, generation)| state.broker.generation(id) != *generation)
                .map(|(id, _)| id)
                .collect()
        };
        for id in changed {
            self.inner.forget_files(&id);
            self.inner.notify(Notice::Changed(id));
        }
        self.inner.refresh_watches();
    }

    /// What the host last reported macOS allows for `permission`.
    pub fn os_access(&self, permission: OsPermission) -> OsAccess {
        self.inner
            .lock()
            .os
            .get(&permission)
            .copied()
            .unwrap_or(OsAccess::NotDetermined)
    }

    /// Consent requests waiting for an answer, newest first: all of them, or `id`'s.
    pub fn pending(&self, id: Option<&PackageId>) -> Vec<ConsentRequest> {
        self.inner
            .lock()
            .pending
            .iter()
            .filter(|request| id.is_none_or(|id| request.package == *id))
            .cloned()
            .collect()
    }

    /// Every scope `id` declares and where it stands.
    pub fn scopes(&self, id: &PackageId) -> Vec<ScopeEntry> {
        self.inner
            .lock()
            .broker
            .consents(id)
            .into_iter()
            .map(|(scope, state)| ScopeEntry {
                description: scope.to_string(),
                permission: scope.os_permission(),
                scope,
                state,
            })
            .collect()
    }

    /// The installed revision of `id`, which consent shown from Settings passes back.
    pub fn revision(&self, id: &PackageId) -> Option<Revision> {
        self.inner.lock().broker.revision(id).cloned()
    }

    pub fn os_consumers(&self) -> Vec<OsConsumer> {
        self.inner.lock().broker.os_consumers()
    }

    /// Problems reading or saving the grant files.
    pub fn problems(&self) -> Vec<String> {
        self.inner.lock().problems.clone()
    }

    /// Delivers clipboard changes to `id`'s `events.clipboard` through `deliver` while the
    /// package is on and holds `clipboard:watch`. Changes that arrive while one is being
    /// delivered are coalesced to the newest.
    pub fn watch_clipboard(
        &self,
        id: &PackageId,
        deliver: impl Fn(serde_json::Value) + Send + Sync + 'static,
    ) {
        self.inner.watches.register(id.clone(), Arc::new(deliver));
        self.inner.refresh_watches();
    }

    /// Stops delivering clipboard changes to `id`.
    pub fn unwatch_clipboard(&self, id: &PackageId) {
        self.inner.watches.unregister(id);
    }

    fn change(
        &self,
        id: &PackageId,
        change: impl FnOnce(&mut Broker) -> Result<(), BrokerError>,
    ) -> Result<(), BrokerError> {
        let mut state = self.inner.lock();
        let before = state.broker.generation(id);
        change(&mut state.broker)?;
        let bumped = state.broker.generation(id) != before;
        self.inner.save(&mut state);
        drop(state);
        if bumped {
            self.inner.forget_files(id);
        }
        self.inner.notify(Notice::Changed(id.clone()));
        self.inner.refresh_watches();
        Ok(())
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn notify(&self, notice: Notice) {
        let listener = self
            .listener
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(listener) = listener {
            listener(notice);
        }
    }

    fn save(&self, state: &mut State) {
        let Some(path) = &self.local else { return };
        if let Err(error) = state.broker.save(path) {
            let problem = format!("{}: {error}", path.display());
            if !state.problems.contains(&problem) {
                state.problems.push(problem);
            }
        }
    }

    fn files(&self, id: &PackageId) -> FilesTable {
        self.files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(id.clone())
            .or_default()
            .clone()
    }

    fn forget_files(&self, id: &PackageId) {
        self.files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
    }

    /// Keeps the refusal for the panel and Settings and describes it for the guest.
    fn needs_consent(
        &self,
        state: &mut State,
        id: &PackageId,
        needed: dupin_broker::ConsentNeeded,
    ) -> Failure {
        let request = ConsentRequest {
            package: id.clone(),
            revision: needed.revision,
            description: needed.scope.to_string(),
            permission: match needed.reason {
                ConsentReason::OsAccess(permission) => Some(permission),
                _ => needed.scope.os_permission(),
            },
            reason: needed.reason,
            scope: needed.scope,
        };
        let key = request.key();
        let mut failure = Failure::new(
            "needs-consent",
            match request.reason {
                ConsentReason::OsAccess(permission) => format!(
                    "Dupin needs macOS {} access before the extension can {}.",
                    permission.name(),
                    request.description
                ),
                ConsentReason::Changed => format!(
                    "The extension changed what it asks for; allow it to {} again.",
                    request.description
                ),
                ConsentReason::NotGranted => format!("Allow the extension to {} first.", request.description),
            },
        )
        .scope(key.to_string());
        failure.description = Some(request.description.clone());
        failure.permission = request.permission.map(|permission| permission.name().into());
        state
            .pending
            .retain(|pending| pending.package != *id || pending.key() != key);
        state.pending.insert(0, request);
        state.pending.truncate(MAX_PENDING);
        failure
    }
}

/// What a guest learns of a denial.
pub(crate) fn failure_for(denial: &Denial) -> Failure {
    let code = match denial {
        Denial::NotInstalled => "not-installed",
        Denial::StaleRevision => "stale",
        Denial::GrantsUnreadable => "unavailable",
        Denial::Disabled => "disabled",
        Denial::Invalid(_) => "invalid",
        Denial::Undeclared(_) => "undeclared",
        Denial::OutOfScope {
            key: ScopeKey::Storage,
            ..
        } => "quota-exceeded",
        Denial::OutOfScope { .. } => "out-of-scope",
        Denial::NeedsActivation => "needs-activation",
        Denial::Revoked(_) => "revoked",
        Denial::DeniedByBase(_) => "denied",
        Denial::OsDenied(_) | Denial::OsRestricted(_) => "os-denied",
    };
    let failure = Failure::new(code, denial.to_string());
    match denial {
        Denial::Undeclared(key)
        | Denial::OutOfScope { key, .. }
        | Denial::Revoked(key)
        | Denial::DeniedByBase(key) => failure.scope(key.to_string()),
        Denial::OsDenied(permission) | Denial::OsRestricted(permission) => Failure {
            permission: Some(permission.name().into()),
            ..failure
        },
        _ => failure,
    }
}

/// Every placeholder an exec profile's template names, for Settings.
pub fn exec_inputs(scope: &Scope) -> Vec<String> {
    match scope {
        Scope::Exec { args, .. } => args.iter().filter_map(|arg| call::placeholder(arg)).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests;
