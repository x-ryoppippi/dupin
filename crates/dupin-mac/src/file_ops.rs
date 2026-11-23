//! File operations that create or rename entries beside an existing one. None of them
//! replaces an existing entry: each picks a free name, and the system call it ends in fails
//! rather than overwrite one that appeared in the meantime.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Command;

use dupin_core::file_name;
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSFileManager, NSString, NSURL};

/// Rename `path` to `name` in the same folder, returning the new path.
pub fn rename(path: &Path, name: &str) -> Result<PathBuf, String> {
    if let Some(problem) = file_name::problem(name) {
        return Err(problem.into());
    }
    let old = file_name_of(path)?;
    let destination = sibling(path, name)?;
    if old != name && old.to_lowercase() == name.to_lowercase() && same_entry(path, &destination) {
        // On a case-insensitive volume the new spelling "exists" because it is the source itself,
        // so rename(2) can't replace anything else. Between two hard links it does nothing.
        std::fs::rename(path, &destination).map_err(|error| error.to_string())?;
        return if listed(path) {
            Err(format!("“{name}” already exists."))
        } else {
            Ok(destination)
        };
    }
    autoreleasepool(|_| {
        NSFileManager::defaultManager()
            .moveItemAtURL_toURL_error(&file_url(path), &file_url(&destination))
            .map(|()| destination.clone())
            .map_err(|error| error.localizedDescription().to_string())
    })
}

/// Copy `path` beside itself as "name copy", "name copy 2", …, returning the copy's path.
pub fn duplicate(path: &Path) -> Result<PathBuf, String> {
    let name = file_name_of(path)?;
    claim(
        path,
        file_name::duplicate_names(name, has_extension(path)),
        |destination| {
            autoreleasepool(|_| {
                NSFileManager::defaultManager()
                    .copyItemAtURL_toURL_error(&file_url(path), &file_url(destination))
                    .map_err(|error| {
                        if error.domain().to_string() == "NSCocoaErrorDomain"
                            && error.code() == NS_FILE_WRITE_FILE_EXISTS_ERROR
                        {
                            Attempt::Taken
                        } else {
                            Attempt::Failed(error.localizedDescription().to_string())
                        }
                    })
            })
        },
    )
}

/// Zip `path` beside itself as "name.zip", "name 2.zip", …, as Finder's Compress does,
/// returning the archive's path.
pub fn compress(path: &Path) -> Result<PathBuf, String> {
    let name = file_name_of(path)?;
    if !path.exists() {
        return Err(format!("“{name}” no longer exists."));
    }
    // ditto replaces whatever is at its destination, so claim the name first.
    let destination = claim(path, file_name::archive_names(name), |destination| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map(drop)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::AlreadyExists => Attempt::Taken,
                _ => Attempt::Failed(error.to_string()),
            })
    })?;
    let mut ditto = Command::new("/usr/bin/ditto");
    ditto.args(["-c", "-k", "--sequesterRsrc"]);
    // A folder's archive holds the folder itself; a file's would otherwise hold its parent.
    if path.is_dir() {
        ditto.arg("--keepParent");
    }
    let output = ditto.arg(path).arg(&destination).output();
    match output {
        Ok(output) if output.status.success() => Ok(destination),
        failed => {
            let _ = std::fs::remove_file(&destination);
            Err(match failed {
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let reason = stderr.lines().find(|line| !line.trim().is_empty());
                    reason.unwrap_or("ditto failed").trim().to_owned()
                }
                Err(error) => error.to_string(),
            })
        }
    }
}

/// `NSFileWriteFileExistsError` in `NSCocoaErrorDomain`.
const NS_FILE_WRITE_FILE_EXISTS_ERROR: isize = 516;

enum Attempt {
    /// Something took the name since it looked free; try the next one.
    Taken,
    Failed(String),
}

/// Create an entry beside `path` under the first candidate name nothing occupies. `create`
/// must refuse an existing destination; when it does, the next candidate is tried, so an
/// entry that appears meanwhile (another Duplicate of the same item) is never replaced.
fn claim(
    path: &Path,
    candidates: impl Iterator<Item = String>,
    mut create: impl FnMut(&Path) -> Result<(), Attempt>,
) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("“{}” has no parent folder.", path.display()))?;
    for candidate in candidates.map(|name| parent.join(name)) {
        if std::fs::symlink_metadata(&candidate).is_ok() {
            continue;
        }
        match create(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(Attempt::Taken) => continue,
            Err(Attempt::Failed(error)) => return Err(error),
        }
    }
    unreachable!("candidate names never run out")
}

/// Whether the name ends in an extension Finder treats as one: files and packages have
/// them, folders don't.
pub fn has_extension(path: &Path) -> bool {
    !path.is_dir()
        || autoreleasepool(|_| {
            NSWorkspace::sharedWorkspace().isFilePackageAtPath(&NSString::from_str(&path.to_string_lossy()))
        })
}

fn file_name_of(path: &Path) -> Result<&str, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("“{}” has no usable name.", path.display()))
}

fn sibling(path: &Path, name: &str) -> Result<PathBuf, String> {
    path.parent()
        .map(|parent| parent.join(name))
        .ok_or_else(|| format!("“{}” has no parent folder.", path.display()))
}

