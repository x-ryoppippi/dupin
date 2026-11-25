//! The snippets the user stores in Dupin: the `snippets` namespace of encrypted storage, one
//! record per snippet under its keyword, as the [sync design](../../../docs/design/sync.md#snippets)
//! names it, so syncing it later moves the same records. config.toml's snippets stay in
//! config.toml; Dupin lists both, and hands them to the bundled `snippets` package.
//!
//! Every call that reaches the store blocks on file I/O and runs on a worker thread or a guest
//! call's thread, never the UI thread; the UI reads only the list held in memory.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use dupin_core::config::Snippet as ConfigSnippet;
use dupin_core::snippets::{Problems, Snippet, check_save};
use dupin_extension_bridge::Failure;
use dupin_extension_bridge::services::{self, SnippetDraft};
use dupin_storage::{Limits, NewRecord, Storage, Store};
use serde_json::Value;

/// The namespace's name, which the sync design gives built-in snippets.
pub const NAMESPACE: &str = "snippets";

/// Room for every snippet Dupin lists, so storing one never evicts another: writes go through
/// `upsert_within_limits`, which refuses instead.
const LIMITS: Limits = Limits {
    max_age: None,
    max_entries: dupin_core::snippets::MAX_SNIPPETS,
    max_total_bytes: 64 * 1024 * 1024,
    max_item_bytes: 1024 * 1024,
};

type Listener = Box<dyn Fn() + Send + Sync>;

/// config.toml's snippets and paste setting, as the last configuration handed them over.
#[derive(Clone, Default)]
struct Declared {
    snippets: BTreeMap<String, ConfigSnippet>,
    paste: bool,
}

#[derive(Default)]
enum Opened {
    #[default]
    Closed,
    Open(Box<Store>),
    Failed(String),
}

pub struct Library {
    storage: Option<Arc<Storage>>,
    /// The namespace once opened; held while a write checks and stores, so two writers never
    /// pass the checks together.
    store: Mutex<Opened>,
    /// The stored snippets in keyword order, as the last load or write left them.
    stored: RwLock<Arc<Vec<Snippet>>>,
    declared: RwLock<Declared>,
    changed: RwLock<Vec<Listener>>,
}

/// The launcher's library: the data directory's storage, a scratch store in a headless run, and
/// none in unit tests, which make their own.
pub fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let storage = if cfg!(test) {
            None
        } else if rune::tooling::is_headless() {
            headless_storage()
        } else {
            crate::extensions::access::shared_storage()
        };
        Library::new(storage)
    })
}

fn headless_storage() -> Option<Arc<Storage>> {
    dupin_storage::scratch_process();
    let root = tempfile::Builder::new()
        .prefix("dupin-headless-snippets-")
        .tempdir()
        .ok()?
        .keep();
    Some(Arc::new(Storage::new(root, dupin_storage::MemoryKey::random())))
}

impl Library {
    pub fn new(storage: Option<Arc<Storage>>) -> Self {
        Self {
            storage,
            store: Mutex::default(),
            stored: RwLock::default(),
            declared: RwLock::default(),
            changed: RwLock::default(),
        }
    }

    /// Calls `listener` after each change to the stored snippets, on the thread that made it.
    pub fn on_change(&self, listener: impl Fn() + Send + Sync + 'static) {
        write(&self.changed).push(Box::new(listener));
    }

    /// The stored snippets in keyword order, without touching the disk.
    pub fn stored(&self) -> Arc<Vec<Snippet>> {
        read(&self.stored).clone()
    }

    /// Why the stored snippets couldn't be read, if they couldn't.
    pub fn problem(&self) -> Option<String> {
        match &*lock(&self.store) {
            Opened::Failed(problem) => Some(problem.clone()),
            _ => None,
        }
    }

    /// Opens the namespace and reads every snippet, once; later calls do nothing. Blocks on
    /// the disk, and on the Keychain the first time. With nothing ever stored, it opens nothing,
    /// so the key is first read when the user stores a snippet.
    pub fn load(&self) {
        if !self
            .storage
            .as_ref()
            .is_some_and(|storage| storage.exists(NAMESPACE))
        {
            return;
        }
        let loaded = {
            let mut store = lock(&self.store);
            matches!(*store, Opened::Closed) && self.open(&mut store).is_ok()
        };
        if loaded {
            self.changed();
        }
    }

