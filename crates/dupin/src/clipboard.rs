//! Clipboard history's lifetime. The bundled extension in extensions/clipboard-history keeps
//! the history in its own encrypted storage and lists it; Dupin hands it the clipboard watch
//! and the `[clipboard]` settings while the history is on, and deletes everything it stored as
//! soon as it is turned off, and again at every start while it is off.

use std::sync::{Arc, RwLock};

use dupin_core::config;
use dupin_core::session::unix_now;
use dupin_extension_bridge::{Bridge, Origin, PackageId};

/// The bundled extension that keeps and lists the history.
pub(crate) const PACKAGE: &str = "clipboard-history";

pub struct Clipboard {
    /// What the clipboard service reads for every copy: the apps whose copies are never read
    /// and the largest text and image kept, which every extension follows too.
    settings: Arc<RwLock<config::Clipboard>>,
    enabled: bool,
}

impl Clipboard {
    pub fn new(config: &config::Clipboard) -> Self {
        Self {
            settings: Arc::new(RwLock::new(config.clone())),
            enabled: config.enabled,
        }
    }

    pub fn settings(&self) -> Arc<RwLock<config::Clipboard>> {
        self.settings.clone()
    }

    /// Deletes what the history stored while it is off, however that came about: turned off in
    /// the config file while Dupin was quit, or a deletion that quitting cut short. Nothing runs
    /// for the history yet, so this waits for nothing.
    pub fn start(&self, bridge: &Bridge) {
        if !self.enabled {
            delete(bridge);
        }
    }

    /// Follows `config`, and deletes what the history stored when it was turned off. Returns
    /// whether the history was turned on or off.
    pub fn apply(&mut self, config: &config::Clipboard, bridge: &Bridge) -> bool {
        *self
            .settings
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config.clone();
        let toggled = self.enabled != config.enabled;
        self.enabled = config.enabled;
        if toggled && !config.enabled {
            forget(bridge);
        }
        toggled
    }
}

/// Deletes what the history stored off the panel's thread, since deleting waits for the
/// history's requests already running.
fn forget(bridge: &Bridge) {
    let bridge = bridge.clone();
    std::thread::spawn(move || delete(&bridge));
}

/// Deletes what the bundled history, and a copy of it in the extensions folder, stored.
fn delete(bridge: &Bridge) {
    let packages = [
        (format!("bundled:{PACKAGE}"), Origin::Bundled),
        (PACKAGE.to_owned(), Origin::User),
    ];
    for (id, origin) in packages {
        let Ok(id) = PackageId::new(id) else { continue };
        if let Err(error) = bridge.delete_data(&id, origin) {
            crate::log::warn(&format!("couldn't delete the stored clipboard history: {error}"));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A bridge over storage in `root` that holds a record in the bundled history's namespace,
    /// and that namespace's folder.
    fn stored_history(root: &std::path::Path) -> (Bridge, std::path::PathBuf) {
        let package = PackageId::new(format!("bundled:{PACKAGE}")).unwrap();
        let namespace = dupin_extension_bridge::storage::namespace(&package, Origin::Bundled);
        let storage = dupin_storage::Storage::new(root, dupin_storage::MemoryKey::random());
        storage
            .open(&namespace, dupin_storage::Limits::default())
            .unwrap()
            .put(dupin_storage::NewRecord {
                data: serde_json::json!({ "kind": "text", "text": "secret", "copiedAt": 0 }),
                blobs: Vec::new(),
            })
            .unwrap();
        let bridge = Bridge::new(
            dupin_extension_bridge::Broker::new(Default::default(), Default::default()),
            None,
            dupin_extension_bridge::Services {
                storage: Box::new(dupin_extension_bridge::PackageStorage::new(storage)),
                ..dupin_extension_bridge::Services::none()
            },
        );
        (bridge, root.join(namespace))
    }

    #[test]
    fn a_history_that_starts_off_deletes_what_it_stored_before() {
        let root = tempfile::tempdir().unwrap();
        let (bridge, namespace) = stored_history(root.path());
        let mut config = config::Clipboard::default();
        config.enabled = true;
        Clipboard::new(&config).start(&bridge);
        assert!(namespace.exists(), "kept while on");

        config.enabled = false;
        Clipboard::new(&config).start(&bridge);
        assert!(!namespace.exists(), "deleted at start while off");
    }
}
