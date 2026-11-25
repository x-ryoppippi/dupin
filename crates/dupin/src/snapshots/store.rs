//! Saved snapshots: the `snapshots` namespace of encrypted storage, one record per snapshot
//! under its ID. A snapshot holds a query and the results it showed, so it is never written in
//! the clear; snapshots stay on this Mac ([sync design](../../../../docs/design/sync.md#what-syncs)).
//!
//! One [`Store`] per process owns the namespace. Every call blocks on the disk, and on the
//! Keychain the first time, so it runs on the snapshot writer's thread or a search worker,
//! never the UI thread.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use dupin_core::snapshots::{MAX_BYTES, MAX_ENTRIES, MAX_ENTRY_BYTES, Snapshot};
use dupin_storage::{Error, Limits, NewRecord, Storage};

pub const NAMESPACE: &str = "snapshots";
/// What Settings calls the store when it can't be opened.
const NAME: &str = "Saved snapshots";

const LIMITS: Limits = Limits {
    max_age: None,
    max_entries: MAX_ENTRIES,
    max_total_bytes: MAX_BYTES as u64,
    max_item_bytes: MAX_ENTRY_BYTES as u64,
};

pub struct Store {
    storage: Option<Arc<Storage>>,
    namespace: Option<dupin_storage::Store>,
    /// Newest first.
    entries: Vec<Snapshot>,
    problem: Option<String>,
}

/// The launcher's store: the data directory's storage, a scratch one holding the
/// `DUPIN_HEADLESS_SNAPSHOTS` fixture in a headless run, and none in unit tests.
pub fn shared() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| {
        let storage = if cfg!(test) {
            None
        } else if rune::tooling::is_headless() {
            headless_storage()
        } else {
            crate::extensions::access::shared_storage()
        };
        Mutex::new(Store::open(storage))
    })
}

pub fn lock(store: &Mutex<Store>) -> MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Store {
    /// Reads the saved snapshots, if any were ever saved. With none, it reads no key until the
    /// first save.
    pub fn open(storage: Option<Arc<Storage>>) -> Self {
        let mut store = Self {
            storage,
            namespace: None,
            entries: Vec::new(),
            problem: None,
        };
        if store
            .storage
            .as_ref()
            .is_some_and(|storage| storage.exists(NAMESPACE))
        {
            let _ = store.namespace();
        }
        store
    }

    /// Newest first.
    pub fn list(&self) -> &[Snapshot] {
        &self.entries
    }

    /// Why the saved snapshots couldn't be read, while they can't.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    pub fn save(&mut self, mut snapshot: Snapshot) -> Result<String, String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Clock unavailable.")?;
        snapshot.saved_at = now.as_secs();
        snapshot.id = format!("{:x}", now.as_nanos());
        snapshot.validate()?;
        if self.entries.iter().any(|saved| saved.id == snapshot.id) {
            return Err("Snapshot identifier collision.".into());
        }
        let data = serde_json::to_value(&snapshot).map_err(|_| "Cannot encode snapshot.")?;
        self.namespace()?
            .upsert_within_limits(
                &snapshot.id,
                NewRecord {
                    data,
                    blobs: Vec::new(),
                },
            )
            .map_err(|error| match error {
                Error::QuotaExceeded { entries, .. } if entries > MAX_ENTRIES => {
                    "Snapshot limit reached. Delete an entry before saving.".to_owned()
                }
                Error::QuotaExceeded { .. } => {
                    "Snapshot storage is full. Delete an entry before saving.".to_owned()
                }
                Error::TooLarge { .. } => "Snapshot exceeds the supported size or schema.".to_owned(),
                other => format!("Cannot save the snapshot: {other}."),
            })?;
        let id = snapshot.id.clone();
        self.entries.insert(0, snapshot);
        Ok(id)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        self.namespace()?;
        if !self.entries.iter().any(|saved| saved.id == id) {
            return Ok(());
        }
        let namespace = self.namespace()?;
        namespace
            .delete(&namespace.key_id(id))
            .map_err(|error| format!("Cannot delete the snapshot: {error}."))?;
        self.entries.retain(|saved| saved.id != id);
        Ok(())
    }

    /// The open namespace, opening and reading it on first use, and again after it failed, as
    /// a Keychain that was locked may have been unlocked since.
    fn namespace(&mut self) -> Result<&dupin_storage::Store, String> {
        if self.namespace.is_none() {
            let read = self.read();
            crate::stored::report(NAME, read.as_ref().err().map(String::as_str));
            match read {
                Ok((namespace, entries)) => {
                    self.namespace = Some(namespace);
                    self.entries = entries;
                    self.problem = None;
                }
                Err(problem) => {
                    self.problem = Some(problem.clone());
                    return Err(problem);
                }
            }
        }
        self.namespace
            .as_ref()
            .ok_or_else(|| "Saved snapshots aren't available.".into())
    }

    fn read(&self) -> Result<(dupin_storage::Store, Vec<Snapshot>), String> {
        let storage = self
            .storage
            .as_ref()
            .ok_or("Saved snapshots need XDG_DATA_HOME or HOME")?;
        let namespace = storage
            .open(NAMESPACE, LIMITS)
            .map_err(|error| error.to_string())?;
        let mut entries = Vec::new();
        let mut before = None;
        loop {
            let page = namespace
                .list(before.as_ref(), 200)
                .map_err(|error| error.to_string())?;
            for record in page.records {
                match serde_json::from_value::<Snapshot>(record.data) {
                    // A record is stored under its snapshot's ID, so a body naming another ID
                    // was moved or forged and is left out.
                    Ok(snapshot)
                        if snapshot.validate().is_ok() && namespace.key_id(&snapshot.id) == record.id =>
                    {
                        entries.push(snapshot);
                    }
                    _ => crate::log::warn("saved snapshots: left out a record that isn't a snapshot"),
                }
            }
            match page.next {
                Some(next) => before = Some(next),
                None => break,
            }
        }
        entries.sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then_with(|| b.id.cmp(&a.id)));
        Ok((namespace, entries))
    }
}

