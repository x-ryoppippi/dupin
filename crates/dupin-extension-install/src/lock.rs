//! `extensions.lock` in Dupin's data directory: what `dupin ext install` placed, pinned by
//! commit and content hash, with the scopes the user granted.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::Error;

pub const LOCK_FILE: &str = "extensions.lock";
const VERSION: u32 = 1;
const MAX_LOCK_BYTES: u64 = 1024 * 1024;

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
    /// The scopes the user granted; always a subset of `requested`.
    #[serde(default)]
    pub granted: Vec<String>,
    /// Scopes an update added that the user hasn't consented to yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_consent: Vec<String>,
}

impl Lock {
    /// The lock at `path`, or an empty one when there is none yet.
    pub fn read(path: &Path) -> Result<Self, Error> {
        let source = match std::fs::metadata(path) {
            Ok(metadata) if metadata.len() > MAX_LOCK_BYTES => {
                return Err(Error::new("invalid-lock", "is larger than 1 MiB").file(LOCK_FILE));
            }
            Ok(_) => std::fs::read_to_string(path)
                .map_err(|error| Error::io("invalid-lock", format!("read {}", path.display()), error))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(error) => {
                return Err(Error::io(
                    "invalid-lock",
                    format!("read {}", path.display()),
                    error,
                ));
            }
        };
        let lock: Self = toml::from_str(&source).map_err(|error| {
            Error::new("invalid-lock", error.message().trim_end().to_owned())
                .file(LOCK_FILE)
                .hint("Restore extensions.lock, or delete it and reinstall your extensions.")
        })?;
        if lock.version != VERSION {
            return Err(Error::new(
                "invalid-lock",
                format!("has version {}, which this Dupin doesn't read", lock.version),
            )
            .file(LOCK_FILE));
        }
        Ok(lock)
    }

    pub fn new() -> Self {
        Self {
            version: VERSION,
            packages: BTreeMap::new(),
        }
    }

    /// Replaces the file at `path` in one rename, so a reader sees the old lock or the new one.
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        let text = toml::to_string(self).map_err(|error| Error::new("io-error", error.to_string()))?;
        let text = format!(
            "# Written by `dupin ext`; change it with `dupin ext install`, `update` and `remove`.\n{text}"
        );
        let dir = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)
            .map_err(|error| Error::io("io-error", format!("create {}", dir.display()), error))?;
        let temporary = dir.join(format!(".{LOCK_FILE}.{}", std::process::id()));
        let written = std::fs::File::create(&temporary)
            .and_then(|mut file| {
                file.write_all(text.as_bytes())?;
                file.sync_all()
            })
            .and_then(|()| std::fs::rename(&temporary, path));
        written.map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            Error::io("io-error", format!("write {}", path.display()), error)
        })
    }
}

/// Serializes `dupin ext` commands that change the extensions directory or the lock.
pub(crate) struct Guard {
    _file: std::fs::File,
}

impl Guard {
    pub(crate) fn acquire(lock_path: &Path) -> Result<Self, Error> {
        let path = guard_path(lock_path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|error| Error::io("io-error", format!("create {}", dir.display()), error))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|error| Error::io("io-error", format!("open {}", path.display()), error))?;
        // Waits rather than failing fast: a child spawned elsewhere in this process can hold a
        // duplicate of an earlier guard's descriptor for a moment after that guard is dropped.
        file.lock()
            .map_err(|error| Error::io("io-error", format!("lock {}", path.display()), error))?;
        Ok(Self { _file: file })
    }
}

fn guard_path(lock_path: &Path) -> PathBuf {
    let mut name = lock_path.file_name().unwrap_or_default().to_owned();
    name.push(".guard");
    lock_path.with_file_name(name)
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

    #[test]
    fn the_lock_round_trips_and_rejects_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        assert_eq!(Lock::read(&path).unwrap(), Lock::new());
        let mut lock = Lock::new();
        lock.packages.insert(
            "word-count".into(),
            Entry {
                source: "github:ryo/word-count".into(),
                rev: Some("v1.0.0".into()),
                commit: Some("a".repeat(40)),
                content_hash: "blake3:00".into(),
                installed_at: now(),
                version: "1.0.0".into(),
                keyword: "wc".into(),
                requested: vec!["http:https://example.com".into()],
                granted: vec![],
                pending_consent: vec![],
            },
        );
        lock.write(&path).unwrap();
        assert_eq!(Lock::read(&path).unwrap(), lock);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[packages.word-count]"), "{text}");

        std::fs::write(&path, format!("{text}\nsurprise = 1\n")).unwrap();
        assert_eq!(Lock::read(&path).unwrap_err().code, "invalid-lock");
    }

    #[test]
    fn one_command_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
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
