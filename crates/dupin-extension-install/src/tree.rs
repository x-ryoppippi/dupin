//! The files of a package on their way into the extensions directory, checked against the
//! limits before any of them is written.

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::Error;

pub const MAX_FILES: usize = 1024;
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_PATH_BYTES: usize = 512;
pub const MAX_DEPTH: usize = 16;

/// What a source lists before its contents are read, so limits fail fast.
pub(crate) struct Listing {
    files: usize,
    bytes: u64,
    /// Lowercased paths, because APFS is case-insensitive by default and two names that differ
    /// only in case would overwrite each other.
    files_folded: BTreeSet<String>,
    dirs_folded: BTreeSet<String>,
}

impl Listing {
    pub(crate) fn new() -> Self {
        Self {
            files: 0,
            bytes: 0,
            files_folded: BTreeSet::new(),
            dirs_folded: BTreeSet::new(),
        }
    }

    pub(crate) fn add(&mut self, path: &str, size: u64) -> Result<(), Error> {
        check_path(path)?;
        self.files += 1;
        if self.files > MAX_FILES {
            return Err(
                Error::new("package-too-large", format!("has more than {MAX_FILES} files"))
                    .hint("Ship only the built entry and the assets it needs."),
            );
        }
        if size > MAX_FILE_BYTES {
            return Err(Error::new(
                "package-too-large",
                format!("is larger than {} MiB", MAX_FILE_BYTES / 1024 / 1024),
            )
            .file(path));
        }
        self.bytes += size;
        if self.bytes > MAX_TOTAL_BYTES {
            return Err(Error::new(
                "package-too-large",
                format!("is larger than {} MiB in all", MAX_TOTAL_BYTES / 1024 / 1024),
            )
            .hint("Ship only the built entry and the assets it needs."));
        }
        let folded = path.to_lowercase();
        let mut parent = String::new();
        for part in folded.split('/').take(folded.matches('/').count()) {
            if !parent.is_empty() {
                parent.push('/');
            }
            parent.push_str(part);
            if self.files_folded.contains(&parent) {
                return Err(collision(path));
            }
            self.dirs_folded.insert(parent.clone());
        }
        if self.dirs_folded.contains(&folded) || !self.files_folded.insert(folded) {
            return Err(collision(path));
        }
        Ok(())
    }
}

fn collision(path: &str) -> Error {
    Error::new(
        "invalid-package-path",
        "clashes with another file whose name differs only in case",
    )
    .file(path)
}

/// A relative path of plain components, within the length and depth limits, with no `.git`.
pub(crate) fn check_path(path: &str) -> Result<(), Error> {
    let invalid = |message: &str| Error::new("invalid-package-path", message.to_owned()).file(path);
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err(invalid(&format!("must be 1–{MAX_PATH_BYTES} bytes")));
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() > MAX_DEPTH {
        return Err(invalid(&format!("is nested deeper than {MAX_DEPTH} folders")));
    }
    for part in parts {
        if part.is_empty() || part == "." || part == ".." || part.len() > 255 {
            return Err(invalid("must be a relative path inside the package"));
        }
        if part.chars().any(char::is_control) {
            return Err(invalid("must not contain control characters"));
        }
        if part.eq_ignore_ascii_case(".git") {
            return Err(invalid("must not contain a `.git` folder"));
        }
    }
    Ok(())
}

/// A hidden folder in the extensions directory that a package is written into before it is
/// renamed into place. The extensions scanner skips names that start with a dot.
pub(crate) struct Stage {
    pub(crate) root: PathBuf,
}

impl Stage {
    pub(crate) fn new(parent: &Path, purpose: &str) -> Result<Self, Error> {
        std::fs::create_dir_all(parent)
            .map_err(|error| Error::io("io-error", format!("create {}", parent.display()), error))?;
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = parent.join(format!(".dupin-{purpose}-{}-{n}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root)
                .map_err(|error| Error::io("io-error", format!("clear {}", root.display()), error))?;
        }
        std::fs::create_dir(&root)
            .map_err(|error| Error::io("io-error", format!("create {}", root.display()), error))?;
        Ok(Self { root })
    }

    pub(crate) fn package(&self) -> PathBuf {
        self.root.join("package")
    }

    pub(crate) fn write(&self, path: &str, bytes: &[u8], executable: bool) -> Result<(), Error> {
        check_path(path)?;
        let target = self.package().join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| Error::io("io-error", format!("create {}", parent.display()), error))?;
        }
        std::fs::write(&target, bytes)
            .and_then(|()| {
                std::fs::set_permissions(
                    &target,
                    std::fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
                )
            })
            .map_err(|error| Error::io("io-error", format!("write {}", target.display()), error))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_and_names_differ_by_more_than_case() {
        for bad in [
            "",
            "/abs",
            "a/../b",
            "./a",
            "a//b",
            ".git/config",
            "x/.GIT/hooks",
            "a\nb",
        ] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
        let deep = vec!["d"; MAX_DEPTH + 1].join("/");
        assert!(check_path(&deep).is_err());

        let mut listing = Listing::new();
        listing.add("main.js", 10).unwrap();
        listing.add("lib/a.js", 10).unwrap();
        assert!(listing.add("Main.js", 10).is_err());
        assert!(listing.add("LIB", 1).is_err());
        assert!(listing.add("main.js/x", 1).is_err());
        assert!(listing.add("big.bin", MAX_FILE_BYTES + 1).is_err());
    }

    #[test]
    fn totals_are_capped() {
        let mut listing = Listing::new();
        for n in 0..4 {
            listing.add(&format!("f{n}"), MAX_FILE_BYTES).unwrap();
        }
        assert_eq!(listing.add("one-more", 1).unwrap_err().code, "package-too-large");
        let mut many = Listing::new();
        for n in 0..MAX_FILES {
            many.add(&format!("f{n}"), 0).unwrap();
        }
        assert!(many.add("last", 0).is_err());
    }
}
