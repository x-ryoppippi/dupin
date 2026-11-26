//! The installed extensions list `dupin ext install` keeps, in encrypted storage: one record per
//! package upserted under its name, in the `extensions` namespace for a package installed from
//! Git and in `extensions-local` for one copied from a folder, whose path means nothing on
//! another Mac ([sync design](../../../../docs/design/sync.md#extensions)). A record holds the
//! source, so neither is written in the clear.
//!
//! The running launcher owns both namespaces for its lifetime, through [`shared`], and checks
//! installed folders against them. `dupin ext` opens them only while the launcher isn't
//! running; while it is, they are busy and the command asks the launcher over the control
//! socket instead. Every call blocks on the disk, and on the Keychain the first time, so it
//! runs on the extensions watcher's thread, a control worker or the command itself.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use dupin_extension_install::{Entry, Lock, LockStore, MAX_PACKAGES};
use dupin_storage::{Limits, NewRecord, Storage, Store};
use serde_json::Value;

pub const GIT_NAMESPACE: &str = "extensions";
pub const LOCAL_NAMESPACE: &str = "extensions-local";
/// What Settings calls the store when it can't be opened.
const NAME: &str = "Installed extensions list";

/// Each namespace stays small enough that the whole list crosses the control socket in one
/// frame. Writes refuse rather than evict: an entry evicted would let its folder run unchecked.
const LIMITS: Limits = Limits {
    max_age: None,
    max_entries: MAX_PACKAGES,
    max_total_bytes: 192 * 1024,
    max_item_bytes: 16 * 1024,
};

/// Why the list couldn't be read or written.
#[derive(Debug)]
pub enum Failure {
    /// Another process has it open: the launcher, or another `dupin ext` command.
    Busy,
    Other(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => f.write_str("the installed extensions list is open in another Dupin process"),
            Self::Other(problem) => f.write_str(problem),
        }
    }
}

impl From<dupin_storage::Error> for Failure {
    fn from(error: dupin_storage::Error) -> Self {
        match error {
            dupin_storage::Error::Busy => Self::Busy,
            error => Self::Other(error.to_string()),
        }
    }
}

struct Open {
    git: Store,
    local: Store,
    lock: Lock,
}

impl Open {
    fn namespace(&self, entry: &Entry) -> &Store {
        if entry.commit.is_some() {
            &self.git
        } else {
            &self.local
        }
    }
}

pub struct Installed {
    storage: Option<Arc<Storage>>,
    open: Mutex<Option<Open>>,
    /// Whether Settings hears whether it opened: only the launcher's own list is shown.
    reports: bool,
}

/// The launcher's list, over the data directory's storage; none in unit tests and headless
/// runs, whose extensions folder holds no installed packages, or without a data directory.
pub fn shared() -> Option<&'static Installed> {
    static INSTALLED: OnceLock<Option<Installed>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| {
            if cfg!(test) || rune::tooling::is_headless() {
                return None;
            }
            crate::extensions::access::shared_storage().map(|storage| Installed {
                reports: true,
                ..Installed::new(Some(storage))
            })
        })
        .as_ref()
}

impl Installed {
    pub fn new(storage: Option<Arc<Storage>>) -> Self {
        Self {
            storage,
            open: Mutex::new(None),
            reports: false,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Open>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn read(&self) -> Result<Lock, Failure> {
        let mut open = self.lock();
        Ok(match self.namespaces(&mut open, false)? {
            Some(open) => open.lock.clone(),
            None => Lock::new(),
        })
    }

    /// Replaces the list with `lock`, writing only the entries that changed. An entry written
    /// before a later one failed stays written, and is what the next read finds.
    pub fn write(&self, lock: &Lock) -> Result<(), Failure> {
        lock.validate()
            .map_err(|error| Failure::Other(error.to_string()))?;
        let mut open = self.lock();
        let open = self
            .namespaces(&mut open, true)?
            .ok_or_else(|| Failure::Other("the installed extensions list couldn't be opened".into()))?;
        let gone: Vec<_> = open
            .lock
            .packages
            .keys()
            .filter(|name| !lock.packages.contains_key(*name))
            .cloned()
            .collect();
        for name in gone {
            let entry = &open.lock.packages[&name];
            let namespace = open.namespace(entry);
            namespace.delete(&namespace.key_id(&name))?;
            open.lock.packages.remove(&name);
        }
        for (name, entry) in &lock.packages {
            let previous = open.lock.packages.get(name);
            if previous == Some(entry) {
                continue;
            }
            let namespace = open.namespace(entry);
            namespace.upsert_within_limits(
                name,
                NewRecord {
                    data: record(name, entry)?,
                    blobs: Vec::new(),
                },
            )?;
            if let Some(previous) = previous
                && !std::ptr::eq(open.namespace(previous), namespace)
            {
                let other = open.namespace(previous);
                other.delete(&other.key_id(name))?;
            }
            open.lock.packages.insert(name.clone(), entry.clone());
        }
        Ok(())
    }

    /// Both namespaces, opened and read on first use, and again after that failed, as a
    /// Keychain that was locked may have been unlocked since. With nothing ever installed and
    /// `create` false, it opens nothing, so no key is read.
    fn namespaces<'a>(
        &self,
        open: &'a mut Option<Open>,
        create: bool,
    ) -> Result<Option<&'a mut Open>, Failure> {
        if open.is_none() {
            let storage = self.storage.as_ref().ok_or_else(|| {
                Failure::Other("the installed extensions list needs XDG_DATA_HOME or HOME".into())
            })?;
            if !create && !storage.exists(GIT_NAMESPACE) && !storage.exists(LOCAL_NAMESPACE) {
                return Ok(None);
            }
            let opened = read(storage);
            if self.reports {
                let problem = opened.as_ref().err().map(ToString::to_string);
                crate::stored::report(NAME, problem.as_deref());
            }

            *open = Some(opened?);
        }
        Ok(open.as_mut())
    }
}

