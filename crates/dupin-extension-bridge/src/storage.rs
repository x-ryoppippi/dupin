//! Each package's `storage` capability: a `dupin-storage` namespace of its own, opened with the
//! quota and retention its declaration names, bounded by Dupin's maxima.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use dupin_broker::PackageId;
use dupin_storage::{Error, Limits, Store, Sweeper};
use sha2::{Digest, Sha256};

use crate::services::Failure;

/// Every extension namespace starts with this, and no native feature's does, so neither can
/// open the other's data.
const NAMESPACE_PREFIX: &str = "ext-";
/// The most one package may keep, whatever it declares.
const MAX_QUOTA_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RETENTION_DAYS: u32 = 3650;
const MAX_ENTRIES: usize = 10_000;
/// One record's JSON and the blobs the guest sends with it, which cross from the guest whole.
pub(crate) const MAX_ITEM_BYTES: u64 = 1024 * 1024;
/// One record with a copied image the host stores for it, which never crosses to the guest.
pub(crate) const MAX_STORED_ITEM_BYTES: u64 =
    MAX_ITEM_BYTES + dupin_host_services::clipboard::MAX_WATCHED_IMAGE_BYTES as u64;
const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// Where an installed package came from. A user's copy of a bundled package, even an
/// unmodified one, keeps its data apart from the bundled package's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Origin {
    #[default]
    User,
    Bundled,
}

/// The namespace `package` stores into. Package IDs are up to 512 bytes of almost anything,
/// and a namespace is a short file name, so it is named by a hash of the ID and its origin.
pub fn namespace(package: &PackageId, origin: Origin) -> String {
    let tag = match origin {
        Origin::User => "u",
        Origin::Bundled => "b",
    };
    let mut hash = Sha256::new();
    hash.update(b"dupin extension storage\0");
    hash.update(tag.as_bytes());
    hash.update(b"\0");
    hash.update(package.as_str().as_bytes());
    let digest = hash.finalize();
    let hex: String = digest[..20].iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{NAMESPACE_PREFIX}{tag}-{hex}")
}

/// What a package's declared quota and retention allow, within Dupin's maxima.
pub(crate) fn limits(quota_bytes: u64, retention_days: Option<u32>) -> Limits {
    let quota = quota_bytes.clamp(1, MAX_QUOTA_BYTES);
    Limits {
        max_age: retention_days.map(|days| DAY * days.clamp(1, MAX_RETENTION_DAYS)),
        max_entries: MAX_ENTRIES,
        max_total_bytes: quota,
        max_item_bytes: quota.min(MAX_STORED_ITEM_BYTES),
    }
}

type ErrorSink = Arc<dyn Fn(&str, &Error) + Send + Sync>;

/// One package's open namespace. Deleting the data closes it for good, so a call that opened
/// it before the deletion fails instead of writing into a namespace nothing lists any more.
pub struct Namespace {
    store: RwLock<Option<Arc<Store>>>,
}

impl Namespace {
    /// Runs `op` on the store; a deletion waits until it returns.
    pub fn with<R>(&self, op: impl FnOnce(&Store) -> Result<R, Error>) -> Result<R, Failure> {
        let store = self.store.read().unwrap_or_else(PoisonError::into_inner);
        let store = store
            .as_ref()
            .ok_or_else(|| Failure::new("unavailable", "the package's stored data was deleted"))?;
        op(store).map_err(failure)
    }
}

/// Packages' namespaces under one [`dupin_storage::Storage`], each opened on first use and kept
/// open, with a sweeper applying its retention while nothing is written.
pub struct PackageStorage {
    storage: dupin_storage::Storage,
    open: Mutex<HashMap<String, Opened>>,
    on_error: ErrorSink,
}

struct Opened {
    namespace: Arc<Namespace>,
    limits: Limits,
    sweeper: Sweeper,
}

impl PackageStorage {
    pub fn new(storage: dupin_storage::Storage) -> Self {
        Self {
            storage,
            open: Mutex::default(),
            on_error: Arc::new(|_, _| {}),
        }
    }

    /// Where a background sweep's failure goes, with the namespace it was in.
    pub fn on_error(mut self, report: impl Fn(&str, &Error) + Send + Sync + 'static) -> Self {
        self.on_error = Arc::new(report);
        self
    }
}

impl crate::services::Storage for PackageStorage {
    fn open(&self, namespace: &str, limits: Limits) -> Result<Arc<Namespace>, Failure> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(opened) = open.get_mut(namespace) {
            if opened.limits != limits {
                opened.namespace.with(|store| store.set_limits(limits))?;
                opened.limits = limits;
            } else {
                // Reads never apply retention themselves, and the sweeper runs only hourly.
                opened.namespace.with(Store::sweep)?;
            }
            return Ok(opened.namespace.clone());
        }
        let store = Arc::new(self.storage.open(namespace, limits).map_err(failure)?);
        let (on_error, name) = (self.on_error.clone(), namespace.to_owned());
        let sweeper = Sweeper::spawn(&store, SWEEP_EVERY, move |error| on_error(&name, error))
            .map_err(|error| failure(Error::Io(error)))?;
        let opened = Arc::new(Namespace {
            store: RwLock::new(Some(store)),
        });
        open.insert(
            namespace.to_owned(),
            Opened {
                namespace: opened.clone(),
                limits,
                sweeper,
            },
        );
        Ok(opened)
    }

    fn discard(&self, namespace: &str) -> Result<(), Failure> {
        // Held to the end, so no call reopens the namespace while it is being deleted.
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(Opened {
            namespace: opened,
            sweeper,
            ..
        }) = open.remove(namespace)
        {
            drop(sweeper);
            // The only strong reference once the sweeper has stopped, so dropping it unlocks
            // the namespace for the deletion below.
            drop(
                opened
                    .store
                    .write()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take(),
            );
        }
        self.storage.discard(namespace).map_err(failure)
    }
}

