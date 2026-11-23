//! File operations that never replace an existing entry.

use std::ffi::CString;
use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::MetadataExt;
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
    file_name_of(path)?;
    move_exclusively(path, parent_folder(path)?.join(name))
}

/// Move an entry into a different folder, preserving its name.
pub fn move_to(path: &Path, folder: &Path) -> Result<PathBuf, String> {
    let name = file_name_of(path)?;
    let folder = transfer_folder(path, folder)?;
    if path
        .parent()
        .is_some_and(|parent| same_entry_following_links(parent, &folder))
    {
        return Err("The item is already in this folder.".into());
    }
    let destination = folder.join(name);
    match rename_exclusive(path, &destination) {
        Ok(()) => Ok(destination),
        Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
            move_across_volumes(path, &folder, destination)
        }
        Err(error) => Err(move_problem(error, &destination)),
    }
}

fn move_exclusively(path: &Path, destination: PathBuf) -> Result<PathBuf, String> {
    rename_exclusive(path, &destination).map_err(|error| move_problem(error, &destination))?;
    Ok(destination)
}

fn rename_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
    let source = CString::new(source.as_os_str().as_encoded_bytes())?;
    let destination = CString::new(destination.as_os_str().as_encoded_bytes())?;
    // SAFETY: Both paths are NUL-terminated and live through the call.
    let status = unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn move_problem(error: io::Error, destination: &Path) -> String {
    if error.kind() == io::ErrorKind::AlreadyExists {
        let name = destination.file_name().unwrap_or_default().to_string_lossy();
        format!("“{name}” already exists in the destination folder.")
    } else {
        error.to_string()
    }
}

fn move_across_volumes(path: &Path, folder: &Path, destination: PathBuf) -> Result<PathBuf, String> {
    let source = path.symlink_metadata().map_err(|error| error.to_string())?;
    let staging = CopyStaging::new(path, folder)?;
    let destination = move_exclusively(&staging.entry(), destination)?;
    if !path
        .symlink_metadata()
        .is_ok_and(|current| file_identity(&current) == file_identity(&source))
    {
        return Err("The copy was saved in the destination. The source changed during the move.".into());
    }
    autoreleasepool(|_| {
        NSFileManager::defaultManager()
            .removeItemAtURL_error(&file_url(path))
            .map_err(|error| {
                format!(
                    "The copy was saved in the destination. Couldn't remove the source: {}",
                    error.localizedDescription()
                )
            })
    })?;
    Ok(destination)
}

/// Copy into a folder, keeping the original name when available.
pub fn copy_to(path: &Path, folder: &Path) -> Result<PathBuf, String> {
    let name = file_name_of(path)?;
    let folder = transfer_folder(path, folder)?;
    let candidates =
        std::iter::once(name.to_owned()).chain(file_name::duplicate_names(name, has_extension(path)));
    copy_with_names(path, &folder, candidates)
}

fn transfer_folder(path: &Path, folder: &Path) -> Result<PathBuf, String> {
    let folder = folder
        .canonicalize()
        .map_err(|error| format!("Destination folder: {error}"))?;
    if !folder.is_dir() {
        return Err("The destination is no longer a folder.".into());
    }
    let source = path.symlink_metadata().map_err(|error| error.to_string())?;
    // Compare identities so case-insensitive volumes and symlinked destinations are covered.
    if source.is_dir()
        && folder.ancestors().any(|ancestor| {
            ancestor
                .metadata()
                .is_ok_and(|entry| file_identity(&entry) == file_identity(&source))
        })
    {
        return Err("Choose a folder outside the source folder.".into());
    }
    Ok(folder)
}

/// Copy `path` beside itself as "name copy", "name copy 2", …, returning the copy's path.
pub fn duplicate(path: &Path) -> Result<PathBuf, String> {
    let name = file_name_of(path)?;
    copy_with_names(
        path,
        parent_folder(path)?,
        file_name::duplicate_names(name, has_extension(path)),
    )
}

fn copy_with_names(
    path: &Path,
    folder: &Path,
    names: impl Iterator<Item = String>,
) -> Result<PathBuf, String> {
    let staging = CopyStaging::new(path, folder)?;
    let entry = staging.entry();
    claim(folder, names, |destination| {
        rename_exclusive(&entry, destination).map_err(Attempt::from)
    })
}

