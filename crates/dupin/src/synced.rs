//! The built-in namespaces that sync, and the stores their owners hold open. An owner opens its
//! namespace through [`open`], which seals it under the vault key while sync is on and under the
//! data key otherwise, and registers the store so the sync worker can carry it. When sync turns
//! on or off, the worker asks every owner to reopen, which re-seals its files; when a pass
//! brings another Mac's changes, it tells the owner to read them again.

use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};

use dupin_storage::{Error, Limits, Storage, Store, VaultKey};
use dupin_sync::Mode;

/// Every built-in namespace sync carries, with how it merges.
pub const SYNCED: &[(&str, Mode)] = &[
    ("usage", Mode::PerDevice),
    (crate::snippets::NAMESPACE, Mode::Lww),
    (crate::settings::overlay::SHARED, Mode::Lww),
];

type Callback = Arc<dyn Fn() + Send + Sync>;

/// An owner's open store, and how to tell it about sync.
struct Owner {
    name: String,
    mode: Mode,
    store: Weak<Store>,
    /// Drop the store and open it again through [`open`].
    reopen: Callback,
    /// Read the store again: a pass imported another Mac's versions.
    changed: Callback,
}

/// What a namespace that syncs is opened under while sync is on, set by the sync worker.
static VAULT: RwLock<Option<VaultKey>> = RwLock::new(None);
/// The extensions' package storage, whose granted namespaces reopen when the vault changes.
static EXTENSIONS: std::sync::OnceLock<Weak<dupin_extension_bridge::PackageStorage>> =
    std::sync::OnceLock::new();
static OWNERS: Mutex<Vec<Owner>> = Mutex::new(Vec::new());
/// Extension namespaces whose `storage:sync` grant was revoked, which this Mac is leaving.
static LEAVING: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Which namespaces this Mac's `[sync]` switches let sync carry.
static SWITCHES: RwLock<Option<dupin_core::config::Sync>> = RwLock::new(None);

/// This Mac's `[sync]` switches; owners reopen when the sync worker next sets the vault.
pub fn set_switches(switches: dupin_core::config::Sync) {
    *SWITCHES.write().unwrap_or_else(PoisonError::into_inner) = Some(switches);
}

fn switched_on(name: &str) -> bool {
    SWITCHES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .is_none_or(|switches| switches.includes(name))
}

fn mode(name: &str) -> Option<Mode> {
    SYNCED
        .iter()
        .find(|(synced, _)| *synced == name)
        .map(|(_, mode)| *mode)
}

/// Opens `name` under the vault key when it syncs and sync is on, under the data key otherwise.
/// Either way the storage re-seals what an earlier open left under the other key.
pub fn open(storage: &Storage, name: &str, limits: Limits) -> Result<Arc<Store>, Error> {
    let vault = VAULT.read().unwrap_or_else(PoisonError::into_inner).clone();
    let store = match vault.filter(|_| mode(name).is_some() && switched_on(name)) {
        Some(vault) => storage.open_synced(name, limits, &vault)?,
        None => storage.open(name, limits)?,
    };
    Ok(Arc::new(store))
}

/// Lets sync carry `store`, the one its owner opened through [`open`] for `name`. A store
/// registered again replaces the one before.
pub fn register(
    name: &'static str,
    store: &Arc<Store>,
    reopen: impl Fn() + Send + Sync + 'static,
    changed: impl Fn() + Send + Sync + 'static,
) {
    let Some(mode) = mode(name) else {
        return;
    };
    register_as(name.to_owned(), mode, store, reopen, changed);
}

/// [`register`] for an extension's namespace, which syncs `lww` under the name its grant gave.
pub fn register_extension(name: String, store: &Arc<Store>, reopen: impl Fn() + Send + Sync + 'static) {
    LEAVING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|leaving| *leaving != name);
    register_as(name, Mode::Lww, store, reopen, || {});
}

/// This Mac stops syncing the extension namespace `name`.
pub fn leave_extension(name: &str) {
    let mut leaving = LEAVING.lock().unwrap_or_else(PoisonError::into_inner);
    if !leaving.iter().any(|known| known == name) {
        leaving.push(name.to_owned());
    }
}

/// Extension namespaces this Mac is leaving, until a pass says it has.
pub fn leaving_extensions() -> Vec<String> {
    LEAVING.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

pub fn left_extension(name: &str) {
    LEAVING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|leaving| leaving != name);
}

fn register_as(
    name: String,
    mode: Mode,
    store: &Arc<Store>,
    reopen: impl Fn() + Send + Sync + 'static,
    changed: impl Fn() + Send + Sync + 'static,
) {
    let mut owners = OWNERS.lock().unwrap_or_else(PoisonError::into_inner);
    owners.retain(|owner| owner.name != name && owner.store.strong_count() > 0);
    owners.push(Owner {
        name,
        mode,
        store: Arc::downgrade(store),
        reopen: Arc::new(reopen),
        changed: Arc::new(changed),
    });
}

