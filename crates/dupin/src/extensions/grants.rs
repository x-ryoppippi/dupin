//! The grants the user chose in Dupin: the `extension-grants` namespace of encrypted storage, one
//! record per package upserted under its package ID, as the broker encodes it. They hold the
//! paths the user bound, so they are never written in the clear, and they never sync
//! ([sync design](../../../../docs/design/sync.md#what-syncs)).
//!
//! The running launcher owns the namespace for its lifetime, through [`shared`]. `dupin ext`
//! opens it only while the launcher isn't running; while it is, the namespace is busy and the
//! command asks the launcher over the control socket instead. Every call blocks on the disk, and
//! on the Keychain the first time, so it runs on a worker or guest thread.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use dupin_extension_bridge::{Bridge, ConsentState, Local, LocalStore, PackageId};
use dupin_storage::{Error, Limits, NewRecord, Storage, Store};
use serde_json::Value;

pub const NAMESPACE: &str = "extension-grants";
/// What Settings calls the store when it can't be opened.
const NAME: &str = "Extension grants";

/// Room for far more packages than anyone installs. Writes refuse rather than evict, since a
/// decision evicted would hand a bundled package back a pre-grant the user revoked.
const LIMITS: Limits = Limits {
    max_age: None,
    max_entries: 10_000,
    max_total_bytes: 64 * 1024 * 1024,
    max_item_bytes: 1024 * 1024,
};

pub struct Grants {
    storage: Option<Arc<Storage>>,
    store: Option<Store>,
    /// Whether Settings hears whether it opened: only the launcher's own grants are shown.
    reports: bool,
}

/// The launcher's grants, over the data directory's storage. Only the launcher's bridge uses it.
pub fn shared() -> &'static Mutex<Grants> {
    static GRANTS: OnceLock<Mutex<Grants>> = OnceLock::new();
    GRANTS.get_or_init(|| {
        let storage = if cfg!(test) || rune::tooling::is_headless() {
            None
        } else {
            crate::extensions::access::shared_storage()
        };
        Mutex::new(Grants {
            reports: true,
            ..Grants::new(storage)
        })
    })
}

/// The [`LocalStore`] the launcher's bridge saves to: [`shared`].
pub struct Shared;

impl LocalStore for Shared {
    fn read(&mut self) -> Result<Vec<Value>, String> {
        shared().lock().unwrap_or_else(PoisonError::into_inner).read()
    }

    fn write(&mut self, changes: Vec<(PackageId, Option<Value>)>) -> Result<(), String> {
        shared()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .write(changes)
    }
}

impl Grants {
    pub fn new(storage: Option<Arc<Storage>>) -> Self {
        Self {
            storage,
            store: None,
            reports: false,
        }
    }

    /// The open namespace, opening it on first use and again after it failed, as a Keychain that
    /// was locked may have been unlocked since. With nothing ever saved and `create` false, it
    /// opens nothing, so no key is read.
    fn namespace(&mut self, create: bool) -> Result<Option<&Store>, Error> {
        if self.store.is_none() {
            let storage = self
                .storage
                .as_ref()
                .ok_or_else(|| Error::Key("extension grants need XDG_DATA_HOME or HOME".into()))?;
            if !create && !storage.exists(NAMESPACE) {
                return Ok(None);
            }
            let opened = storage.open(NAMESPACE, LIMITS);
            if self.reports {
                let problem = opened.as_ref().err().map(ToString::to_string);
                crate::stored::report(NAME, problem.as_deref());
            }

            self.store = Some(opened?);
        }
        Ok(self.store.as_ref())
    }

    /// Forgets `package`'s decisions, if any were saved.
    pub fn forget(&mut self, package: &PackageId) -> Result<(), Error> {
        if let Some(store) = self.namespace(false)? {
            store.delete(&store.key_id(package.as_str()))?;
        }
        Ok(())
    }

    /// Every package's decisions, read for a broker of `dupin ext`'s own.
    pub fn local(&mut self) -> Result<Local, Error> {
        let records = match self.namespace(false)? {
            Some(store) => records(store).map_err(Error::Corrupt)?,
            None => Vec::new(),
        };
        Local::from_records(records).map_err(|error| Error::Corrupt(error.to_string()))
    }
}

impl LocalStore for Grants {
    fn read(&mut self) -> Result<Vec<Value>, String> {
        match self.namespace(false).map_err(|error| error.to_string())? {
            Some(store) => records(store),
            None => Ok(Vec::new()),
        }
    }

