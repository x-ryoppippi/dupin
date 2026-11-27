//! What Settings saves over a read-only config.toml, such as one Nix manages: encrypted storage,
//! one record per top-level key, in `settings` for the keys every Mac shares and `settings-local`
//! for the rest, as the [sync design](../../../../docs/design/sync.md#settings) assigns them.
//! While sync is on, `settings` is sealed under the vault key and syncs; `settings-local` never
//! leaves the Mac.
//!
//! One `Overlay` per process owns both namespaces once it opens them. In the launcher that is the
//! one [`shared`] returns; another process, such as `dupin ext install`, then finds them
//! [`Error::Busy`] and asks the launcher for its configuration instead.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use dupin_storage::{Error, Limits, NewRecord, Storage, Store};
use serde::{Deserialize, Serialize};

pub const SHARED: &str = "settings";
pub const LOCAL: &str = "settings-local";

const LIMITS: Limits = Limits {
    max_age: None,
    max_entries: 1000,
    max_total_bytes: 16 * 1024 * 1024,
    max_item_bytes: 4 * 1024 * 1024,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    key: String,
    value: toml::Value,
}

pub struct Overlay {
    storage: Arc<Storage>,
    open: Mutex<Option<Open>>,
}

struct Open {
    shared: Arc<Store>,
    local: Arc<Store>,
    /// Every stored key, with whether it sits in `settings`, as this process last read or wrote it.
    stored: BTreeMap<String, (bool, toml::Value)>,
}

type Listener = Box<dyn Fn() + Send + Sync>;

/// Told when sync brought another Mac's shared settings, so the configuration is read again.
static LISTENER: OnceLock<Listener> = OnceLock::new();

/// Calls `listener` whenever sync changed the stored shared settings.
pub fn on_synced(listener: impl Fn() + Send + Sync + 'static) {
    let _ = LISTENER.set(Box::new(listener));
}

/// The launcher's overlay over the data directory's storage; none in a headless run or a test,
/// which never read the user's Keychain.
pub fn shared() -> Option<Arc<Overlay>> {
    static OVERLAY: OnceLock<Option<Arc<Overlay>>> = OnceLock::new();
    OVERLAY
        .get_or_init(|| {
            if cfg!(test) || rune::tooling::is_headless() {
                return None;
            }
            crate::extensions::access::shared_storage().map(|storage| Arc::new(Overlay::new(storage)))
        })
        .clone()
}