/// Whether a directory listing holds exactly this name, case included.
fn listed(path: &Path) -> bool {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    std::fs::read_dir(parent).is_ok_and(|entries| entries.flatten().any(|entry| entry.file_name() == name))
}

fn same_entry(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

fn file_url(path: &Path) -> objc2::rc::Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("dupin-file-ops-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn file(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, contents).unwrap();
            path
        }

        fn names(&self) -> Vec<String> {
            let mut names: Vec<_> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn rename_moves_within_the_folder_and_never_replaces() {
        let dir = Scratch::new("rename");
        let source = dir.file("a.txt", "a");
        dir.file("b.txt", "b");
        assert!(rename(&source, "b.txt").is_err());
        assert_eq!(std::fs::read_to_string(dir.0.join("b.txt")).unwrap(), "b");
        assert!(rename(&source, "../escape.txt").is_err());
        let renamed = rename(&source, "c.txt").unwrap();
        assert_eq!(renamed, dir.0.join("c.txt"));
        assert_eq!(dir.names(), ["b.txt", "c.txt"]);
        assert_eq!(std::fs::read_to_string(renamed).unwrap(), "a");
    }

    #[test]
    fn renaming_onto_a_hard_link_of_itself_fails_instead_of_doing_nothing() {
        let dir = Scratch::new("hard-link");
        let source = dir.file("a.txt", "a");
        std::fs::hard_link(&source, dir.0.join("b.txt")).unwrap();
        assert!(rename(&source, "b.txt").is_err());
        assert_eq!(dir.names(), ["a.txt", "b.txt"]);
    }

    #[test]
    fn a_name_taken_after_it_looked_free_moves_on_to_the_next() {
        let dir = Scratch::new("claim");
        let source = dir.file("report.pdf", "r");
        let mut tries = Vec::new();
        let claimed = claim(
            &source,
            file_name::duplicate_names("report.pdf", true),
            |candidate| {
                tries.push(candidate.file_name().unwrap().to_string_lossy().into_owned());
                if tries.len() == 1 {
                    std::fs::write(candidate, "racing").unwrap();
                    return Err(Attempt::Taken);
                }
                std::fs::write(candidate, "mine").map_err(|error| Attempt::Failed(error.to_string()))
            },
        )
        .unwrap();
        assert_eq!(tries, ["report copy.pdf", "report copy 2.pdf"]);
        assert_eq!(claimed, dir.0.join("report copy 2.pdf"));
        assert_eq!(
            std::fs::read_to_string(dir.0.join("report copy.pdf")).unwrap(),
            "racing"
        );
    }

    #[test]
    fn rename_can_change_only_the_case() {
        let dir = Scratch::new("case");
        let source = dir.file("notes.txt", "n");
        let renamed = rename(&source, "Notes.txt").unwrap();
        assert_eq!(dir.names(), ["Notes.txt"]);
        assert_eq!(std::fs::read_to_string(renamed).unwrap(), "n");
    }

    #[test]
    fn duplicates_take_the_next_free_finder_name() {
        let dir = Scratch::new("duplicate");
        let source = dir.file("report.pdf", "r");
        dir.file("report copy.pdf", "existing");
        let copy = duplicate(&source).unwrap();
        assert_eq!(copy, dir.0.join("report copy 2.pdf"));
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "r");
        assert_eq!(
            std::fs::read_to_string(dir.0.join("report copy.pdf")).unwrap(),
            "existing"
        );
        let folder = dir.0.join("v1.2");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("inner"), "i").unwrap();
        let copy = duplicate(&folder).unwrap();
        assert_eq!(copy, dir.0.join("v1.2 copy"));
        assert_eq!(std::fs::read_to_string(copy.join("inner")).unwrap(), "i");
    }

    #[test]
    fn compress_zips_beside_the_original_without_replacing_an_archive() {
        let dir = Scratch::new("compress");
        let source = dir.file("report.pdf", "r");
        dir.file("report.pdf.zip", "existing");
        let archive = compress(&source).unwrap();
        assert_eq!(archive, dir.0.join("report.pdf 2.zip"));
        assert_eq!(
            std::fs::read_to_string(dir.0.join("report.pdf.zip")).unwrap(),
            "existing"
        );
        let listing = Command::new("/usr/bin/zipinfo")
            .arg("-1")
            .arg(&archive)
            .output()
            .unwrap();
        let entries = String::from_utf8_lossy(&listing.stdout).into_owned();
        assert_eq!(entries.lines().next(), Some("report.pdf"));
        let folder = dir.0.join("Docs");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("inner.txt"), "i").unwrap();
        let archive = compress(&folder).unwrap();
        assert_eq!(archive, dir.0.join("Docs.zip"));
        let listing = Command::new("/usr/bin/zipinfo")
            .arg("-1")
            .arg(&archive)
            .output()
            .unwrap();
        let entries = String::from_utf8_lossy(&listing.stdout).into_owned();
        assert!(
            entries.lines().any(|entry| entry == "Docs/inner.txt"),
            "{entries}"
        );
    }

    #[test]
    fn a_failed_compress_leaves_no_archive() {
        let dir = Scratch::new("compress-missing");
        assert!(compress(&dir.0.join("missing")).is_err());
        assert!(dir.names().is_empty());
    }
}