struct CopyStaging(tempfile::TempDir);

impl CopyStaging {
    fn new(path: &Path, folder: &Path) -> Result<Self, String> {
        // Publish the completed copy atomically, even when another app creates the same name.
        let staging = Self(
            tempfile::Builder::new()
                .prefix(".dupin-")
                .tempdir_in(folder)
                .map_err(|error| error.to_string())?,
        );
        autoreleasepool(|_| {
            NSFileManager::defaultManager()
                .copyItemAtURL_toURL_error(&file_url(path), &file_url(&staging.entry()))
                .map_err(|error| error.localizedDescription().to_string())
        })?;
        Ok(staging)
    }

    fn entry(&self) -> PathBuf {
        self.0.path().join("entry")
    }
}

/// Zip `path` beside itself as "name.zip", "name 2.zip", …, as Finder's Compress does,
/// returning the archive's path.
pub fn compress(path: &Path) -> Result<PathBuf, String> {
    let name = file_name_of(path)?;
    if !path.exists() {
        return Err(format!("“{name}” no longer exists."));
    }
    // ditto replaces whatever is at its destination, so claim the name first.
    let destination = claim(
        parent_folder(path)?,
        file_name::archive_names(name),
        |destination| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)
                .map(drop)
                .map_err(Attempt::from)
        },
    )?;
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

enum Attempt {
    /// Something took the name since it looked free; try the next one.
    Taken,
    Failed(String),
}

impl From<io::Error> for Attempt {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::AlreadyExists => Self::Taken,
            _ => Self::Failed(error.to_string()),
        }
    }
}