/// What a guest learns of a storage error.
fn failure(error: Error) -> Failure {
    let code = match &error {
        // The bridge refuses a record past Dupin's own item bound before it reaches the store,
        // so a record too large for the store is one larger than the quota.
        Error::QuotaExceeded { .. } | Error::TooLarge { .. } => "quota-exceeded",
        Error::TooManyBlobs { .. } => "too-large",
        Error::NotFound | Error::Corrupt(_) => "not-found",
        Error::Key(_) | Error::WrongKey | Error::NewerFormat | Error::Busy => "unavailable",
        Error::Io(_) | Error::InvalidNamespace(_) | Error::InvalidLimits(_) | Error::Json(_) => "failed",
    };
    Failure::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_are_file_names_apart_per_package_and_origin() {
        let weather = PackageId::new("weather").unwrap();
        let local = PackageId::new("local:/Users/me/My Extension/Ünïcode").unwrap();
        let names = [
            namespace(&weather, Origin::User),
            namespace(&weather, Origin::Bundled),
            namespace(&local, Origin::User),
            namespace(&local, Origin::Bundled),
        ];
        for (index, name) in names.iter().enumerate() {
            assert!(name.starts_with(NAMESPACE_PREFIX), "{name}");
            assert!(name.len() <= 64, "{name}");
            assert!(
                name.bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
                "{name}"
            );
            assert!(!names[..index].contains(name), "{name} collides");
        }
        assert_eq!(names[0], namespace(&weather, Origin::User), "stable");
    }

    #[test]
    fn limits_follow_the_declaration_within_dupins_maxima() {
        assert_eq!(
            limits(1024, Some(30)),
            Limits {
                max_age: Some(DAY * 30),
                max_entries: MAX_ENTRIES,
                max_total_bytes: 1024,
                max_item_bytes: 1024,
            }
        );
        let widest = limits(u64::MAX, Some(u32::MAX));
        assert_eq!(widest.max_total_bytes, MAX_QUOTA_BYTES);
        assert_eq!(widest.max_item_bytes, MAX_STORED_ITEM_BYTES);
        assert_eq!(widest.max_age, Some(DAY * MAX_RETENTION_DAYS));
        assert_eq!(limits(0, None).max_total_bytes, 1);
        assert_eq!(limits(0, None).max_age, None);
    }

    fn package_storage(dir: &std::path::Path) -> Arc<PackageStorage> {
        Arc::new(PackageStorage::new(dupin_storage::Storage::new(
            dir,
            dupin_storage::MemoryKey::random(),
        )))
    }

    fn record(text: &str) -> dupin_storage::NewRecord {
        dupin_storage::NewRecord {
            data: serde_json::json!(text),
            blobs: Vec::new(),
        }
    }

    #[test]
    fn deleting_waits_for_a_running_write_and_removes_what_it_wrote() {
        use crate::services::Storage as _;
        let dir = tempfile::tempdir().unwrap();
        let storage = package_storage(dir.path());
        let name = namespace(&PackageId::new("notes").unwrap(), Origin::User);
        let opened = storage.open(&name, limits(1024, None)).unwrap();

        let (started, wait_started) = std::sync::mpsc::channel();
        let (go, wait_go) = std::sync::mpsc::channel::<()>();
        let writer = std::thread::spawn({
            let opened = opened.clone();
            move || {
                opened.with(|store| {
                    started.send(()).unwrap();
                    wait_go.recv().unwrap();
                    store.put_within_limits(record("written while deleting"))
                })
            }
        });
        wait_started.recv().unwrap();
        let deleter = std::thread::spawn({
            let (storage, name) = (storage.clone(), name.clone());
            move || storage.discard(&name)
        });
        std::thread::sleep(Duration::from_millis(100));
        assert!(!deleter.is_finished(), "the deletion waits for the write");
        go.send(()).unwrap();
        writer.join().unwrap().unwrap();
        deleter.join().unwrap().unwrap();
        assert!(
            !dir.path().join(&name).exists(),
            "the write went with the namespace"
        );
    }

    #[test]
    fn a_namespace_opened_before_its_deletion_refuses_later_calls() {
        use crate::services::Storage as _;
        let dir = tempfile::tempdir().unwrap();
        let storage = package_storage(dir.path());
        let name = namespace(&PackageId::new("notes").unwrap(), Origin::User);
        let stale = storage.open(&name, limits(1024, None)).unwrap();
        stale
            .with(|store| store.put_within_limits(record("old")))
            .unwrap();

        storage.discard(&name).unwrap();
        assert!(!dir.path().join(&name).exists());
        let refused = stale
            .with(|store| store.put_within_limits(record("late")))
            .unwrap_err();
        assert_eq!(refused.code, "unavailable");
        assert!(!dir.path().join(&name).exists(), "nothing was written");

        let fresh = storage.open(&name, limits(1024, None)).unwrap();
        assert_eq!(fresh.with(|store| Ok(store.usage().entries)).unwrap(), 0);
    }
}
