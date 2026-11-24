//! Each package's `storage` capability: a `dupin-storage` namespace of its own, opened with the
//! quota and retention its declaration names, bounded by Dupin's maxima.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
/// One record's JSON and its blobs, which must cross to the guest whole.
pub(crate) const MAX_ITEM_BYTES: u64 = 1024 * 1024;
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
        max_item_bytes: quota.min(MAX_ITEM_BYTES),
    }
}

type ErrorSink = Arc<dyn Fn(&str, &Error) + Send + Sync>;

/// Packages' namespaces under one [`dupin_storage::Storage`], each opened on first use and kept
/// open, with a sweeper applying its retention while nothing is written.
pub struct PackageStorage {
    storage: dupin_storage::Storage,
    open: Mutex<HashMap<String, Opened>>,
    on_error: ErrorSink,
}

struct Opened {
    store: Arc<Store>,
    limits: Limits,
    _sweeper: Sweeper,
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
    fn open(&self, namespace: &str, limits: Limits) -> Result<Arc<Store>, Failure> {
        let mut open = self.open.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(opened) = open.get_mut(namespace) {
            if opened.limits != limits {
                opened.store.set_limits(limits).map_err(failure)?;
                opened.limits = limits;
            } else {
                // Reads never apply retention themselves, and the sweeper runs only hourly.
                opened.store.sweep().map_err(failure)?;
            }
            return Ok(opened.store.clone());
        }
        let store = Arc::new(self.storage.open(namespace, limits).map_err(failure)?);
        let (on_error, name) = (self.on_error.clone(), namespace.to_owned());
        let sweeper = Sweeper::spawn(&store, SWEEP_EVERY, move |error| on_error(&name, error))
            .map_err(|error| failure(Error::Io(error)))?;
        open.insert(
            namespace.to_owned(),
            Opened {
                store: store.clone(),
                limits,
                _sweeper: sweeper,
            },
        );
        Ok(store)
    }

    fn discard(&self, namespace: &str) -> Result<(), Failure> {
        let opened = self
            .open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(namespace);
        let cleared = match opened {
            Some(opened) => {
                opened.store.clear().map_err(failure)?;
                true
            }
            None => false,
        };
        match self.storage.discard(namespace) {
            // A call still holding the store keeps the namespace locked, but its data is gone.
            Err(Error::Busy) if cleared => Ok(()),
            result => result.map_err(failure),
        }
    }
}

/// What a guest learns of a storage error.
pub(crate) fn failure(error: Error) -> Failure {
    let code = match &error {
        // The bridge refuses a record past Dupin's own item bound before it reaches the store,
        // so a record too large for the store is one larger than the quota.
        Error::QuotaExceeded { .. } | Error::TooLarge { .. } => "quota-exceeded",
        Error::TooManyBlobs { .. } => "too-large",
        Error::NotFound | Error::Corrupt(_) => "not-found",
        Error::Key(_) | Error::WrongKey | Error::Busy => "unavailable",
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
        assert_eq!(widest.max_item_bytes, MAX_ITEM_BYTES);
        assert_eq!(widest.max_age, Some(DAY * MAX_RETENTION_DAYS));
        assert_eq!(limits(0, None).max_total_bytes, 1);
        assert_eq!(limits(0, None).max_age, None);
    }
}