fn record(name: &str, entry: &Entry) -> Result<Value, Failure> {
    let mut data = serde_json::to_value(entry).map_err(|error| Failure::Other(error.to_string()))?;
    data["name"] = name.into();
    Ok(data)
}

fn read(storage: &Storage) -> Result<Open, Failure> {
    let git = storage.open(GIT_NAMESPACE, LIMITS)?;
    let local = storage.open(LOCAL_NAMESPACE, LIMITS)?;
    let mut lock = Lock::new();
    for (store, pinned) in [(&git, true), (&local, false)] {
        let report = store.open_report();
        if report.skipped_records > 0 {
            return Err(Failure::Other(
                "some entries were saved with another key or by a newer Dupin".into(),
            ));
        }
        if report.corrupt_records > 0 {
            crate::log::warn("installed extensions: dropped an entry that failed authentication");
        }
        let mut before = None;
        loop {
            let page = store.list(before.as_ref(), 200)?;
            for record in page.records {
                let mut data = record.data;
                let name = data
                    .as_object_mut()
                    .and_then(|data| data.remove("name"))
                    .and_then(|name| name.as_str().map(str::to_owned));
                // A record is stored under its package's name, so one naming another was moved
                // or forged and is left out.
                let Some(name) = name.filter(|name| store.key_id(name) == record.id) else {
                    crate::log::warn("installed extensions: left out an entry saved under another name");
                    continue;
                };
                let entry: Entry = serde_json::from_value(data).map_err(|error| {
                    Failure::Other(format!("the entry for {name} can't be read: {error}"))
                })?;
                if entry.commit.is_some() != pinned {
                    return Err(Failure::Other(format!(
                        "the entry for {name} is in the wrong namespace"
                    )));
                }
                if lock.packages.insert(name.clone(), entry).is_some() {
                    return Err(Failure::Other(format!("{name} is listed twice")));
                }
            }
            match page.next {
                Some(next) => before = Some(next),
                None => break,
            }
        }
    }
    Ok(Open { git, local, lock })
}

fn error(failure: Failure) -> dupin_extension_install::Error {
    match failure {
        Failure::Busy => dupin_extension_install::Error::new("busy", failure.to_string()),
        Failure::Other(problem) => dupin_extension_install::Error::new(
            "invalid-lock",
            format!("the installed extensions list couldn't be used: {problem}"),
        ),
    }
}

impl LockStore for Installed {
    fn read(&self) -> Result<Lock, dupin_extension_install::Error> {
        Installed::read(self).map_err(error)
    }

    fn write(&self, lock: &Lock) -> Result<(), dupin_extension_install::Error> {
        Installed::write(self, lock).map_err(error)
    }
}

/// Asks the running launcher, as [`crate::control::exchange`] does; tests answer for it.
pub type Launcher = Box<dyn Fn(crate::control::Command) -> Result<Value, String> + Send + Sync>;

/// The list as `dupin ext` reaches it: opened for each read or write and closed again, so the
/// launcher, if it starts meanwhile, can take it; or, while the launcher has it, through the
/// launcher.
pub struct Command {
    pub storage: Option<Arc<Storage>>,
    pub launcher: Launcher,
}