/// A scratch store, gone with the process, holding the snapshots listed in
/// `DUPIN_HEADLESS_SNAPSHOTS`, a JSON list, as if they had been saved.
fn headless_storage() -> Option<Arc<Storage>> {
    let path = std::env::var_os("DUPIN_HEADLESS_SNAPSHOTS")?;
    dupin_storage::scratch_process();
    let root = tempfile::Builder::new()
        .prefix("dupin-headless-snapshots-")
        .tempdir()
        .ok()?
        .keep();
    let storage = Arc::new(Storage::new(root, dupin_storage::MemoryKey::random()));
    let fixture = std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|source| serde_json::from_str::<Vec<Snapshot>>(&source).map_err(|error| error.to_string()));
    let seeded = fixture.and_then(|snapshots| {
        let namespace = storage
            .open(NAMESPACE, LIMITS)
            .map_err(|error| error.to_string())?;
        for snapshot in snapshots {
            let data = serde_json::to_value(&snapshot).map_err(|error| error.to_string())?;
            namespace
                .upsert(
                    &snapshot.id,
                    NewRecord {
                        data,
                        blobs: Vec::new(),
                    },
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    });
    if let Err(error) = seeded {
        crate::log::warn(&format!("couldn't seed the snapshots fixture: {error}"));
    }
    Some(storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dupin_core::View;

    fn storage(root: &std::path::Path, key: &dupin_storage::DataKey) -> Option<Arc<Storage>> {
        Some(Arc::new(Storage::new(
            root,
            dupin_storage::MemoryKey::new(key.clone()),
        )))
    }

    fn fixture(query: &str) -> Snapshot {
        Snapshot {
            version: 1,
            id: String::new(),
            saved_at: 0,
            mode: "apps.search".into(),
            explicit: false,
            query: query.into(),
            title: "Calculator".into(),
            views: vec![View::Text {
                title: "Calculator".into(),
                subtitle: "/System/Applications/Calculator.app".into(),
            }],
            state: None,
        }
    }

    fn files(dir: &std::path::Path) -> Vec<Vec<u8>> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                found.extend(files(&path));
            } else {
                found.push(std::fs::read(path).unwrap());
            }
        }
        found
    }

    #[test]
    fn snapshots_are_sealed_and_outlive_the_store() {
        let root = tempfile::tempdir().unwrap();
        let key = dupin_storage::DataKey::generate().unwrap();
        let mut store = Store::open(storage(root.path(), &key));
        assert!(
            !root.path().join(NAMESPACE).exists(),
            "nothing is opened, and no key read, before the first save"
        );
        let first = store.save(fixture("secret query")).unwrap();
        let second = store.save(fixture("another")).unwrap();
        assert_eq!(store.list()[0].id, second);
        assert!(store.problem().is_none());

        for bytes in files(root.path()) {
            let text = String::from_utf8_lossy(&bytes);
            for plain in ["secret query", "Calculator", "/System/Applications"] {
                assert!(!text.contains(plain), "{plain} is on disk in the clear");
            }
        }

        drop(store);
        let mut reopened = Store::open(storage(root.path(), &key));
        assert_eq!(
            reopened
                .list()
                .iter()
                .map(|saved| saved.id.as_str())
                .collect::<Vec<_>>(),
            [second.as_str(), first.as_str()]
        );
        reopened.delete(&first).unwrap();
        reopened.delete("never saved").unwrap();
        drop(reopened);
        let reopened = Store::open(storage(root.path(), &key));
        assert_eq!(reopened.list().len(), 1);
        assert_eq!(reopened.list()[0].query, "another");
    }

    #[test]
    fn a_missing_or_wrong_key_fails_closed_and_keeps_what_was_saved() {
        let root = tempfile::tempdir().unwrap();
        let key = dupin_storage::DataKey::generate().unwrap();
        Store::open(storage(root.path(), &key))
            .save(fixture("kept"))
            .unwrap();
        let before = files(root.path());

        let other = dupin_storage::DataKey::generate().unwrap();
        let mut wrong = Store::open(storage(root.path(), &other));
        assert!(wrong.problem().is_some());
        assert!(wrong.list().is_empty());
        assert!(wrong.save(fixture("lost")).is_err());
        assert!(wrong.delete("anything").is_err());
        drop(wrong);

        let mut missing = Store::open(Some(Arc::new(Storage::new(root.path(), NoKey))));
        assert!(missing.problem().unwrap().contains("storage key"));
        assert!(missing.save(fixture("lost")).is_err());
        drop(missing);

        assert_eq!(files(root.path()).len(), before.len(), "nothing written");
        let reopened = Store::open(storage(root.path(), &key));
        assert_eq!(reopened.list().len(), 1);
        assert_eq!(reopened.list()[0].query, "kept");
        assert!(
            !root.path().join("snapshots").join("archive.json").exists(),
            "never a plaintext fallback"
        );
    }

    #[test]
    fn a_tampered_record_is_dropped_and_a_moved_one_left_out() {
        let root = tempfile::tempdir().unwrap();
        let key = dupin_storage::DataKey::generate().unwrap();
        let mut store = Store::open(storage(root.path(), &key));
        store.save(fixture("one")).unwrap();
        store.save(fixture("two")).unwrap();
        drop(store);
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
        let store = Store::open(storage(root.path(), &key));
        assert!(store.problem().is_none());
        assert_eq!(store.list().len(), 1, "the tampered record is dropped");

        let root = tempfile::tempdir().unwrap();
        let storage = storage(root.path(), &key).unwrap();
        {
            let namespace = storage.open(NAMESPACE, LIMITS).unwrap();
            let mut forged = fixture("forged");
            forged.id = "other".into();
            namespace
                .upsert(
                    "listed",
                    NewRecord {
                        data: serde_json::to_value(&forged).unwrap(),
                        blobs: Vec::new(),
                    },
                )
                .unwrap();
        }
        assert!(Store::open(Some(storage)).list().is_empty());
    }

    #[test]
    fn limits_refuse_a_save_without_evicting_one() {
        let root = tempfile::tempdir().unwrap();
        let key = dupin_storage::DataKey::generate().unwrap();
        let mut store = Store::open(storage(root.path(), &key));
        let mut oversized = fixture("large");
        oversized.views.push(View::Detail {
            title: "large".into(),
            body: "x".repeat(MAX_ENTRY_BYTES),
        });
        assert!(store.save(oversized).is_err());
        for _ in 0..MAX_ENTRIES {
            store.save(fixture("full")).unwrap();
        }
        assert_eq!(
            store.save(fixture("one more")).unwrap_err(),
            "Snapshot limit reached. Delete an entry before saving."
        );
        drop(store);
        assert_eq!(Store::open(storage(root.path(), &key)).list().len(), MAX_ENTRIES);
    }

    struct NoKey;

    impl dupin_storage::KeyProvider for NoKey {
        fn data_key(&self) -> Result<dupin_storage::DataKey, Error> {
            Err(Error::Key("the Keychain is locked".into()))
        }
    }
}