impl Overlay {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            open: Mutex::default(),
        }
    }

    /// The stored keys as TOML, or `None` while there are none. With nothing ever stored it opens
    /// nothing, so the key is first read when Settings saves over a read-only config.toml.
    pub fn read(&self) -> Result<Option<String>, Error> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if open.is_none() && !self.storage.exists(SHARED) && !self.storage.exists(LOCAL) {
            return Ok(None);
        }
        let open = self.opened(&mut open)?;
        if open.stored.is_empty() {
            return Ok(None);
        }
        let table: toml::Table = open
            .stored
            .iter()
            .map(|(key, (_, value))| (key.clone(), value.clone()))
            .collect();
        toml::to_string(&table)
            .map(Some)
            .map_err(|error| Error::Corrupt(format!("the stored settings ({error})")))
    }

    /// Stores `overlay` in place of what is stored, writing only the keys that changed and
    /// moving a key to the namespace its classification names.
    pub fn write(&self, overlay: &toml::Table) -> Result<(), Error> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let open = self.opened(&mut open)?;
        for (key, value) in overlay {
            let shared = dupin_core::config::is_shared(key);
            match open.stored.get(key) {
                Some((was_shared, stored)) if *was_shared == shared && stored == value => continue,
                Some((was_shared, _)) if *was_shared != shared => {
                    let from = if *was_shared { &open.shared } else { &open.local };
                    from.delete(&from.key_id(key))?;
                }
                _ => {}
            }
            let to = if shared { &open.shared } else { &open.local };
            let data = serde_json::to_value(Entry {
                key: key.clone(),
                value: value.clone(),
            })?;
            to.upsert_within_limits(
                key,
                NewRecord {
                    data,
                    blobs: Vec::new(),
                },
            )?;
            if shared {
                crate::sync::wrote();
            }
            open.stored.insert(key.clone(), (shared, value.clone()));
        }
        let removed: Vec<_> = open
            .stored
            .keys()
            .filter(|key| !overlay.contains_key(*key))
            .cloned()
            .collect();
        for key in removed {
            if let Some((shared, _)) = open.stored.remove(&key) {
                let from = if shared { &open.shared } else { &open.local };
                from.delete(&from.key_id(&key))?;
            }
        }
        Ok(())
    }

    /// Closes both namespaces and opens them again, under the key sync now calls for.
    fn reopen(&self) {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if open.take().is_some()
            && let Err(error) = self.opened(&mut open)
        {
            crate::log::warn(&format!("stored settings: {error}"));
        }
    }

    /// Reads both namespaces again after sync changed the shared one, then says so.
    fn resynced(&self) {
        {
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(opened) = open.as_mut() else {
                return;
            };
            match read_both(&opened.shared, &opened.local) {
                Ok(stored) => opened.stored = stored,
                Err(error) => {
                    crate::log::warn(&format!("stored settings: {error}"));
                    return;
                }
            }
        }
        if let Some(listener) = LISTENER.get() {
            listener();
        }
    }

    /// Both namespaces, opened and read on first use, and again after a failure, as a Keychain
    /// that was locked may have been unlocked since.
    fn opened<'a>(&self, open: &'a mut Option<Open>) -> Result<&'a mut Open, Error> {
        if open.is_none() {
            let shared = crate::synced::open(&self.storage, SHARED, LIMITS)?;
            let local = Arc::new(self.storage.open(LOCAL, LIMITS)?);
            let stored = read_both(&shared, &local)?;
            if !cfg!(test) && !rune::tooling::is_headless() {
                crate::synced::register(
                    SHARED,
                    &shared,
                    || {
                        if let Some(overlay) = shared_overlay() {
                            overlay.reopen();
                        }
                    },
                    || {
                        if let Some(overlay) = shared_overlay() {
                            overlay.resynced();
                        }
                    },
                );
            }
            *open = Some(Open {
                shared,
                local,
                stored,
            });
        }
        Ok(open.as_mut().expect("opened above"))
    }
}

fn shared_overlay() -> Option<Arc<Overlay>> {
    shared()
}

/// Every stored key, with whether it sits in `settings`.
fn read_both(shared: &Store, local: &Store) -> Result<BTreeMap<String, (bool, toml::Value)>, Error> {
    let mut stored = BTreeMap::new();
    for (namespace, in_shared) in [(shared, true), (local, false)] {
        for (key, value) in entries(namespace)? {
            stored.insert(key, (in_shared, value));
        }
    }
    Ok(stored)
}

