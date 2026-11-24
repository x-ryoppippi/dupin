//! Checking installed folders against extensions.lock as the launcher loads them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::hash::{self, content_hash};
use crate::lock::Lock;

/// Remembers each folder's file list and verdict, so a rescan rehashes only folders whose files
/// or locked hash changed.
pub struct Integrity {
    lock_path: PathBuf,
    lock_stamp: Option<(u64, SystemTime)>,
    lock: Result<Lock, String>,
    verdicts: HashMap<PathBuf, Verdict>,
}

struct Verdict {
    files: Vec<(String, bool, u64, SystemTime)>,
    expected: String,
    problem: Option<String>,
}

impl Integrity {
    pub fn new(lock_path: PathBuf) -> Self {
        Self {
            lock_path,
            lock_stamp: None,
            lock: Ok(Lock::new()),
            verdicts: HashMap::new(),
        }
    }

    /// Why the extension in `folder` must stay off, or `None` when the lock doesn't list it or
    /// its contents still match. An unreadable lock keeps every folder off, so damaging the lock
    /// can't switch the check off.
    pub fn check(&mut self, folder: &Path) -> Option<String> {
        self.refresh_lock();
        let lock = match &self.lock {
            Ok(lock) => lock,
            Err(problem) => return Some(problem.clone()),
        };
        let name = folder.file_name()?.to_str()?;
        let expected = &lock.packages.get(name)?.content_hash;
        let files = match hash::files(folder) {
            Ok(files) => files
                .into_iter()
                .map(|file| (file.path, file.executable, file.len, file.modified))
                .collect(),
            Err(error) => return Some(tampered(&error.to_string())),
        };
        if let Some(verdict) = self.verdicts.get(folder)
            && verdict.files == files
            && &verdict.expected == expected
        {
            return verdict.problem.clone();
        }
        let problem = match content_hash(folder) {
            Ok(actual) if &actual == expected => None,
            Ok(_) => Some(tampered("its files differ from what was installed")),
            Err(error) => Some(tampered(&error.to_string())),
        };
        self.verdicts.insert(
            folder.to_owned(),
            Verdict {
                files,
                expected: expected.clone(),
                problem: problem.clone(),
            },
        );
        problem
    }

    fn refresh_lock(&mut self) {
        let stamp = std::fs::metadata(&self.lock_path)
            .ok()
            .and_then(|metadata| Some((metadata.len(), metadata.modified().ok()?)));
        if stamp.is_some() && stamp == self.lock_stamp {
            return;
        }
        self.lock_stamp = stamp;
        self.lock = Lock::read(&self.lock_path).map_err(|error| {
            format!(
                "disabled: {} ({error}). Fix or delete it, then reinstall with `dupin ext install`.",
                self.lock_path.display()
            )
        });
    }
}

fn tampered(reason: &str) -> String {
    format!(
        "disabled: {reason}, so it doesn't match extensions.lock. Run `dupin ext sync` to restore it, or `dupin ext remove` and install it again."
    )
}
