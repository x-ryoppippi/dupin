//! The broker bridge: every host request guest JavaScript makes reaches Dupin's host services
//! only through here. The extension host carries a request from the sandboxed helper; a
//! [`Session`] the app made for that one call decodes it, asks the [`Broker`] whether the
//! installed package, at its revision and generation and under the trigger the app saw, may
//! perform it, runs the service and checks the permit again before releasing the result.
//!
//! The bridge owns the one grant store: the read-only declarative base, a file Home Manager
//! writes, which it reads again whenever it changed before deciding, and the user's local
//! grants, a [`LocalStore`] it alone writes while it runs, saving after every decision. Another
//! process that must forget a package's grants asks the app to. Until the local grants are read
//! nothing is granted. It never prompts. A request that needs consent fails with
//! `needs-consent` and is kept as a [`ConsentRequest`] for the panel and Settings to show;
//! granting it runs nothing again.

mod call;
pub mod services;
pub mod storage;
mod watch;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Instant, SystemTime};

pub use dupin_broker::{
    Base, Broker, BrokerError, Capabilities, ClipboardAccess, ConsentReason, ConsentState, Denial, FilesKind,
    Generation, GrantSource, Local, LocalStore, MemoryLocal, OsAccess, OsConsumer, OsPermission, PackageId,
    Permit, Reconciliation, Request, Revision, SaveError, Scope, ScopeKey, ScopeName, StoreError, Trigger,
    WindowsAccess,
};
use dupin_extension_host::Services as HostServices;
use dupin_host_services::files;
pub use services::{Failure, Services};
pub use storage::{Origin, PackageStorage};

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

/// An image a package's row names by a handle it was given, never by its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageRef<'a> {
    /// The handle of a blob one of the package's records carries.
    Blob(&'a str),
    /// A file the package's clipboard watch delivered, by its path and the handle it came with.
    File { path: &'a str, handle: &'a str },
}

/// The image an [`ImageRef`] names, once the bridge has checked that it is the package's own.
#[derive(Clone, Copy, Debug)]
pub enum ImageData<'a> {
    /// A stored blob's bytes, which are wiped once the reader returns.
    Bytes(&'a [u8]),
    File(&'a Path),
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
    /// The package's own storage namespace, for the origin it was installed from.
    namespace: String,
    /// The calendar event links this call's replies gave the guest, the only ones not on the
    /// web its rows may open.
    links: Mutex<HashSet<String>>,
}

impl Session {
    /// The same package, revision and generation, asked for another reason.
    fn with_trigger(&self, trigger: Trigger) -> Self {
        Self {
            bridge: self.bridge.clone(),
            package: self.package.clone(),
            revision: self.revision.clone(),
            generation: self.generation,
            trigger,
            namespace: self.namespace.clone(),
            links: Mutex::default(),
        }
    }

    fn issue_links(&self, events: &serde_json::Value) {
        let links = events
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|event| event.get("link")?.as_str());
        self.links
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(links.map(str::to_owned));
    }
}

impl HostServices for Session {
    fn opens(&self, url: &str) -> bool {
        self.links
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(url)
    }

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
    /// What loading the package's catalogue reaches the host through while the call runs: for
    /// a search, the same services asked as Dupin asking for the catalogue
    /// ([`Trigger::Catalogue`]) rather than as typing.
    pub catalogue: Arc<dyn HostServices>,
    pub generation: Option<Generation>,
}

impl Call {
    /// A call that reaches no host service, as `dupin ext run` and tests make.
    pub fn none() -> Self {
        let services: Arc<dyn HostServices> = Arc::new(dupin_extension_host::NoServices);
        Self {
            catalogue: services.clone(),
            services,
            generation: None,
        }
    }
}