fn entries(namespace: &Store) -> Result<Vec<(String, toml::Value)>, Error> {
    let mut entries = Vec::new();
    let mut before = None;
    loop {
        let page = namespace.list(before.as_ref(), 200)?;
        for record in page.records {
            match serde_json::from_value::<Entry>(record.data) {
                // A record is stored under its key, so a body naming another key was moved or
                // forged and is left out.
                Ok(entry) if namespace.key_id(&entry.key) == record.id => {
                    entries.push((entry.key, entry.value))
                }
                _ => crate::log::warn("stored settings: left out a record that isn't a setting"),
            }
        }
        match page.next {
            Some(next) => before = Some(next),
            None => return Ok(entries),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage(root: &std::path::Path, key: &dupin_storage::DataKey) -> Arc<Storage> {
        Arc::new(Storage::new(root, dupin_storage::MemoryKey::new(key.clone())))
    }

    fn table(source: &str) -> toml::Table {
        source.parse().unwrap()
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

    fn records(root: &std::path::Path, namespace: &str) -> usize {
        std::fs::read_dir(root.join(namespace).join("records")).map_or(0, |entries| entries.count())
    }

    #[test]
    fn keys_are_sealed_where_the_sync_design_puts_them_and_outlive_the_overlay() {
        let root = tempfile::tempdir().unwrap();
        let key = dupin_storage::DataKey::generate().unwrap();
        let overlay = Overlay::new(storage(root.path(), &key));
        assert_eq!(overlay.read().unwrap(), None);
        assert!(!root.path().join(SHARED).exists(), "nothing opened before a save");

        let saved = table(
            "hotkey = \"ctrl+alt+k\"\nfile_limit = 123\nfile_demotions = [\"~/Secret Project\"]\n[theme]\npanel_width = 900\n",
        );
        overlay.write(&saved).unwrap();
        assert_eq!(
            records(root.path(), SHARED),
            3,
            "file_limit, file_demotions, theme"
        );
        assert_eq!(records(root.path(), LOCAL), 1, "hotkey");
        for bytes in files(root.path()) {
            let text = String::from_utf8_lossy(&bytes);
            for plain in ["ctrl+alt+k", "Secret Project", "file_limit", "panel_width"] {
                assert!(!text.contains(plain), "{plain} is on disk in the clear");
            }
        }
        drop(overlay);

        let reopened = Overlay::new(storage(root.path(), &key));
        let read = reopened.read().unwrap().unwrap();
        assert_eq!(table(&read), saved);

        reopened.write(&table("file_limit = 7\n")).unwrap();
        assert_eq!(records(root.path(), SHARED), 1);
        assert_eq!(records(root.path(), LOCAL), 0);
        drop(reopened);
        let reopened = Overlay::new(storage(root.path(), &key));
        assert_eq!(
            table(&reopened.read().unwrap().unwrap()),
            table("file_limit = 7\n")
        );
        reopened.write(&toml::Table::new()).unwrap();
        assert_eq!(reopened.read().unwrap(), None);
    }

    #[test]
    fn a_missing_or_wrong_key_fails_closed_and_another_process_finds_it_busy() {
        let root = tempfile::tempdir().unwrap();
        let key = dupin_storage::DataKey::generate().unwrap();
        let overlay = Overlay::new(storage(root.path(), &key));
        overlay.write(&table("file_limit = 9\n")).unwrap();
        assert!(matches!(
            Overlay::new(storage(root.path(), &key)).read(),
            Err(Error::Busy)
        ));
        drop(overlay);
        let before = files(root.path());

        let wrong = Overlay::new(storage(root.path(), &dupin_storage::DataKey::generate().unwrap()));
        assert!(wrong.read().is_err());
        assert!(wrong.write(&table("file_limit = 1\n")).is_err());
        drop(wrong);
        let locked = Overlay::new(Arc::new(Storage::new(root.path(), NoKey)));
        assert!(matches!(locked.read(), Err(Error::Key(_))));
        drop(locked);
        assert_eq!(files(root.path()).len(), before.len(), "nothing written");

        let right = Overlay::new(storage(root.path(), &key));
        assert_eq!(table(&right.read().unwrap().unwrap()), table("file_limit = 9\n"));
    }

    #[test]
    fn a_record_moved_under_another_key_is_left_out() {
        let root = tempfile::tempdir().unwrap();
        let key = dupin_storage::DataKey::generate().unwrap();
        let storage = storage(root.path(), &key);
        {
            let namespace = storage.open(SHARED, LIMITS).unwrap();
            let data = serde_json::to_value(Entry {
                key: "file_limit".into(),
                value: toml::Value::Integer(1),
            })
            .unwrap();
            namespace
                .upsert(
                    "show_recent",
                    NewRecord {
                        data,
                        blobs: Vec::new(),
                    },
                )
                .unwrap();
        }
        assert_eq!(Overlay::new(storage).read().unwrap(), None);
    }

    struct NoKey;

    impl dupin_storage::KeyProvider for NoKey {
        fn data_key(&self) -> Result<dupin_storage::DataKey, Error> {
            Err(Error::Key("the Keychain is locked".into()))
        }
    }
}
