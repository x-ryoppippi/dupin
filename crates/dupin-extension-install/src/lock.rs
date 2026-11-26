//! The installed extensions list: what `dupin ext install` placed, pinned by commit and content
//! hash, with the scopes each package requests. The broker's grants, not the list, hold what
//! the user granted. The host keeps it, through a [`LockStore`]; nothing here writes it to a
//! file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::Error;

const VERSION: u32 = 1;
/// The most packages the list holds, so it stays small enough to hand between processes whole.
pub const MAX_PACKAGES: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    version: u32,
    /// By package name, which is also the folder name in the extensions directory.
    #[serde(default)]
    pub packages: BTreeMap<String, Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// `github:owner/repo`, a Git URL, or an absolute folder path, without a ref.
    pub source: String,
    /// The tag or branch the commit was resolved from, if any.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    /// The resolved commit SHA; absent for a folder source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub content_hash: String,
    /// RFC 3339, UTC.
    pub installed_at: String,
    pub version: String,
    pub keyword: String,
    /// The scopes the package's manifest requests.
    #[serde(default)]
    pub requested: Vec<String>,
}

impl Lock {
    pub fn new() -> Self {
        Self {
            version: VERSION,
            packages: BTreeMap::new(),
        }
    }

    /// Checks a list handed over from elsewhere: this version, package names that are folder
    /// names, and no more than [`MAX_PACKAGES`].
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != VERSION {
            return Err(Error::new(
                "invalid-lock",
                format!(
                    "the installed extensions list has version {}, which this Dupin doesn't read",
                    self.version
                ),
            ));
        }
        if self.packages.len() > MAX_PACKAGES {
            return Err(Error::new(
                "too-many-extensions",
                format!("no more than {MAX_PACKAGES} extensions can be installed"),
            )
            .hint("Remove one with `dupin ext remove` first."));
        }
        if let Some(name) = self
            .packages
            .keys()
            .find(|name| !crate::manifest::valid_name(name))
        {
            return Err(Error::new(
                "invalid-lock",
                format!("the installed extensions list names {name:?}, which isn't a package name"),
            ));
        }
        Ok(())
    }
}

/// Where the installed extensions list is kept. Reading one that exists but can't be read is
/// an error, never an empty list: the launcher keeps every listed folder off while it stands.
pub trait LockStore: Send + Sync {
    fn read(&self) -> Result<Lock, Error>;
    /// Replaces the list with `lock`.
    fn write(&self, lock: &Lock) -> Result<(), Error>;
}

impl<T: LockStore + ?Sized> LockStore for &T {
    fn read(&self) -> Result<Lock, Error> {
        (**self).read()
    }

    fn write(&self, lock: &Lock) -> Result<(), Error> {
        (**self).write(lock)
    }
}

impl<T: LockStore + ?Sized> LockStore for Arc<T> {
    fn read(&self) -> Result<Lock, Error> {
        (**self).read()
    }

    fn write(&self, lock: &Lock) -> Result<(), Error> {
        (**self).write(lock)
    }
}

#[derive(Debug)]
struct Memory {
    lock: Lock,
    unreadable: Option<Error>,
    unwritable: Option<Error>,
}

/// A list kept in memory, for tests and fixtures. Clones share it.
#[derive(Clone, Debug)]
pub struct MemoryLock(Arc<Mutex<Memory>>);

impl Default for MemoryLock {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Memory {
            lock: Lock::new(),
            unreadable: None,
            unwritable: None,
        })))
    }
}

impl MemoryLock {
    fn memory(&self) -> MutexGuard<'_, Memory> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes every read fail with `error` while it is set.
    pub fn set_unreadable(&self, error: Option<Error>) {
        self.memory().unreadable = error;
    }

    /// Makes every write fail with `error` while it is set.
    pub fn set_unwritable(&self, error: Option<Error>) {
        self.memory().unwritable = error;
    }
}

impl LockStore for MemoryLock {
    fn read(&self) -> Result<Lock, Error> {
        let memory = self.memory();
        match &memory.unreadable {
            Some(error) => Err(error.clone()),
            None => Ok(memory.lock.clone()),
        }
    }