/// The key namespaces that sync open under from now on; owners holding one open reopen it.
/// Runs on the sync worker, between passes.
pub fn set_vault(vault: Option<VaultKey>) {
    *VAULT.write().unwrap_or_else(PoisonError::into_inner) = vault;
    let reopen: Vec<Callback> = OWNERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|owner| owner.store.strong_count() > 0)
        .map(|owner| owner.reopen.clone())
        .collect();
    for reopen in reopen {
        reopen();
    }
    if let Some(packages) = EXTENSIONS.get().and_then(Weak::upgrade) {
        packages.reopen_synced();
    }
}

/// The registered stores still open, for a sync pass.
pub fn open_stores() -> Vec<(String, Mode, Arc<Store>)> {
    OWNERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter_map(|owner| Some((owner.name.clone(), owner.mode, owner.store.upgrade()?)))
        .collect()
}

/// The vault key while sync is on, for an extension namespace granted `storage:sync`.
pub fn vault() -> Option<VaultKey> {
    VAULT.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// What the extension bridge asks of sync: which name a Git-installed package syncs under,
/// and opening a granted package's namespace under the vault key while sync is on.
pub struct ExtensionSync {
    packages: std::sync::OnceLock<Weak<dupin_extension_bridge::PackageStorage>>,
}

impl ExtensionSync {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            packages: std::sync::OnceLock::new(),
        })
    }

    /// The package storage this host serves, which it asks to reopen a namespace.
    pub fn serve(&self, packages: &Arc<dupin_extension_bridge::PackageStorage>) {
        let _ = self.packages.set(Arc::downgrade(packages));
        let _ = EXTENSIONS.set(Arc::downgrade(packages));
    }
}

impl dupin_extension_bridge::storage::SyncHost for ExtensionSync {
    fn synced_name(&self, package: &dupin_extension_bridge::PackageId) -> Option<String> {
        let lock = crate::extensions::installed::shared()?.read().ok()?;
        let entry = lock.packages.get(package.as_str())?;
        entry.commit.as_ref()?;
        Some(dupin_extension_bridge::storage::synced_namespace(
            &entry.source,
            package,
        ))
    }

    fn open(
        &self,
        storage: &Storage,
        namespace: &str,
        synced: &str,
        limits: Limits,
    ) -> Option<Result<Store, Error>> {
        let vault = vault()?;
        Some(storage.open_synced_as(namespace, synced, limits, &vault))
    }

    fn opened(&self, namespace: &str, synced: &str, store: &Arc<Store>) {
        let packages = self.packages.get().cloned();
        let namespace = namespace.to_owned();
        register_extension(synced.to_owned(), store, move || {
            if let Some(packages) = packages.as_ref().and_then(Weak::upgrade) {
                packages.reopen(&namespace);
            }
        });
    }

    fn left(&self, synced: &str) {
        leave_extension(synced);
    }
}

/// Tells the owner of `name` that a pass changed its records.
pub fn changed(name: &str) {
    let callbacks: Vec<Callback> = OWNERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|owner| owner.name == name && owner.store.strong_count() > 0)
        .map(|owner| owner.changed.clone())
        .collect();
    for callback in callbacks {
        callback();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_namespaces_sync() {
        assert_eq!(mode("usage"), Some(Mode::PerDevice));
        assert_eq!(mode("snippets"), Some(Mode::Lww));
        assert_eq!(mode("snapshots"), None, "the design keeps snapshots on each Mac");
        assert_eq!(mode("settings"), Some(Mode::Lww));
        assert_eq!(
            mode("settings-local"),
            None,
            "device-local settings never leave the Mac"
        );
    }

    #[test]
    fn a_synced_namespace_opens_under_the_vault_while_sync_is_on_and_others_never_do() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path().join("storage"), dupin_storage::MemoryKey::random());
        let vault = VaultKey::new([5; 16], 1, dupin_storage::DataKey::generate().unwrap()).unwrap();
        let reopened = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let store = open(&storage, "snippets", Limits::default()).unwrap();
        let local_id = store.namespace_id();
        let counter = reopened.clone();
        register(
            "snippets",
            &store,
            move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
            || {},
        );
        set_vault(Some(vault.clone()));
        assert_eq!(reopened.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(open_stores().len(), 1);
        drop(store);
        let synced = open(&storage, "snippets", Limits::default()).unwrap();
        assert_eq!(synced.namespace_id(), vault.namespace_id("snippets"));
        assert_ne!(synced.namespace_id(), local_id);
        let snapshots = open(&storage, "snapshots", Limits::default()).unwrap();
        assert_ne!(snapshots.namespace_id(), vault.namespace_id("snapshots"));
        set_vault(None);
        drop(synced);
        assert!(open_stores().is_empty(), "a dropped store isn't synced");
    }
}
