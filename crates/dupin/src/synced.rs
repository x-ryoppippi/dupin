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
];

type Callback = Arc<dyn Fn() + Send + Sync>;

/// An owner's open store, and how to tell it about sync.
struct Owner {
    name: &'static str,
    store: Weak<Store>,
    /// Drop the store and open it again through [`open`].
    reopen: Callback,
    /// Read the store again: a pass imported another Mac's versions.
    changed: Callback,
}

/// What a namespace that syncs is opened under while sync is on, set by the sync worker.
static VAULT: RwLock<Option<VaultKey>> = RwLock::new(None);
static OWNERS: Mutex<Vec<Owner>> = Mutex::new(Vec::new());

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
    let store = match vault.filter(|_| mode(name).is_some()) {
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
    let mut owners = OWNERS.lock().unwrap_or_else(PoisonError::into_inner);
    owners.retain(|owner| owner.name != name && owner.store.strong_count() > 0);
    owners.push(Owner {
        name,
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
}

/// The registered stores still open, for a sync pass.
pub fn open_stores() -> Vec<(&'static str, Mode, Arc<Store>)> {
    OWNERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter_map(|owner| Some((owner.name, mode(owner.name)?, owner.store.upgrade()?)))
        .collect()
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
        assert_eq!(mode("settings-local"), None);
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