    fn write(&self, lock: &Lock) -> Result<(), Error> {
        lock.validate()?;
        let mut memory = self.memory();
        if let Some(error) = &memory.unwritable {
            return Err(error.clone());
        }
        memory.lock = lock.clone();
        Ok(())
    }
}

/// Serializes `dupin ext` commands that change the extensions directory or the list, by an
/// advisory lock on an empty file.
pub(crate) struct Guard {
    _file: std::fs::File,
}

impl Guard {
    pub(crate) fn acquire(path: &Path) -> Result<Self, Error> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|error| Error::io("io-error", format!("create {}", dir.display()), error))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(|error| Error::io("io-error", format!("open {}", path.display()), error))?;
        // Waits rather than failing fast: a child spawned elsewhere in this process can hold a
        // duplicate of an earlier guard's descriptor for a moment after that guard is dropped.
        file.lock()
            .map_err(|error| Error::io("io-error", format!("lock {}", path.display()), error))?;
        Ok(Self { _file: file })
    }
}

/// The guard file `dupin ext` commands take turns on, in `dir`.
pub fn guard_path(dir: &Path) -> PathBuf {
    dir.join("extensions.lock.guard")
}

pub(crate) fn now() -> String {
    rfc3339(SystemTime::now())
}

fn rfc3339(time: SystemTime) -> String {
    let seconds = time.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // Howard Hinnant's days-to-civil conversion.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timestamps_are_utc_rfc3339() {
        assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            rfc3339(UNIX_EPOCH + Duration::from_secs(1_795_000_000)),
            "2026-11-18T11:06:40Z"
        );
        assert_eq!(
            rfc3339(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00Z"
        );
    }

    fn entry() -> Entry {
        Entry {
            source: "github:ryo/word-count".into(),
            rev: Some("v1.0.0".into()),
            commit: Some("a".repeat(40)),
            content_hash: "blake3:00".into(),
            installed_at: now(),
            version: "1.0.0".into(),
            keyword: "wc".into(),
            requested: vec!["http:https://example.com".into()],
        }
    }

    #[test]
    fn the_list_round_trips_and_rejects_unknown_keys_versions_and_names() {
        let mut lock = Lock::new();
        lock.packages.insert("word-count".into(), entry());
        let json = serde_json::to_value(&lock).unwrap();
        assert_eq!(json["packages"]["word-count"]["ref"], "v1.0.0");
        assert_eq!(serde_json::from_value::<Lock>(json.clone()).unwrap(), lock);
        lock.validate().unwrap();

        let mut surprise = json.clone();
        surprise["packages"]["word-count"]["granted"] = serde_json::json!([]);
        assert!(serde_json::from_value::<Lock>(surprise).is_err());

        let mut newer = json;
        newer["version"] = 2.into();
        let newer: Lock = serde_json::from_value(newer).unwrap();
        assert_eq!(newer.validate().unwrap_err().code, "invalid-lock");

        let mut named = Lock::new();
        named.packages.insert("../escape".into(), entry());
        assert_eq!(named.validate().unwrap_err().code, "invalid-lock");

        let mut full = Lock::new();
        for n in 0..=MAX_PACKAGES {
            full.packages.insert(format!("p{n}"), entry());
        }
        let store = MemoryLock::default();
        assert_eq!(store.write(&full).unwrap_err().code, "too-many-extensions");
        assert_eq!(store.read().unwrap(), Lock::new());
    }

    #[test]
    fn one_command_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = guard_path(dir.path());
        let held = Guard::acquire(&path).unwrap();
        let (acquired, waiting) = std::sync::mpsc::channel();
        let second = path.clone();
        let thread = std::thread::spawn(move || {
            let _guard = Guard::acquire(&second).unwrap();
            acquired.send(()).unwrap();
        });
        assert!(waiting.recv_timeout(Duration::from_millis(200)).is_err());
        drop(held);
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        thread.join().unwrap();
    }
}