#[derive(Clone)]
pub struct Bridge {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    services: Services,
    /// The declarative base, if the bridge reads one from a file.
    base: Option<PathBuf>,
    /// Where decisions are saved, if anywhere. Locked before the state whenever both are.
    local: Mutex<Option<Box<dyn LocalStore>>>,
    /// Whether the broker holds what the store holds. Until then it holds unreadable grants,
    /// and only [`Bridge::open_grants`] holds the store's lock for longer than a write.
    read: AtomicBool,
    files: Mutex<HashMap<PackageId, FilesTable>>,
    watches: watch::Watches,
    /// The image of each package's latest clipboard change, which its handler may store
    /// without the bytes crossing to the guest.
    clips: Mutex<HashMap<PackageId, watch::Staged>>,
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
    /// Problems reading or saving grants, for Settings.
    problems: Vec<String>,
    /// The base file as the bridge last read it.
    seen: Option<Stamp>,
    /// Where each installed package came from, which picks its storage namespace.
    origins: HashMap<PackageId, Origin>,
}

/// How Settings names the local grants in a problem.
const LOCAL: &str = "your extension grants";

/// What a file looked like, so a change another process made, even an atomic replace of the
/// same size, is noticed without reading the file each time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
    inode: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    Some(Stamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
        inode: metadata.ino(),
    })
}

impl Bridge {
    /// A bridge over `broker`, which already holds what `local` contains, if anything.
    pub fn new(broker: Broker, local: Option<Box<dyn LocalStore>>, services: Services) -> Self {
        Self::over(broker, None, local, true, services)
    }