    /// The configuration's snippets and paste setting, which a save is checked against and the
    /// package is handed beside the stored ones.
    pub fn declare(&self, snippets: &BTreeMap<String, ConfigSnippet>, paste: bool) {
        *write(&self.declared) = Declared {
            snippets: snippets.clone(),
            paste,
        };
    }

    /// What the bundled package reads as `ctx.preferences` now, and why any snippet is left
    /// out.
    pub fn preferences(&self) -> (dupin_js_runtime::Preferences, Option<String>) {
        let declared = read(&self.declared).clone();
        let (preferences, notice) =
            dupin_core::snippets::preferences(&declared.snippets, &self.stored(), declared.paste);
        let problem = self
            .problem()
            .map(|problem| format!("Stored snippets couldn't be read: {problem}"));
        let notice = match (notice, problem) {
            (Some(notice), Some(problem)) => Some(format!("{notice}\n{problem}")),
            (notice, problem) => notice.or(problem),
        };
        (preferences, notice)
    }

    /// Stores `draft` in place of the stored snippet `replaces`, or as a new one, once it
    /// passes [`check_save`]; returns what is wrong otherwise. Renaming deletes the record
    /// under the old keyword after storing the new one, so a crash between leaves both.
    pub fn save(&self, draft: Snippet, replaces: Option<&str>) -> Result<Problems, String> {
        let draft = draft.normalized();
        {
            let mut opened = lock(&self.store);
            let store = self.open(&mut opened)?;
            let declared = read(&self.declared).snippets.clone();
            let stored = self.stored();
            let problems = check_save(&declared, &stored, &draft, replaces);
            if !problems.is_empty() {
                return Ok(problems);
            }
            store
                .upsert_within_limits(&draft.keyword, record(&draft)?)
                .map_err(|error| error.to_string())?;
            if let Some(old) = replaces.filter(|old| *old != draft.keyword) {
                store
                    .delete(&store.key_id(old))
                    .map_err(|error| error.to_string())?;
            }
            let mut next: Vec<Snippet> = stored
                .iter()
                .filter(|snippet| {
                    Some(snippet.keyword.as_str()) != replaces && snippet.keyword != draft.keyword
                })
                .cloned()
                .collect();
            next.push(draft);
            next.sort_by(|a, b| a.keyword.cmp(&b.keyword));
            *write(&self.stored) = Arc::new(next);
        }
        self.changed();
        Ok(Problems::default())
    }

    /// Deletes the stored snippet `keyword`, and says whether there was one.
    pub fn delete(&self, keyword: &str) -> Result<bool, String> {
        let deleted = {
            let mut opened = lock(&self.store);
            let store = self.open(&mut opened)?;
            let deleted = store
                .delete(&store.key_id(keyword))
                .map_err(|error| error.to_string())?;
            let next: Vec<Snippet> = self
                .stored()
                .iter()
                .filter(|snippet| snippet.keyword != keyword)
                .cloned()
                .collect();
            *write(&self.stored) = Arc::new(next);
            deleted
        };
        if deleted {
            self.changed();
        }
        Ok(deleted)
    }

    /// Stores each of `snippets`, config.toml's, in place of a stored one with the same keyword,
    /// which config.toml's already took the place of; the caller then removes them from
    /// config.toml. An invalid one is left where it is. Returns the keywords stored.
    pub fn import(&self, snippets: &BTreeMap<String, ConfigSnippet>) -> Result<Vec<String>, String> {
        let mut moved = Vec::new();
        {
            let mut opened = lock(&self.store);
            let store = self.open(&mut opened)?;
            let mut stored: BTreeMap<String, Snippet> = self
                .stored()
                .iter()
                .map(|snippet| (snippet.keyword.clone(), snippet.clone()))
                .collect();
            for (keyword, snippet) in snippets {
                let draft = Snippet {
                    keyword: keyword.clone(),
                    name: snippet.name.clone(),
                    text: snippet.text.clone(),
                }
                .normalized();
                let others: Vec<Snippet> = stored
                    .values()
                    .filter(|other| other.keyword != draft.keyword)
                    .cloned()
                    .collect();
                if !check_save(&BTreeMap::new(), &others, &draft, None).is_empty() {
                    continue;
                }
                let written = store.upsert_within_limits(&draft.keyword, record(&draft)?);
                if let Err(error) = written {
                    *write(&self.stored) = Arc::new(stored.into_values().collect());
                    drop(opened);
                    self.changed();
                    return Err(error.to_string());
                }
                moved.push(draft.keyword.clone());
                stored.insert(draft.keyword.clone(), draft);
            }
            *write(&self.stored) = Arc::new(stored.into_values().collect());
        }
        self.changed();
        Ok(moved)
    }