    fn write(&mut self, changes: Vec<(PackageId, Option<Value>)>) -> Result<(), String> {
        let store = self
            .namespace(true)
            .map_err(|error| error.to_string())?
            .ok_or("extension grants couldn't be opened")?;
        for (package, record) in changes {
            match record {
                Some(data) => store
                    .upsert_within_limits(
                        package.as_str(),
                        NewRecord {
                            data,
                            blobs: Vec::new(),
                        },
                    )
                    .map(drop),
                None => store.delete(&store.key_id(package.as_str())).map(drop),
            }
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

/// What the launcher answers a `dupin ext` command that found its grants busy: it forgets a
/// package's decisions through its own bridge, or lists each named package's granted scopes.
pub fn answer(bridge: &Bridge, command: &crate::control::Command) -> Result<Value, String> {
    bridge.open_grants();
    match command {
        crate::control::Command::ExtensionGrantsForget { package } => {
            let id = PackageId::new(package.as_str())?;
            bridge
                .forget_grants(&id)
                .map(|()| Value::Null)
                .map_err(|error| error.to_string())
        }
        crate::control::Command::ExtensionGrantsList { packages } => {
            let mut granted = serde_json::Map::new();
            for package in packages {
                let id = PackageId::new(package.as_str())?;
                let scopes: Vec<_> = bridge
                    .scopes(&id)
                    .into_iter()
                    .filter(|entry| matches!(entry.state, ConsentState::Granted(_)))
                    .map(|entry| Value::from(entry.scope.key().to_string()))
                    .collect();
                granted.insert(package.clone(), scopes.into());
            }
            Ok(serde_json::json!({ "granted": granted }))
        }
        _ => Err("not a request about extension grants".into()),
    }
}

/// Every record that names the package it is stored under. One that names another was moved or
/// forged and is left out. Records sealed under a key this Mac doesn't hold make the whole
/// store unreadable: what they would deny is unknown.
fn records(store: &Store) -> Result<Vec<Value>, String> {
    let report = store.open_report();
    if report.skipped_records > 0 {
        return Err("some grants were saved with another key or by a newer Dupin".into());
    }
    if report.corrupt_records > 0 {
        crate::log::warn("extension grants: dropped a record that failed authentication");
    }
    let mut records = Vec::new();
    let mut before = None;
    loop {
        let page = store
            .list(before.as_ref(), 200)
            .map_err(|error| error.to_string())?;
        for record in page.records {
            match Local::record_package(&record.data) {
                Some(package) if store.key_id(package.as_str()) == record.id => records.push(record.data),
                _ => crate::log::warn("extension grants: left out a record saved under another package"),
            }
        }
        match page.next {
            Some(next) => before = Some(next),
            None => break,
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dupin_extension_bridge::{Capabilities, Revision, Services};
    use dupin_storage::{DataKey, MemoryKey};

    fn storage(root: &std::path::Path, key: &DataKey) -> Option<Arc<Storage>> {
        Some(Arc::new(Storage::new(root, MemoryKey::new(key.clone()))))
    }

    fn files(dir: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                found.extend(files(&path));
            } else {
                let bytes = std::fs::read(&path).unwrap();
                found.push((path, bytes));
            }
        }
        found.sort();
        found
    }

    const SECRET: &str = "/Users/x/Secret Folder";

    /// A bridge over `grants` with `notes` declaring a folder, which the user allowed at
    /// [`SECRET`] when `allow` is set.
    fn open_bridge(grants: Grants, allow: bool) -> (Bridge, PackageId) {
        let base = tempfile::tempdir().unwrap();
        let bridge = Bridge::open(
            &base.path().join("extension-grants.toml"),
            Box::new(grants),
            Services::none(),
        );
        bridge.open_grants();
        let notes = PackageId::new("notes").unwrap();
        let capabilities = Capabilities::parse(&serde_json::json!({
            "files": { "notes": { "kind": "directory", "access": ["read"] } }
        }))
        .unwrap();
        bridge.install(&notes, Revision::new("sha256:notes").unwrap(), capabilities);
        if allow {
            let scope = bridge.scopes(&notes).remove(0).scope;
            bridge
                .allow(
                    &notes,
                    &Revision::new("sha256:notes").unwrap(),
                    &scope,
                    Some(SECRET.into()),
                )
                .unwrap();
        }
        (bridge, notes)
    }

    fn granted(bridge: &Bridge, id: &PackageId) -> bool {
        bridge
            .scopes(id)
            .iter()
            .any(|entry| matches!(entry.state, dupin_extension_bridge::ConsentState::Granted(_)))
    }

    #[test]
    fn grants_are_sealed_and_outlive_the_launcher() {
        let root = tempfile::tempdir().unwrap();
        let key = DataKey::generate().unwrap();
        let (bridge, notes) = open_bridge(Grants::new(storage(root.path(), &key)), false);
        assert!(
            !root.path().join(NAMESPACE).exists(),
            "nothing is opened, and no key read, before the first decision"
        );
        let scope = bridge.scopes(&notes).remove(0).scope;
        bridge
            .allow(
                &notes,
                &Revision::new("sha256:notes").unwrap(),
                &scope,
                Some(SECRET.into()),
            )
            .unwrap();
        for (path, bytes) in files(root.path()) {
            let text = String::from_utf8_lossy(&bytes);
            for plain in [SECRET, "Secret Folder", "notes", "granted"] {
                assert!(
                    !text.contains(plain),
                    "{plain} is in {} in the clear",
                    path.display()
                );
            }
        }
        drop(bridge);

        let (reopened, notes) = open_bridge(Grants::new(storage(root.path(), &key)), false);
        assert!(granted(&reopened, &notes));
        assert!(reopened.is_enabled(&notes));
        assert!(reopened.problems().is_empty());

        reopened.forget_grants(&notes).unwrap();
        drop(reopened);
        let mut grants = Grants::new(storage(root.path(), &key));
        assert_eq!(
            grants.read().unwrap(),
            Vec::<Value>::new(),
            "no decisions, no record"
        );
    }

    #[test]
    fn a_wrong_or_missing_key_fails_closed_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let key = DataKey::generate().unwrap();
        drop(open_bridge(Grants::new(storage(root.path(), &key)), true));
        let before = files(root.path());

        let other = DataKey::generate().unwrap();
        let (wrong, notes) = open_bridge(Grants::new(storage(root.path(), &other)), false);
        assert!(!granted(&wrong, &notes), "nothing is granted");
        assert!(!wrong.is_enabled(&notes));
        assert!(!wrong.problems().is_empty());
        assert!(wrong.set_enabled(&notes, true).is_err());
        assert!(wrong.forget_grants(&notes).is_err());
        drop(wrong);
        assert!(Grants::new(storage(root.path(), &other)).local().is_err());

        let refusing = Some(Arc::new(Storage::new(root.path(), NoKey)));
        let (locked, notes) = open_bridge(Grants::new(refusing.clone()), false);
        assert!(locked.problems()[0].contains("the Keychain is locked"));
        assert!(locked.set_enabled(&notes, false).is_err());
        drop(locked);
        assert!(Grants::new(refusing.clone()).forget(&notes).is_err());

        assert_eq!(files(root.path()), before, "nothing written");
        let (kept, notes) = open_bridge(Grants::new(storage(root.path(), &key)), false);
        assert!(granted(&kept, &notes));
    }

    #[test]
    fn a_tampered_record_is_dropped_and_a_moved_one_left_out() {
        let root = tempfile::tempdir().unwrap();
        let key = DataKey::generate().unwrap();
        let mut grants = Grants::new(storage(root.path(), &key));
        let record = |package: &str| serde_json::json!({ "package": package, "enabled": true });
        grants
            .write(vec![
                (PackageId::new("one").unwrap(), Some(record("one"))),
                (PackageId::new("two").unwrap(), Some(record("two"))),
            ])
            .unwrap();
        drop(grants);
        let records = root.path().join(NAMESPACE).join("records");
        let mut paths: Vec<_> = std::fs::read_dir(&records)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        let mut bytes = std::fs::read(&paths[0]).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(&paths[0], bytes).unwrap();
        assert_eq!(
            Grants::new(storage(root.path(), &key)).read().unwrap().len(),
            1,
            "the tampered record is dropped"
        );

        let root = tempfile::tempdir().unwrap();
        let storage = storage(root.path(), &key).unwrap();
        {
            let namespace = storage.open(NAMESPACE, LIMITS).unwrap();
            namespace
                .upsert(
                    "notes",
                    NewRecord {
                        data: record("bundled:clipboard"),
                        blobs: Vec::new(),
                    },
                )
                .unwrap();
        }
        let mut moved = Grants::new(Some(storage));
        assert!(moved.read().unwrap().is_empty());
        assert_eq!(moved.local().unwrap(), Local::default());
    }

    #[test]
    fn a_second_owner_is_busy() {
        let root = tempfile::tempdir().unwrap();
        let key = DataKey::generate().unwrap();
        let mut owner = Grants::new(storage(root.path(), &key));
        owner
            .write(vec![(
                PackageId::new("notes").unwrap(),
                Some(serde_json::json!({ "package": "notes", "enabled": false })),
            )])
            .unwrap();
        let mut second = Grants::new(storage(root.path(), &key));
        assert!(matches!(second.local(), Err(Error::Busy)));
        assert!(matches!(
            second.forget(&PackageId::new("notes").unwrap()),
            Err(Error::Busy)
        ));
        drop(owner);
        assert_eq!(
            second.local().unwrap(),
            Local::from_records(vec![serde_json::json!({ "package": "notes", "enabled": false })]).unwrap()
        );
    }

    #[test]
    fn a_decision_that_cannot_be_saved_changes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let (bridge, notes) = open_bridge(
            Grants::new(Some(Arc::new(Storage::new(root.path(), NoKey)))),
            false,
        );
        assert!(
            bridge.problems().is_empty(),
            "nothing saved yet reads as no decisions"
        );
        assert!(bridge.set_enabled(&notes, false).is_err());
        assert!(!bridge.is_turned_off(&notes));
    }

    struct NoKey;

    impl dupin_storage::KeyProvider for NoKey {
        fn data_key(&self) -> Result<DataKey, Error> {
            Err(Error::Key("the Keychain is locked".into()))
        }
    }
}