/// Create an entry in `folder` under the first candidate name nothing occupies. `create`
/// must refuse an existing destination; when it does, the next candidate is tried, so an
/// entry that appears meanwhile (another Duplicate of the same item) is never replaced.
fn claim(
    folder: &Path,
    candidates: impl Iterator<Item = String>,
    mut create: impl FnMut(&Path) -> Result<(), Attempt>,
) -> Result<PathBuf, String> {
    for candidate in candidates.map(|name| folder.join(name)) {
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

fn parent_folder(path: &Path) -> Result<&Path, String> {
    path.parent()
        .ok_or_else(|| format!("“{}” has no parent folder.", path.display()))
}

fn same_entry_following_links(a: &Path, b: &Path) -> bool {
    match (a.metadata(), b.metadata()) {
        (Ok(a), Ok(b)) => file_identity(&a) == file_identity(&b),
        _ => false,
    }
}

fn file_identity(entry: &std::fs::Metadata) -> (u64, u64) {
    (entry.dev(), entry.ino())
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
        let mut tries = Vec::new();
        let claimed = claim(
            &dir.0,
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
    fn copy_keeps_the_original_name_until_a_collision() {
        let dir = Scratch::new("copy-to");
        let source = dir.file("report.pdf", "original");
        let folder = dir.0.join("destination");
        std::fs::create_dir(&folder).unwrap();
        let folder = folder.canonicalize().unwrap();
        let first = copy_to(&source, &folder).unwrap();
        assert_eq!(first, folder.join("report.pdf"));
        std::fs::write(&first, "existing").unwrap();
        std::os::unix::fs::symlink("missing", folder.join("report copy.pdf")).unwrap();
        let next = copy_to(&source, &folder).unwrap();
        assert_eq!(next, folder.join("report copy 2.pdf"));
        assert_eq!(std::fs::read_to_string(first).unwrap(), "existing");
        assert_eq!(std::fs::read_to_string(next).unwrap(), "original");
        assert_eq!(std::fs::read_to_string(source).unwrap(), "original");
    }

    #[test]
    fn move_keeps_the_name_and_refuses_existing_entries_and_its_own_folder() {
        let dir = Scratch::new("move-to");
        let source = dir.file("report.pdf", "original");
        let folder = dir.0.join("destination");
        std::fs::create_dir(&folder).unwrap();
        let alias = folder.join("own-folder");
        std::os::unix::fs::symlink(&dir.0, &alias).unwrap();
        assert_eq!(
            move_to(&source, &alias).unwrap_err(),
            "The item is already in this folder."
        );
        let occupied = folder.join("report.pdf");
        std::fs::write(&occupied, "existing").unwrap();
        assert!(move_to(&source, &folder).unwrap_err().contains("already exists"));
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "original");
        assert_eq!(std::fs::read_to_string(&occupied).unwrap(), "existing");
        std::fs::remove_file(&occupied).unwrap();
        let output = move_to(&source, &folder).unwrap();
        assert_eq!(output, occupied.canonicalize().unwrap());
        assert!(!source.exists());
        assert_eq!(std::fs::read_to_string(output).unwrap(), "original");
    }

    #[test]
    fn folder_transfers_refuse_the_source_its_descendants_and_non_folders() {
        let dir = Scratch::new("recursive-transfer");
        let source = dir.0.join("source");
        let child = source.join("child");
        std::fs::create_dir_all(&child).unwrap();
        let alias = dir.0.join("alias");
        std::os::unix::fs::symlink(&child, &alias).unwrap();
        let file = dir.file("file", "contents");
        for operation in [move_to, copy_to] {
            for folder in [&source, &child, &alias] {
                assert_eq!(
                    operation(&source, folder).unwrap_err(),
                    "Choose a folder outside the source folder."
                );
            }
            assert_eq!(
                operation(&source, &file).unwrap_err(),
                "The destination is no longer a folder."
            );
        }
        assert_eq!(std::fs::read_dir(&child).unwrap().count(), 0);
    }

    #[test]
    fn competing_moves_cannot_replace_one_another() {
        let dir = Scratch::new("competing-moves");
        for name in ["a", "b", "destination"] {
            std::fs::create_dir(dir.0.join(name)).unwrap();
        }
        let destination = dir.0.join("destination");
        for index in 0..24 {
            let name = format!("document-{index}");
            let a = dir.file(&format!("a/{name}"), "a");
            let b = dir.file(&format!("b/{name}"), "b");
            let barrier = std::sync::Barrier::new(2);
            let (first, second) = std::thread::scope(|scope| {
                let first = scope.spawn(|| {
                    barrier.wait();
                    move_to(&a, &destination)
                });
                barrier.wait();
                let second = move_to(&b, &destination);
                (first.join().unwrap(), second)
            });
            assert_ne!(first.is_ok(), second.is_ok());
            let (output, winner, remaining) = match (first, second) {
                (Ok(output), Err(_)) => (output, "a", b),
                (Err(_), Ok(output)) => (output, "b", a),
                _ => unreachable!(),
            };
            assert_eq!(std::fs::read_to_string(output).unwrap(), winner);
            assert!(remaining.exists());
        }
    }

    #[test]
    fn copies_and_moves_competing_for_a_name_keep_both_contents() {
        let dir = Scratch::new("competing-transfers");
        for name in ["a", "b", "destination"] {
            std::fs::create_dir(dir.0.join(name)).unwrap();
        }
        let destination = dir.0.join("destination").canonicalize().unwrap();
        let a = dir.file("a/report.pdf", "a");
        let b = dir.file("b/report.pdf", "b");
        let (copy, moved) = std::thread::scope(|scope| {
            let copy = scope.spawn(|| copy_to(&a, &destination).unwrap());
            let moved = move_to(&b, &destination);
            (copy.join().unwrap(), moved)
        });
        assert_eq!(std::fs::read_to_string(copy).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(moved.unwrap_or(b)).unwrap(), "b");
        assert_eq!(std::fs::read_to_string(a).unwrap(), "a");
        assert!(std::fs::read_dir(destination).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".dupin-")
        }));
    }

    #[test]
    fn staged_move_preserves_a_folder_and_removes_it_only_after_publishing() {
        let dir = Scratch::new("staged-move");
        let source = dir.0.join("source");
        let folder = dir.0.join("destination");
        let target = folder.join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        dir.file("source/child.txt", "original");
        assert!(move_across_volumes(&source, &folder, target.clone()).is_err());
        assert!(source.join("child.txt").exists());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        std::fs::remove_dir(&target).unwrap();
        let output = move_across_volumes(&source, &folder, target).unwrap();
        assert_eq!(
            std::fs::read_to_string(output.join("child.txt")).unwrap(),
            "original"
        );
        assert!(!source.exists());
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