    /// The open namespace, opening and reading it on first use, and again after it failed, as
    /// a Keychain that was locked may have been unlocked since.
    fn open<'a>(&self, opened: &'a mut Opened) -> Result<&'a Store, String> {
        if !matches!(opened, Opened::Open(_)) {
            *opened = match self.read_all() {
                Ok((store, snippets)) => {
                    *write(&self.stored) = Arc::new(snippets);
                    Opened::Open(Box::new(store))
                }
                Err(problem) => {
                    crate::log::warn(&format!("stored snippets: {problem}"));
                    Opened::Failed(problem)
                }
            };
        }
        match opened {
            Opened::Open(store) => Ok(&**store),
            Opened::Failed(problem) => Err(format!("Stored snippets couldn't be read: {problem}")),
            Opened::Closed => Err("Stored snippets aren't available".into()),
        }
    }

    fn read_all(&self) -> Result<(Store, Vec<Snippet>), String> {
        let storage = self
            .storage
            .as_ref()
            .ok_or("Dupin's storage needs XDG_DATA_HOME or HOME")?;
        let store = storage
            .open(NAMESPACE, LIMITS)
            .map_err(|error| error.to_string())?;
        let mut snippets = BTreeMap::new();
        let mut before = None;
        loop {
            let page = store
                .list(before.as_ref(), 200)
                .map_err(|error| error.to_string())?;
            for record in page.records {
                match serde_json::from_value::<Snippet>(record.data) {
                    // The record's key is its keyword, so a body naming another keyword was
                    // moved or forged; it is left out rather than listed twice.
                    Ok(snippet) if store.key_id(&snippet.keyword) == record.id => {
                        snippets.insert(snippet.keyword.clone(), snippet);
                    }
                    _ => crate::log::warn("stored snippets: left out a record that isn't a snippet"),
                }
            }
            match page.next {
                Some(next) => before = Some(next),
                None => break,
            }
        }
        Ok((store, snippets.into_values().collect()))
    }

    /// Hands the bundled package the snippets as they are now, before anything that changed
    /// them answers, then tells the listeners.
    fn changed(&self) {
        let (preferences, _) = self.preferences();
        crate::features::snippets::hand_over(preferences);
        for listener in read(&self.changed).iter() {
            listener();
        }
    }
}

fn record(snippet: &Snippet) -> Result<NewRecord, String> {
    Ok(NewRecord {
        data: serde_json::to_value(snippet).map_err(|error| error.to_string())?,
        blobs: Vec::new(),
    })
}

