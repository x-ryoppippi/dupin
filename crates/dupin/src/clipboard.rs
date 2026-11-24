//! The clipboard history's lifetime: watched while `[clipboard]` is enabled, and forgotten,
//! with everything its store kept, as soon as it is turned off.

use std::sync::{Arc, Mutex};

use dupin_core::clipboard::{
    Access, Content, FileRef, History, MemoryStore, Policy, Record, Store, StoreError,
};
use dupin_core::config;
use dupin_core::session::unix_now;

pub struct Clipboard {
    history: Arc<Mutex<History>>,
    monitor: Option<dupin_mac::clipboard::Monitor>,
    enabled: bool,
}

/// Where the history persists. Records stay in memory until the encrypted store from
/// `dupin-storage` replaces this, keyed to the native history's own namespace.
fn store() -> Box<dyn Store> {
    Box::new(MemoryStore::default())
}

fn report(error: &StoreError) {
    crate::log::warn(&format!("couldn't update the stored clipboard history: {error}"));
}

impl Clipboard {
    pub fn new(config: &config::Clipboard) -> Self {
        let history = History::with_store(config.into(), store(), report, unix_now());
        let mut clipboard = Self {
            history: Arc::new(Mutex::new(history)),
            monitor: None,
            enabled: false,
        };
        clipboard.apply(config);
        clipboard
    }

    pub fn history(&self) -> Arc<Mutex<History>> {
        self.history.clone()
    }

    /// Whether the history's contents or recording changed, so a list on screen is stale.
    pub fn apply(&mut self, config: &config::Clipboard) -> bool {
        let mut history = self.history.lock().unwrap();
        let policy = Policy::from(config);
        let mut changed = self.enabled != config.enabled || *history.policy() != policy;
        history.set_policy(policy, unix_now());
        if !config.enabled {
            self.monitor.take();
            if let Err(error) = history.clear() {
                report(&error);
            }
            history.set_access(Access::default());
        } else if rune::tooling::is_headless() {
            if !self.enabled {
                seed_fixture(&mut history);
            }
        } else if self.monitor.is_none() {
            changed = true;
            self.monitor = dupin_mac::clipboard::Monitor::start(self.history.clone())
                .map_err(|error| crate::log::warn(&format!("couldn't watch the clipboard: {error}")))
                .ok();
        }
        self.enabled = config.enabled;
        changed
    }

    pub fn record(&self, id: u64) -> Option<Arc<Record>> {
        self.history.lock().unwrap().record(id)
    }

    pub fn delete(&self, id: u64) -> Result<(), StoreError> {
        self.history.lock().unwrap().remove(id).map(drop)
    }

    pub fn clear(&self) -> Result<(), StoreError> {
        self.history.lock().unwrap().clear()
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureEntry {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    files: Vec<std::path::PathBuf>,
    /// Seconds before the render.
    age: u64,
}

/// The bundled clipboard history's records, as its package stores them, from
/// `DUPIN_HEADLESS_CLIPBOARD`, a JSON list of entries, oldest first.
fn fixture_records(now_ms: u64) -> Vec<serde_json::Value> {
    let Some(path) = std::env::var_os("DUPIN_HEADLESS_CLIPBOARD") else {
        return Vec::new();
    };
    let entries: Vec<FixtureEntry> = match std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|source| serde_json::from_str(&source).map_err(|error| error.to_string()))
    {
        Ok(entries) => entries,
        Err(error) => {
            crate::log::warn(&format!("couldn't read the clipboard fixture: {error}"));
            return Vec::new();
        }
    };
    entries
        .into_iter()
        .map(|entry| {
            let copied_at = now_ms.saturating_sub(entry.age * 1000);
            match entry.text {
                Some(text) => serde_json::json!({ "kind": "text", "text": text, "copiedAt": copied_at }),
                None => {
                    let files: Vec<_> = entry
                        .files
                        .iter()
                        .map(|path| {
                            serde_json::json!({
                                "path": path,
                                "name": path.file_name().map(|name| name.to_string_lossy()),
                                "folder": path.parent().map(|parent| dupin_core::item::tilde(&parent.to_string_lossy())),
                            })
                        })
                        .collect();
                    serde_json::json!({ "kind": "files", "files": files, "copiedAt": copied_at })
                }
            }
        })
        .collect()
}

/// Storage for a headless run: a scratch store, gone with the process, holding the clipboard
/// fixture in the bundled clipboard history's namespace, as copies it watched would have left.
pub fn headless_storage() -> Option<dupin_extension_bridge::PackageStorage> {
    dupin_storage::scratch_process();
    let root = tempfile::Builder::new()
        .prefix("dupin-headless-storage-")
        .tempdir()
        .ok()?
        .keep();
    let storage = dupin_storage::Storage::new(root, dupin_storage::MemoryKey::random());
    let records = fixture_records(unix_now() * 1000);
    if !records.is_empty() {
        let package = dupin_extension_bridge::PackageId::new(format!("bundled:{PACKAGE}")).ok()?;
        let namespace =
            dupin_extension_bridge::storage::namespace(&package, dupin_extension_bridge::Origin::Bundled);
        let store = storage
            .open(&namespace, dupin_storage::Limits::default())
            .inspect_err(|error| crate::log::warn(&format!("couldn't seed the clipboard fixture: {error}")))
            .ok()?;
        for data in records {
            if let Err(error) = store.put(dupin_storage::NewRecord {
                data,
                blobs: Vec::new(),
            }) {
                crate::log::warn(&format!("couldn't seed the clipboard fixture: {error}"));
                break;
            }
        }
    }
    Some(dupin_extension_bridge::PackageStorage::new(storage))
}

/// The bundled extension, in extensions/clipboard-history, that keeps and lists the history.
pub(crate) const PACKAGE: &str = "clipboard-history";

/// Seed the history from `DUPIN_HEADLESS_CLIPBOARD`, a JSON list of entries, oldest first.
fn seed_fixture(history: &mut History) {
    let Some(path) = std::env::var_os("DUPIN_HEADLESS_CLIPBOARD") else {
        return;
    };
    let entries: Vec<FixtureEntry> = match std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|source| serde_json::from_str(&source).map_err(|error| error.to_string()))
    {
        Ok(entries) => entries,
        Err(error) => {
            crate::log::warn(&format!("couldn't read the clipboard fixture: {error}"));
            return;
        }
    };
    let now = unix_now();
    for entry in entries {
        let content = match entry.text {
            Some(text) => Content::Text(text),
            None => Content::Files(entry.files.into_iter().map(FileRef::new).collect()),
        };
        history.push(content, None, now.saturating_sub(entry.age));
    }
}