impl LockStore for Command {
    fn read(&self) -> Result<Lock, dupin_extension_install::Error> {
        match Installed::new(self.storage.clone()).read() {
            Err(Failure::Busy) => {
                let lock =
                    (self.launcher)(crate::control::Command::ExtensionLockGet {}).map_err(from_launcher)?;
                let lock: Lock =
                    serde_json::from_value(lock).map_err(|error| from_launcher(error.to_string()))?;
                lock.validate()?;
                Ok(lock)
            }
            read => read.map_err(error),
        }
    }

    fn write(&self, lock: &Lock) -> Result<(), dupin_extension_install::Error> {
        match Installed::new(self.storage.clone()).write(lock) {
            Err(Failure::Busy) => (self.launcher)(crate::control::Command::ExtensionLockSet {
                lock: Box::new(lock.clone()),
            })
            .map(drop)
            .map_err(from_launcher),
            written => written.map_err(error),
        }
    }
}

fn from_launcher(problem: String) -> dupin_extension_install::Error {
    dupin_extension_install::Error::new(
        "busy",
        format!(
            "the installed extensions list is open in another Dupin process, which couldn't use it: {problem}"
        ),
    )
    .hint("If another `dupin ext` command is running, wait for it to finish and try again.")
}

/// What the launcher answers a `dupin ext` command that found `installed` busy.
pub fn answer(installed: Option<&Installed>, command: &crate::control::Command) -> Result<Value, String> {
    let installed = installed.ok_or("this Dupin keeps no installed extensions list")?;
    match command {
        crate::control::Command::ExtensionLockGet {} => {
            let lock = installed.read().map_err(|failure| failure.to_string())?;
            serde_json::to_value(lock).map_err(|error| error.to_string())
        }
        crate::control::Command::ExtensionLockSet { lock } => installed
            .write(lock)
            .map(|()| Value::Null)
            .map_err(|failure| failure.to_string()),
        _ => Err("not a request about the installed extensions list".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn entry(source: &str, commit: Option<&str>) -> Entry {
        Entry {
            source: source.into(),
            rev: commit.map(|_| "v1.0.0".into()),
            commit: commit.map(str::to_owned),
            content_hash: "blake3:00".into(),
            installed_at: "2026-01-01T00:00:00Z".into(),
            version: "1.0.0".into(),
            keyword: "wc".into(),
            requested: vec!["files:notes".into()],
        }
    }

    const PRIVATE: &str = "github:someone/private-repo";
    const FOLDER: &str = "/Users/x/Secret Folder/notes";

    fn listed() -> Lock {
        let mut lock = Lock::new();
        lock.packages
            .insert("word-count".into(), entry(PRIVATE, Some(&"a".repeat(40))));
        lock.packages.insert("notes".into(), entry(FOLDER, None));
        lock
    }

    #[test]
    fn the_list_is_sealed_split_by_source_and_outlives_the_launcher() {
        let root = tempfile::tempdir().unwrap();
        let key = DataKey::generate().unwrap();
        let installed = Installed::new(storage(root.path(), &key));
        assert_eq!(installed.read().unwrap(), Lock::new());
        assert!(
            !root.path().join(GIT_NAMESPACE).exists() && !root.path().join(LOCAL_NAMESPACE).exists(),
            "nothing is opened, and no key read, before the first install"
        );
        installed.write(&listed()).unwrap();
        for (path, bytes) in files(root.path()) {
            let text = String::from_utf8_lossy(&bytes);
            for plain in [PRIVATE, "private-repo", "Secret Folder", "word-count", "blake3"] {
                assert!(
                    !text.contains(plain),
                    "{plain} is in {} in the clear",
                    path.display()
                );
            }
        }
        let count = |namespace: &str| {
            std::fs::read_dir(root.path().join(namespace).join("records"))
                .unwrap()
                .count()
        };
        assert_eq!((count(GIT_NAMESPACE), count(LOCAL_NAMESPACE)), (1, 1));
        drop(installed);

        let reopened = Installed::new(storage(root.path(), &key));
        assert_eq!(reopened.read().unwrap(), listed());
        let mut fewer = listed();
        fewer.packages.remove("notes");
        let mut moved = entry("github:someone/notes", Some(&"b".repeat(40)));
        moved.keyword = "nt".into();
        fewer.packages.insert("notes-git".into(), moved);
        reopened.write(&fewer).unwrap();
        drop(reopened);
        assert_eq!(Installed::new(storage(root.path(), &key)).read().unwrap(), fewer);
        assert_eq!((count(GIT_NAMESPACE), count(LOCAL_NAMESPACE)), (2, 0));
    }

    #[test]
    fn a_wrong_or_missing_key_fails_closed_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let key = DataKey::generate().unwrap();
        Installed::new(storage(root.path(), &key))
            .write(&listed())
            .unwrap();
        let before = files(root.path());

        let wrong = Installed::new(storage(root.path(), &DataKey::generate().unwrap()));
        assert!(matches!(wrong.read(), Err(Failure::Other(_))));
        assert!(wrong.write(&Lock::new()).is_err());
        let refusing = Installed::new(Some(Arc::new(Storage::new(root.path(), NoKey))));
        assert!(
            refusing
                .read()
                .unwrap_err()
                .to_string()
                .contains("the Keychain is locked")
        );
        assert!(refusing.write(&Lock::new()).is_err());
        let empty = tempfile::tempdir().unwrap();
        let refusing = Installed::new(Some(Arc::new(Storage::new(empty.path(), NoKey))));
        assert!(
            refusing.write(&listed()).is_err(),
            "a first write needs the key too"
        );

        assert_eq!(files(root.path()), before, "nothing written");
        assert_eq!(
            Installed::new(storage(root.path(), &key)).read().unwrap(),
            listed()
        );
    }

    #[test]
    fn a_tampered_entry_is_dropped_a_moved_one_left_out_and_a_second_owner_is_busy() {
        let root = tempfile::tempdir().unwrap();
        let key = DataKey::generate().unwrap();
        Installed::new(storage(root.path(), &key))
            .write(&listed())
            .unwrap();
        let [record] = std::fs::read_dir(root.path().join(GIT_NAMESPACE).join("records"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let mut bytes = std::fs::read(&record).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(&record, bytes).unwrap();
        let read = Installed::new(storage(root.path(), &key)).read().unwrap();
        assert_eq!(
            read.packages.keys().collect::<Vec<_>>(),
            ["notes"],
            "the tampered entry is dropped"
        );

        let storage = storage(root.path(), &key).unwrap();
        {
            let local = storage.open(LOCAL_NAMESPACE, LIMITS).unwrap();
            local
                .upsert(
                    "listed",
                    NewRecord {
                        data: record_of("notes", &entry("/elsewhere", None)),
                        blobs: Vec::new(),
                    },
                )
                .unwrap();
        }
        let owner = Installed::new(Some(storage.clone()));
        assert_eq!(
            owner.read().unwrap().packages["notes"].source,
            FOLDER,
            "a record naming another package is left out"
        );
        let second = Installed::new(Some(storage));
        assert!(matches!(second.read(), Err(Failure::Busy)));
        assert!(matches!(second.write(&Lock::new()), Err(Failure::Busy)));
        drop(owner);
        assert!(second.read().is_ok());
    }

    #[test]
    fn a_command_asks_the_launcher_while_it_holds_the_list() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let root = tempfile::tempdir().unwrap();
        let key = DataKey::generate().unwrap();
        let running = Arc::new(Mutex::new(Some(Installed::new(storage(root.path(), &key)))));
        let launched = || running.lock().unwrap();
        launched().as_ref().unwrap().write(&listed()).unwrap();
        let asked = Arc::new(AtomicUsize::new(0));
        let command = Command {
            storage: storage(root.path(), &key),
            launcher: Box::new({
                let (running, asked) = (running.clone(), asked.clone());
                move |command| {
                    asked.fetch_add(1, Ordering::SeqCst);
                    answer(running.lock().unwrap().as_ref(), &command)
                }
            }),
        };
        assert_eq!(LockStore::read(&command).unwrap(), listed());
        LockStore::write(&command, &Lock::new()).unwrap();
        assert_eq!(asked.load(Ordering::SeqCst), 2);
        assert_eq!(launched().as_ref().unwrap().read().unwrap(), Lock::new());

        *launched() = None;
        LockStore::write(&command, &listed()).unwrap();
        assert_eq!(LockStore::read(&command).unwrap(), listed());
        assert_eq!(
            asked.load(Ordering::SeqCst),
            2,
            "with the launcher gone it opens the list itself"
        );

        let held = Installed::new(storage(root.path(), &key));
        held.read().unwrap();
        let unanswered = Command {
            storage: storage(root.path(), &key),
            launcher: Box::new(|_| Err("Dupin is not running".into())),
        };
        assert_eq!(LockStore::read(&unanswered).unwrap_err().code, "busy");
    }

    fn record_of(name: &str, entry: &Entry) -> Value {
        record(name, entry).unwrap()
    }

    struct NoKey;

    impl dupin_storage::KeyProvider for NoKey {
        fn data_key(&self) -> Result<DataKey, dupin_storage::Error> {
            Err(dupin_storage::Error::Key("the Keychain is locked".into()))
        }
    }
}