impl services::Snippets for &'static Library {
    fn save(&self, draft: SnippetDraft, replaces: Option<&str>) -> Result<Value, Failure> {
        let draft = Snippet {
            keyword: draft.keyword,
            name: draft.name,
            text: draft.text,
        };
        let problems =
            Library::save(self, draft, replaces).map_err(|problem| Failure::new("unavailable", problem))?;
        serde_json::to_value(problems).map_err(|error| Failure::new("failed", error.to_string()))
    }

    fn delete(&self, keyword: &str) -> Result<bool, Failure> {
        Library::delete(self, keyword).map_err(|problem| Failure::new("unavailable", problem))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(root: &std::path::Path, key: &dupin_storage::DataKey) -> Library {
        Library::new(Some(Arc::new(Storage::new(
            root,
            dupin_storage::MemoryKey::new(key.clone()),
        ))))
    }

    fn snippet(keyword: &str, name: Option<&str>, text: &str) -> Snippet {
        Snippet {
            keyword: keyword.into(),
            name: name.map(str::to_owned),
            text: text.into(),
        }
    }

    fn keywords(library: &Library) -> Vec<String> {
        library
            .stored()
            .iter()
            .map(|snippet| snippet.keyword.clone())
            .collect()
    }

    #[test]
    fn snippets_are_sealed_in_their_namespace_and_outlive_the_library() {
        let root = tempfile::tempdir().unwrap();
        let key = dupin_storage::DataKey::generate().unwrap();
        {
            let library = library(root.path(), &key);
            library.load();
            assert!(library.stored().is_empty());
            let saved = library
                .save(
                    snippet("sig", Some("  Email signature "), "Best wishes,\nSherlock"),
                    None,
                )
                .unwrap();
            assert!(saved.is_empty());
            assert!(
                library
                    .save(snippet("today", None, "{date}"), None)
                    .unwrap()
                    .is_empty()
            );
        }
        let namespace = root.path().join(NAMESPACE);
        assert!(namespace.join("key.check").exists());
        for entry in std::fs::read_dir(namespace.join("records")).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            let sealed = String::from_utf8_lossy(&bytes);
            assert!(
                !sealed.contains("Sherlock") && !sealed.contains("sig"),
                "sealed at rest"
            );
        }

        let library = library(root.path(), &key);
        library.load();
        assert_eq!(
            *library.stored(),
            [
                snippet("sig", Some("Email signature"), "Best wishes,\nSherlock"),
                snippet("today", None, "{date}"),
            ]
        );
    }

    #[test]
    fn edits_rename_and_delete_by_keyword_and_refuse_what_config_toml_holds() {
        let root = tempfile::tempdir().unwrap();
        let library = library(root.path(), &dupin_storage::DataKey::generate().unwrap());
        let declared = BTreeMap::from([(
            "addr".to_owned(),
            ConfigSnippet {
                name: None,
                text: "221B Baker Street".into(),
            },
        )]);
        library.declare(&declared, false);
        assert!(
            library
                .save(snippet("sig", None, "Cheers"), None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            library
                .save(snippet("addr", None, "Elsewhere"), None)
                .unwrap()
                .keyword
                .as_deref(),
            Some("config.toml already has a snippet with this keyword")
        );
        assert_eq!(
            library
                .save(snippet("sig", None, "Again"), None)
                .unwrap()
                .keyword
                .as_deref(),
            Some("Another snippet has this keyword")
        );
        assert!(
            library
                .save(snippet("sign", Some("Sign-off"), "Cheers"), Some("sig"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(keywords(&library), ["sign"]);
        assert!(!library.delete("addr").unwrap(), "config.toml's stay");
        assert!(library.delete("sign").unwrap());
        assert!(!library.delete("sign").unwrap());
        assert!(library.stored().is_empty());

        let reopened = Library::new(library.storage.clone());
        drop(library);
        reopened.load();
        assert!(
            reopened.stored().is_empty(),
            "the rename left nothing under the old keyword"
        );
    }

    #[test]
    fn moving_config_toml_snippets_stores_the_ones_it_can() {
        let root = tempfile::tempdir().unwrap();
        let library = library(root.path(), &dupin_storage::DataKey::generate().unwrap());
        assert!(
            library
                .save(snippet("sig", None, "Stored"), None)
                .unwrap()
                .is_empty()
        );
        let declared = BTreeMap::from([
            (
                "sig".to_owned(),
                ConfigSnippet {
                    name: Some("Signature".into()),
                    text: "From config.toml".into(),
                },
            ),
            (
                "two words".to_owned(),
                ConfigSnippet {
                    name: None,
                    text: "x".into(),
                },
            ),
        ]);
        assert_eq!(library.import(&declared).unwrap(), ["sig"]);
        assert_eq!(
            *library.stored(),
            [snippet("sig", Some("Signature"), "From config.toml")],
            "config.toml's took the stored one's place, as it did while listed"
        );
    }

    #[test]
    fn a_key_that_fails_reports_and_keeps_the_records() {
        let root = tempfile::tempdir().unwrap();
        let first = library(root.path(), &dupin_storage::DataKey::generate().unwrap());
        assert!(first.save(snippet("sig", None, "x"), None).unwrap().is_empty());
        drop(first);
        let other = library(root.path(), &dupin_storage::DataKey::generate().unwrap());
        other.load();
        assert!(other.problem().is_some());
        assert!(other.save(snippet("new", None, "x"), None).is_err());
        assert!(other.preferences().1.unwrap().contains("couldn't be read"));
        assert!(
            root.path()
                .join(NAMESPACE)
                .join("records")
                .read_dir()
                .unwrap()
                .next()
                .is_some()
        );
    }

    #[test]
    fn a_record_moved_under_another_keyword_is_left_out() {
        let root = tempfile::tempdir().unwrap();
        let key = dupin_storage::DataKey::generate().unwrap();
        let storage = Storage::new(root.path(), dupin_storage::MemoryKey::new(key.clone()));
        {
            let store = storage.open(NAMESPACE, LIMITS).unwrap();
            store
                .upsert("sig", record(&snippet("other", None, "forged")).unwrap())
                .unwrap();
        }
        let library = Library::new(Some(Arc::new(storage)));
        library.load();
        assert!(library.stored().is_empty());
    }
}