    fn over(
        broker: Broker,
        base: Option<PathBuf>,
        local: Option<Box<dyn LocalStore>>,
        read: bool,
        services: Services,
    ) -> Self {
        let seen = base.as_deref().and_then(stamp);
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    broker,
                    pending: Vec::new(),
                    asked_while_off: HashSet::new(),
                    os: HashMap::new(),
                    problems: Vec::new(),
                    seen,
                    origins: HashMap::new(),
                }),
                services,
                base,
                local: Mutex::new(local),
                read: AtomicBool::new(read),
                files: Mutex::default(),
                watches: watch::Watches::default(),
                clips: Mutex::default(),
                listener: Mutex::default(),
            }),
        }
    }

    /// Reads the declarative base at `base`, a missing file being empty, and takes `local` for
    /// the user's grants, which [`Bridge::open_grants`] reads. Until they are read nothing is
    /// granted and no decision can be made. Grants that can't be read grant nothing and are
    /// never overwritten; an unusable base is reported, and while it stands only local grants
    /// apply; a base entry that can't be read is reported and keeps its package off.
    pub fn open(base: &Path, local: Box<dyn LocalStore>, services: Services) -> Self {
        // Stamped before reading, so a change made during the read is read again later.
        let seen = stamp(base);
        let base_grants = Base::read(base);
        let problems: Vec<_> = base_grants
            .problems()
            .into_iter()
            .map(|problem| format!("{}: {problem}", base.display()))
            .collect();
        let bridge = Self::over(
            Broker::new(base_grants, Local::unreadable()),
            Some(base.to_owned()),
            Some(local),
            false,
            services,
        );
        let mut state = bridge.inner.lock();
        state.problems = problems;
        state.seen = seen;
        drop(state);
        bridge
    }

    /// Reads the user's grants if they haven't been read yet, and tells the app what that
    /// changed. It blocks on the store, which may read the Keychain, so the app calls it on a
    /// thread of its own, at startup and again while they couldn't be read.
    pub fn open_grants(&self) {
        let changed = self.inner.read_grants();
        self.inner.announce(changed);
    }

    /// Whether the user's grants have been read, so decisions can be made and saved.
    pub fn grants_read(&self) -> bool {
        self.inner.read.load(Ordering::Acquire)
    }

    /// Reads the base file again if it changed since the bridge last read it. Every call and
    /// request does this itself; the app also does it when it shows what extensions may do.
    pub fn reload(&self) {
        self.inner.sync();
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

    /// Records a package that ships inside Dupin at `revision`: while that revision is the one
    /// installed, its scopes are pre-granted and it is on, or waits for the user to turn it on
    /// when `on` is false. Call it before installing.
    pub fn set_bundled(&self, id: &PackageId, revision: Revision, on: bool) {
        self.inner.lock().broker.set_bundled(id.clone(), revision, on);
    }

    /// Loads or updates a package's declaration at `revision`. It never grants anything; what
    /// a new revision widens needs consent again, and what it no longer declares is dropped.
    pub fn install(&self, id: &PackageId, revision: Revision, capabilities: Capabilities) -> Reconciliation {
        self.load(id, revision, capabilities, Origin::User)
    }

    /// Loads a package that ships in Dupin, as [`Bridge::install`] does, into the bundled
    /// origin, which names its own storage namespace. While `revision` is the one
    /// [`Bridge::set_bundled`] recorded, its scopes are granted without asking.
    pub fn install_bundled(
        &self,
        id: &PackageId,
        revision: Revision,
        capabilities: Capabilities,
    ) -> Reconciliation {
        self.load(id, revision, capabilities, Origin::Bundled)
    }

    fn load(
        &self,
        id: &PackageId,
        revision: Revision,
        capabilities: Capabilities,
        origin: Origin,
    ) -> Reconciliation {
        // Until the grants are read there is nothing to save, and the store's lock is the
        // reader's, which may be waiting on the Keychain.
        let mut grants = self.grants_read().then(|| self.inner.grants());
        let mut state = self.inner.lock();
        let synced = self.inner.sync_locked(&mut state);
        state.origins.insert(id.clone(), origin);
        let before = state.broker.generation(id);
        let decisions = state.broker.decisions();
        let report = state.broker.install(id.clone(), revision, capabilities);
        let changed = before != state.broker.generation(id);
        if changed {
            state.pending.retain(|request| request.package != *id);
        }
        if !report.dropped.is_empty() || !report.changed.is_empty() || !report.kept.is_empty() {
            save(
                grants.as_mut().and_then(|grants| grants.as_deref_mut()),
                &mut state,
                &decisions,
            );
        }
        drop(state);
        drop(grants);
        if changed {
            self.inner.forget_files(id);
            self.inner.notify(Notice::Changed(id.clone()));
        }
        self.inner.announce(synced);
        report
    }

    /// Host services for one call of `id`, as `trigger` describes why it runs. The app sets the
    /// trigger from what it saw: typing, a chosen action or a watched event.
    pub fn call(&self, id: &PackageId, trigger: Trigger) -> Call {
        match self.session(id, trigger) {
            Some(session) => {
                let generation = Some(session.generation);
                let catalogue: Option<Arc<dyn HostServices>> = (trigger == Trigger::Query)
                    .then(|| Arc::new(session.with_trigger(Trigger::Catalogue)) as _);
                let services: Arc<dyn HostServices> = Arc::new(session);
                Call {
                    catalogue: catalogue.unwrap_or_else(|| services.clone()),
                    services,
                    generation,
                }
            }
            None => Call::none(),
        }
    }

    fn session(&self, id: &PackageId, trigger: Trigger) -> Option<Session> {
        self.inner.session(id, trigger)
    }

    /// Decides an effect Dupin performs itself for `id`, such as pasting a row's text, exactly
    /// as a request from the package's own code would be decided: a refusal for want of consent
    /// is kept for the panel and Settings. Pass the permit to [`Bridge::validate`] just before
    /// performing the effect.
    pub fn authorize(&self, id: &PackageId, request: &Request, trigger: Trigger) -> Result<Permit, Failure> {
        let session = self
            .session(id, trigger)
            .ok_or_else(|| failure_for(&Denial::NotInstalled))?;
        session.permit(request)
    }

    /// Hands `read` the image a row of `id` names, resolved in `id`'s own storage namespace
    /// alone, so no package can draw another's data. It reads storage and decides as typing
    /// does, but a refusal records no consent request: drawing a row is not the package asking.
    /// Call it off the panel's thread; storage blocks on file IO.
    pub fn read_image<R>(
        &self,
        id: &PackageId,
        image: &ImageRef<'_>,
        read: impl FnOnce(ImageData<'_>) -> R,
    ) -> Result<R, Failure> {
        self.session(id, Trigger::Query)
            .ok_or_else(|| failure_for(&Denial::NotInstalled))?
            .read_image(image, read)
    }

    /// Whether a permit from [`Bridge::authorize`] still holds.
    pub fn validate(&self, permit: &Permit) -> bool {
        self.inner.sync();
        self.inner.lock().broker.validate(permit)
    }

    /// Whether work stamped with `generation` may still be shown for `id`.
    pub fn is_current(&self, id: &PackageId, generation: Option<Generation>) -> bool {
        generation.is_none() || self.inner.lock().broker.generation(id) == generation
    }

    pub fn is_enabled(&self, id: &PackageId) -> bool {
        self.inner.lock().broker.is_enabled(id)
    }

    /// Whether `id` is the shipped revision of a bundled package that ships off, whose switch is
    /// then its feature's switch.
    pub fn ships_off(&self, id: &PackageId) -> bool {
        self.inner.lock().broker.ships_off(id)
    }

    /// Whether the user or the base turned `id` off, rather than never turning it on.
    pub fn is_turned_off(&self, id: &PackageId) -> bool {
        self.inner.lock().broker.is_turned_off(id)
    }

    /// Turns a package's capabilities on or off. Turning off drops its in-flight results.
    pub fn set_enabled(&self, id: &PackageId, enabled: bool) -> Result<(), SaveError> {
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
    ) -> Result<(), SaveError> {
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
    pub fn revoke(&self, id: &PackageId, key: &ScopeKey) -> Result<(), SaveError> {
        self.change(id, |broker| broker.revoke(id, key))?;
        self.inner
            .lock()
            .pending
            .retain(|request| request.package != *id || request.key() != *key);
        Ok(())
    }

    /// Forgets every decision about `id`, as removing or reinstalling the package does. A
    /// package that later takes the name starts afresh.
    pub fn forget_grants(&self, id: &PackageId) -> Result<(), SaveError> {
        self.change(id, |broker| broker.forget(id))?;
        let mut state = self.inner.lock();
        state.asked_while_off.remove(id);
        state.pending.retain(|request| request.package != *id);
        Ok(())
    }

    /// Where `id` was installed from, as [`Bridge::delete_data`] takes it.
    pub fn origin(&self, id: &PackageId) -> Origin {
        self.inner.lock().origins.get(id).copied().unwrap_or_default()
    }

    /// Deletes everything `id` keeps in storage as a package from `origin`. Revoking `storage`
    /// keeps the data, so allowing it again finds it; removing the package should delete it.
    pub fn delete_data(&self, id: &PackageId, origin: Origin) -> Result<(), Failure> {
        self.inner
            .services
            .storage
            .discard(&storage::namespace(id, origin))
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

    /// Problems reading or saving grants.
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
        self.inner
            .clips
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
    }

    /// Makes `change` and saves it, or leaves the decisions as they were if either fails, so
    /// the broker never enforces a choice the local grants don't hold.
    fn change(
        &self,
        id: &PackageId,
        change: impl FnOnce(&mut Broker) -> Result<(), BrokerError>,
    ) -> Result<(), SaveError> {
        if !self.grants_read() {
            return Err(SaveError::Refused(BrokerError::GrantsUnreadable));
        }
        let mut grants = self.inner.grants();
        let mut state = self.inner.lock();
        let synced = self.inner.sync_locked(&mut state);
        let before = state.broker.generation(id);
        let changed = match grants.as_deref_mut() {
            Some(store) => state.broker.change_saved(store, change),
            None => change(&mut state.broker).map_err(SaveError::Refused),
        };
        drop(grants);
        if let Err(error) = changed {
            drop(state);
            self.inner.announce(synced);
            return Err(error);
        }
        let bumped = state.broker.generation(id) != before;
        drop(state);
        if bumped {
            self.inner.forget_files(id);
        }
        self.inner.notify(Notice::Changed(id.clone()));
        self.inner.announce(synced);
        self.inner.refresh_watches();
        Ok(())
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn grants(&self) -> MutexGuard<'_, Option<Box<dyn LocalStore>>> {
        self.local.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Host services for one call of `id` under `trigger`, or none when `id` isn't installed.
    fn session(self: &Arc<Self>, id: &PackageId, trigger: Trigger) -> Option<Session> {
        self.sync();
        let state = self.lock();
        let (Some(revision), Some(generation)) = (state.broker.revision(id), state.broker.generation(id))
        else {
            return None;
        };
        Some(Session {
            bridge: self.clone(),
            package: id.clone(),
            revision: revision.clone(),
            generation,
            trigger,
            namespace: storage::namespace(id, state.origins.get(id).copied().unwrap_or_default()),
            links: Mutex::default(),
        })
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

    /// Reads the user's grants into the broker unless it holds them already, and returns the
    /// packages whose decisions that changed, for [`Inner::announce`]. What reconciling them
    /// with the installed declarations changes is saved at once.
    fn read_grants(&self) -> Vec<PackageId> {
        if self.read.load(Ordering::Acquire) {
            return Vec::new();
        }
        let mut grants = self.grants();
        let Some(store) = grants
            .as_deref_mut()
            .filter(|_| !self.read.load(Ordering::Acquire))
        else {
            return Vec::new();
        };
        let loaded = Local::load(store);
        let mut state = self.lock();
        let problem = loaded.as_ref().err().map(ToString::to_string);
        note_problem(&mut state, LOCAL, &problem);
        let Ok(loaded) = loaded else {
            return Vec::new();
        };
        let changed = state.broker.replace_local(loaded.clone());
        save(Some(store), &mut state, &loaded);
        self.read.store(true, Ordering::Release);
        state
            .pending
            .retain(|request| !changed.contains(&request.package));
        changed
    }

    /// Reads the base file again if it changed, then tells the app what that changed.
    fn sync(self: &Arc<Self>) {
        let changed = {
            let mut state = self.lock();
            self.sync_locked(&mut state)
        };
        self.announce(changed);
    }

    /// Reads the base file again if its stamp moved and returns the packages that changed,
    /// which the caller passes to [`Inner::announce`] once it lets go of the state.
    fn sync_locked(&self, state: &mut State) -> Vec<PackageId> {
        let mut changed = Vec::new();
        if let Some(path) = &self.base {
            let now = stamp(path);
            if now != state.seen {
                state.seen = now;
                let base = Base::read(path);
                note_problem(state, &path.display().to_string(), &base.problems());
                changed.extend(state.broker.replace_base(base));
            }
        }
        changed.sort();
        changed.dedup();
        state
            .pending
            .retain(|request| !changed.contains(&request.package));
        changed
    }

    fn announce(self: &Arc<Self>, changed: Vec<PackageId>) {
        if changed.is_empty() {
            return;
        }
        for id in changed {
            self.forget_files(&id);
            self.notify(Notice::Changed(id));
        }
        self.refresh_watches();
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

/// Saves what changed in the broker's decisions since `before`, or tells Settings why it
/// couldn't.
fn save(store: Option<&mut (dyn LocalStore + 'static)>, state: &mut State, before: &Local) {
    let Some(store) = store else {
        return;
    };
    if let Err(error) = state.broker.save(store, before) {
        let problem = format!("{LOCAL}: {error}");
        if !state.problems.contains(&problem) {
            state.problems.push(problem);
        }
    }
}

/// Replaces what Settings shows about reading `source` with `problems`.
fn note_problem<'a>(state: &mut State, source: &str, problems: impl IntoIterator<Item = &'a String>) {
    let prefix = format!("{source}: ");

    state.problems.retain(|known| !known.starts_with(&prefix));
    for problem in problems {
        state.problems.push(format!("{prefix}{problem}"));
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
