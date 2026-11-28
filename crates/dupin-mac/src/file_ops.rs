//! File operations that never replace an existing entry.

use dupin_spawn::Command;
use std::collections::HashMap;
use std::ffi::CString;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use dupin_core::file_name;
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSFileManager, NSString, NSURL};

mod staging;

#[derive(Clone)]
pub struct FileTransfers {
    journal: Option<PathBuf>,
}

impl Default for FileTransfers {
    fn default() -> Self {
        Self {
            journal: std::env::home_dir()
                .map(|home| home.join("Library/Application Support/Dupin/copy-staging")),
        }
    }
}

impl FileTransfers {
    /// Remove incomplete copies left behind by an interrupted process.
    pub fn recover_abandoned_copies(&self) -> io::Result<()> {
        self.journal.as_deref().map_or(Ok(()), staging::recover)
    }

    /// Move an entry into a different folder, preserving its name.
    pub fn move_to(&self, path: &Path, folder: &Path) -> Result<PathBuf, String> {
        let name = file_name_of(path)?;
        let folder = transfer_folder(path, folder)?;
        if path
            .parent()
            .is_some_and(|parent| same_entry_following_links(parent, &folder))
        {
            return Err("The item is already in this folder.".into());
        }
        let destination = folder.join(name);
        match rename_across_parents(path, &destination, rename_exclusive) {
            Ok(()) => Ok(destination),
            Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
                self.move_across_volumes(path, &folder, destination)
            }
            Err(error) => Err(move_problem(error, &destination)),
        }
    }

    /// Copy into a folder, keeping the original name when available.
    pub fn copy_to(&self, path: &Path, folder: &Path) -> Result<PathBuf, String> {
        let name = file_name_of(path)?;
        let folder = transfer_folder(path, folder)?;
        let candidates =
            std::iter::once(name.to_owned()).chain(file_name::duplicate_names(name, has_extension(path)));
        self.copy_with_names(path, &folder, candidates)
    }

    /// Copy beside the original under a Finder-style name that is available.
    pub fn duplicate(&self, path: &Path) -> Result<PathBuf, String> {
        let name = file_name_of(path)?;
        self.copy_with_names(
            path,
            parent_folder(path)?,
            file_name::duplicate_names(name, has_extension(path)),
        )
    }

    fn move_across_volumes(
        &self,
        path: &Path,
        folder: &Path,
        destination: PathBuf,
    ) -> Result<PathBuf, String> {
        match destination.symlink_metadata() {
            Ok(_) => return Err(move_problem(io::ErrorKind::AlreadyExists.into(), &destination)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(move_problem(error, &destination)),
        }
        let source = SourceSnapshot::read(path).map_err(|error| error.to_string())?;
        let staging = self.stage_copy(path, folder)?;
        publish_move(path, &source, &staging.entry(), destination, |path| {
            // A writer can still modify the source between the last check and removal.
            crate::workspace::run(&crate::workspace::Action::Trash(path.to_owned()))
        })
    }

    fn copy_with_names(
        &self,
        path: &Path,
        folder: &Path,
        names: impl Iterator<Item = String>,
    ) -> Result<PathBuf, String> {
        let staging = self.stage_copy(path, folder)?;
        let entry = staging.entry();
        claim(folder, names, |destination| {
            rename_across_parents(&entry, destination, rename_exclusive).map_err(Attempt::from)
        })
    }

    fn stage_copy(&self, path: &Path, folder: &Path) -> Result<staging::Staging, String> {
        let journal = self
            .journal
            .as_deref()
            .ok_or("Dupin's support folder is unavailable.")?;
        let staging = staging::Staging::new(journal, folder).map_err(|error| error.to_string())?;
        autoreleasepool(|_| {
            let source = file_url(path)?;
            let destination = file_url(&staging.entry())?;
            NSFileManager::defaultManager()
                .copyItemAtURL_toURL_error(&source, &destination)
                .map_err(|error| error.localizedDescription().to_string())
        })?;
        Ok(staging)
    }
}

/// Rename `path` to `name` in the same folder, returning the new path.
pub fn rename(path: &Path, name: &str) -> Result<PathBuf, String> {
    if let Some(problem) = file_name::problem(name) {
        return Err(problem.into());
    }
    file_name_of(path)?;
    let destination = parent_folder(path)?.join(name);
    rename_exclusive(path, &destination).map_err(|error| match error.kind() {
        io::ErrorKind::AlreadyExists => format!("“{name}” already exists."),
        _ => error.to_string(),
    })?;
    Ok(destination)
}

fn publish_move(
    path: &Path,
    original: &SourceSnapshot,
    staged: &Path,
    destination: PathBuf,
    trash: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    rename_across_parents(staged, &destination, rename_exclusive)
        .map_err(|error| move_problem(error, &destination))?;
    match SourceSnapshot::read(path) {
        Ok(current) if current == *original => {}
        Ok(_) => {
            return Err("The copy was saved in the destination. The source changed during the move.".into());
        }
        Err(error) => {
            return Err(format!(
                "The copy was saved in the destination. Couldn't verify the source: {error}"
            ));
        }
    }
    trash_move_source(path, &original.0[path], trash)
        .map_err(|error| format!("The copy was saved in the destination. {error}"))?;
    Ok(destination)
}

fn trash_move_source(
    path: &Path,
    original: &EntryStamp,
    trash: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let trash = || trash(path).map_err(|error| format!("Couldn't move the source to the Trash: {error}"));
    let prepare_error = |error| format!("Couldn't prepare the source for the Trash: {error}");
    let changed = || "The source changed during the move.".to_owned();
    let metadata = path.symlink_metadata().map_err(prepare_error)?;
    if EntryStamp::from(&metadata) != *original {
        return Err(changed());
    }
    let permissions = metadata.permissions();
    // SAFETY: geteuid takes no pointers and has no preconditions.
    let user = unsafe { libc::geteuid() };
    if !metadata.is_dir() || permissions.mode() & 0o200 != 0 || metadata.uid() != user {
        return trash();
    }

    // Moving to the Trash changes the parent. Restore the mode by descriptor
    // because the original path may no longer name this folder.
    let entry = open_directory(path).map_err(prepare_error)?;
    if EntryStamp::from(&entry.metadata().map_err(prepare_error)?) != *original {
        return Err(changed());
    }
    entry
        .set_permissions(fs::Permissions::from_mode(permissions.mode() | 0o200))
        .map_err(prepare_error)?;
    match (trash(), entry.set_permissions(permissions)) {
        (result, Ok(())) => result,
        (Ok(()), Err(error)) => Err(format!(
            "The source was moved to the Trash, but its folder permissions couldn't be restored ({error})."
        )),
        (Err(trash_error), Err(error)) => Err(format!(
            "{trash_error}. Couldn't restore the source folder permissions ({error})."
        )),
    }
}

fn open_directory(path: &Path) -> io::Result<fs::File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

fn rename_across_parents(
    source: &Path,
    destination: &Path,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let metadata = source.symlink_metadata()?;
    let original = metadata.permissions();
    // SAFETY: geteuid takes no pointers and has no preconditions.
    let user = unsafe { libc::geteuid() };
    if !metadata.is_dir() || original.mode() & 0o200 != 0 || metadata.uid() != user {
        return rename(source, destination);
    }

    // Moving a directory to another parent needs write access to update its `..`.
    let entry = unlock_directory(source, &metadata)?;
    let result = rename(source, destination);
    if let Err(error) = entry.set_permissions(original) {
        return Err(io::Error::other(match result {
            Ok(()) => format!(
                "The folder is at “{}”, but its permissions couldn't be restored ({error}).",
                destination.display()
            ),
            Err(rename_error) => {
                format!("{rename_error}. Couldn't restore the folder permissions ({error}).")
            }
        }));
    }
    result
}

/// Give the owner write and search access to the folder `checked` describes, returning a
/// descriptor to restore its mode by. The mode changes only through a descriptor whose
/// identity matches `checked`: another user who can rename entries in the parent could have
/// put a link, file or folder where the folder was.
fn unlock_directory(source: &Path, checked: &fs::Metadata) -> io::Result<fs::File> {
    let open = |access| {
        OpenOptions::new()
            .read(true)
            .custom_flags(access | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(source)
    };
    let entry = open(libc::O_SEARCH).or_else(|_| open(0)).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("The folder couldn't be opened to unlock it ({error})."),
        )
    })?;
    if file_identity(&entry.metadata()?) != file_identity(checked) {
        return Err(io::Error::other("The folder changed during the move."));
    }
    entry.set_permissions(fs::Permissions::from_mode(checked.permissions().mode() | 0o300))?;
    Ok(entry)
}

#[derive(PartialEq, Eq)]
struct SourceSnapshot(HashMap<PathBuf, EntryStamp>);

#[derive(PartialEq, Eq)]
struct EntryStamp {
    identity: (u64, u64),
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl From<&fs::Metadata> for EntryStamp {
    fn from(metadata: &fs::Metadata) -> Self {
        Self {
            identity: file_identity(metadata),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

impl SourceSnapshot {
    fn read(path: &Path) -> io::Result<Self> {
        let mut entries = HashMap::new();
        let mut pending = vec![path.to_owned()];
        while let Some(path) = pending.pop() {
            let metadata = path.symlink_metadata()?;
            if metadata.is_dir() {
                for child in fs::read_dir(&path)? {
                    pending.push(child?.path());
                }
            }
            entries.insert(path, EntryStamp::from(&metadata));
        }
        Ok(Self(entries))
    }
}

fn rename_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
    rename_with_fallback(source, destination, rename_no_replace)
}

fn rename_with_fallback(
    source: &Path,
    destination: &Path,
    rename_no_replace: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    match rename_no_replace(source, destination) {
        Err(error) if error.raw_os_error() == Some(libc::ENOTSUP) => {
            rename_without_exclusive_support(source, destination)
        }
        result => result,
    }
}

fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
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

fn rename_without_exclusive_support(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = source.symlink_metadata()?;
    if let (Some(parent), Some(old), Some(new)) = (
        source.parent(),
        source.file_name().and_then(|name| name.to_str()),
        destination.file_name().and_then(|name| name.to_str()),
    ) && destination.parent() == Some(parent)
        && old != new
        && old.to_lowercase() == new.to_lowercase()
        && destination
            .symlink_metadata()
            .is_ok_and(|entry| file_identity(&entry) == file_identity(&metadata))
    {
        // A case alias already holds the name. A rename between hard links is a no-op.
        fs::rename(source, destination)?;
        for entry in fs::read_dir(parent.join("."))? {
            if entry?.file_name() == old {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
        }
        return Ok(());
    }

    // FAT volumes lack exclusive rename and hard links. Claim the destination before renaming.
    let directory = metadata.is_dir();
    let reservation = if directory {
        DirBuilder::new().mode(0o700).create(destination)?;
        open_directory(destination)?
    } else {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(destination)?
    };
    let identity = file_identity(&reservation.metadata()?);
    let still_reserved = || {
        destination
            .symlink_metadata()
            .is_ok_and(|metadata| file_identity(&metadata) == identity)
    };
    if !still_reserved() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }

    let result = fs::rename(source, destination);
    if result.is_err() && still_reserved() {
        let _ = if directory {
            fs::remove_dir(destination)
        } else {
            fs::remove_file(destination)
        };
    }
    result
}

fn move_problem(error: io::Error, destination: &Path) -> String {
    if error.kind() == io::ErrorKind::AlreadyExists {
        let name = destination.file_name().unwrap_or_default().to_string_lossy();
        format!("“{name}” already exists in the destination folder.")
    } else {
        error.to_string()
    }
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

/// Zip `path` beside itself as "name.zip", "name 2.zip", …, as Finder's Compress does,
/// returning the archive's path.
pub fn compress(path: &Path) -> Result<PathBuf, String> {
    let name = file_name_of(path)?;
    if !path.exists() {
        return Err(format!("“{name}” no longer exists."));
    }
    // Claim the name before writing, so an existing entry is never replaced.
    let mut archive = None;
    let destination = claim(
        parent_folder(path)?,
        file_name::archive_names(name),
        |destination| {
            archive = Some(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(destination)?,
            );
            Ok(())
        },
    )?;
    let archive = archive.ok_or("The archive couldn't be created.")?;
    write_archive(path, &destination, archive)?;
    Ok(destination)
}

/// Zip `path` into `archive`, the file claimed as `destination`. ditto writes to standard
/// output, not to the name: it follows a link there, and another user who can rename
/// entries in the folder could swap the claimed name for one.
fn write_archive(path: &Path, destination: &Path, archive: fs::File) -> Result<(), String> {
    let identity = file_identity(&archive.metadata().map_err(|error| error.to_string())?);
    let still_ours = || {
        destination
            .symlink_metadata()
            .is_ok_and(|entry| file_identity(&entry) == identity)
    };
    let mut ditto = Command::new("/usr/bin/ditto");
    ditto.args(["-c", "-k", "--sequesterRsrc"]);
    // A folder's archive holds the folder itself; a file's would otherwise hold its parent.
    if path.is_dir() {
        ditto.arg("--keepParent");
    }
    let output = ditto.arg(path).arg("-").stdout(archive).output();
    let problem = match output {
        Ok(output) if output.status.success() && still_ours() => return Ok(()),
        Ok(output) if output.status.success() => {
            let name = destination.file_name().unwrap_or_default().to_string_lossy();
            return Err(format!("“{name}” was replaced while it was being written."));
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr.lines().find(|line| !line.trim().is_empty());
            reason.unwrap_or("ditto failed").trim().to_owned()
        }
        Err(error) => error.to_string(),
    };
    if still_ours() {
        let _ = fs::remove_file(destination);
    }
    Err(problem)
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

fn file_url(path: &Path) -> Result<objc2::rc::Retained<NSURL>, String> {
    let path = path.to_str().ok_or("The path cannot be represented as text.")?;
    Ok(NSURL::fileURLWithPath(&NSString::from_str(path)))
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

        fn transfers(&self) -> FileTransfers {
            FileTransfers {
                journal: Some(self.0.join("journal")),
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = staging::remove_staged_directory(&self.0);
        }
    }

    #[test]
    fn rename_moves_within_the_folder_and_never_replaces() {
        let dir = Scratch::new("rename");
        let source = dir.file("a.txt", "a");
        dir.file("b.txt", "b");
        assert_eq!(rename(&source, "b.txt").unwrap_err(), "“b.txt” already exists.");
        assert_eq!(std::fs::read_to_string(dir.0.join("b.txt")).unwrap(), "b");
        assert!(rename(&source, "../escape.txt").is_err());
        let renamed = rename(&source, "c.txt").unwrap();
        assert_eq!(renamed, dir.0.join("c.txt"));
        assert_eq!(dir.names(), ["b.txt", "c.txt"]);
        assert_eq!(std::fs::read_to_string(renamed).unwrap(), "a");
    }

    #[test]
    fn unsupported_exclusive_rename_claims_the_destination_for_files_and_folders() {
        let dir = Scratch::new("rename-unsupported");
        let file = dir.file("source.txt", "original");
        let folder = dir.0.join("source-folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("child"), "original").unwrap();
        let file_destination = dir.0.join("target.txt");
        let folder_destination = dir.0.join("target-folder");
        for (source, destination, contents) in [
            (file, &file_destination, file_destination.clone()),
            (folder, &folder_destination, folder_destination.join("child")),
        ] {
            let taken = dir.0.join("taken");
            std::os::unix::fs::symlink("missing", &taken).unwrap();
            let error = rename_with_fallback(&source, &taken, |_, _| {
                Err(io::Error::from_raw_os_error(libc::ENOTSUP))
            })
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
            assert_eq!(fs::read_link(&taken).unwrap(), Path::new("missing"));
            fs::remove_file(taken).unwrap();

            rename_with_fallback(&source, destination, |_, _| {
                Err(io::Error::from_raw_os_error(libc::ENOTSUP))
            })
            .unwrap();
            assert!(!source.exists());
            assert_eq!(fs::read_to_string(contents).unwrap(), "original");
        }
    }

    #[test]
    fn unsupported_exclusive_rename_can_change_only_the_case() {
        let dir = Scratch::new("rename-unsupported-case");
        let source = dir.file("notes.txt", "original");
        let destination = dir.0.join("Notes.txt");
        if !destination.exists() {
            eprintln!("Case-only rename fallback requires a case-insensitive scratch volume");
            return;
        }

        rename_with_fallback(&source, &destination, |_, _| {
            Err(io::Error::from_raw_os_error(libc::ENOTSUP))
        })
        .unwrap();
        let names = dir.names();
        assert!(names.iter().any(|name| name == "Notes.txt"), "{names:?}");
        assert!(!names.iter().any(|name| name == "notes.txt"), "{names:?}");
        assert_eq!(fs::read_to_string(destination).unwrap(), "original");
    }

    #[test]
    fn failed_fallback_removes_its_reservation() {
        let dir = Scratch::new("rename-rollback");
        let source = dir.0.join("source");
        fs::create_dir(&source).unwrap();
        let destination = source.join("destination");
        assert!(
            rename_with_fallback(&source, &destination, |_, _| {
                Err(io::Error::from_raw_os_error(libc::ENOTSUP))
            })
            .is_err()
        );
        assert!(source.is_dir());
        assert!(!destination.exists());
    }

    #[test]
    fn publishing_a_read_only_folder_restores_its_mode_with_the_reservation_fallback() {
        let dir = Scratch::new("read-only-fallback");
        let staged = dir.0.join("stage/entry");
        fs::create_dir_all(&staged).unwrap();
        dir.file("stage/entry/child", "original");
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o555)).unwrap();

        let publish_without_exclusive = |source: &Path, destination: &Path| {
            rename_with_fallback(source, destination, |_, _| {
                Err(io::Error::from_raw_os_error(libc::ENOTSUP))
            })
        };
        let invalid = staged.join("destination");
        assert!(rename_across_parents(&staged, &invalid, publish_without_exclusive).is_err());
        assert!(!invalid.exists());
        assert_eq!(staged.metadata().unwrap().mode() & 0o7777, 0o555);

        let destination = dir.0.join("destination");
        rename_across_parents(&staged, &destination, publish_without_exclusive).unwrap();
        assert!(!staged.exists());
        assert_eq!(destination.metadata().unwrap().mode() & 0o7777, 0o555);
        assert_eq!(fs::read_to_string(destination.join("child")).unwrap(), "original");
    }

    #[test]
    fn unlocking_a_read_only_folder_never_changes_an_entry_put_in_its_place() {
        let dir = Scratch::new("unlock-swap");
        let aside = dir.0.join("aside");
        fs::create_dir(&aside).unwrap();
        fs::set_permissions(&aside, fs::Permissions::from_mode(0o555)).unwrap();
        let secret = dir.file("secret", "s");
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
        let private = dir.0.join("private");
        fs::create_dir(&private).unwrap();
        fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
        let checked = aside.symlink_metadata().unwrap();
        let folder = dir.0.join("folder");

        for target in [&secret, &private] {
            std::os::unix::fs::symlink(target, &folder).unwrap();
            assert!(unlock_directory(&folder, &checked).is_err());
            fs::remove_file(&folder).unwrap();
        }
        for target in [&secret, &private] {
            fs::rename(target, &folder).unwrap();
            assert!(unlock_directory(&folder, &checked).is_err());
            fs::rename(&folder, target).unwrap();
        }
        assert_eq!(secret.metadata().unwrap().mode() & 0o7777, 0o600);
        assert_eq!(private.metadata().unwrap().mode() & 0o7777, 0o700);
        assert_eq!(aside.metadata().unwrap().mode() & 0o7777, 0o555);
        fs::set_permissions(&aside, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn unlocking_a_folder_opens_it_by_whichever_access_its_owner_has() {
        let dir = Scratch::new("unlock-access");
        for mode in [0o555, 0o444, 0o111] {
            let folder = dir.0.join(format!("folder-{mode:o}"));
            fs::create_dir(&folder).unwrap();
            fs::set_permissions(&folder, fs::Permissions::from_mode(mode)).unwrap();
            let checked = folder.symlink_metadata().unwrap();

            let entry = unlock_directory(&folder, &checked).unwrap();
            assert_eq!(folder.metadata().unwrap().mode() & 0o7777, mode | 0o300);
            entry.set_permissions(checked.permissions()).unwrap();
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn a_folder_its_owner_cannot_open_is_left_locked() {
        let dir = Scratch::new("unlock-closed");
        let folder = dir.0.join("folder");
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o000)).unwrap();
        let checked = folder.symlink_metadata().unwrap();

        assert!(unlock_directory(&folder, &checked).is_err());
        assert_eq!(folder.symlink_metadata().unwrap().mode() & 0o7777, 0o000);
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn other_rename_errors_are_preserved_without_creating_a_destination() {
        let dir = Scratch::new("rename-error");
        let source = dir.file("source.txt", "original");
        let destination = dir.0.join("destination.txt");
        for errno in [libc::EXDEV, libc::EACCES, libc::EEXIST] {
            let error = rename_with_fallback(&source, &destination, |_, _| {
                Err(io::Error::from_raw_os_error(errno))
            })
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(errno));
            assert!(!destination.exists());
        }
        assert_eq!(fs::read_to_string(source).unwrap(), "original");
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
        let files = dir.transfers();
        let source = dir.file("report.pdf", "r");
        dir.file("report copy.pdf", "existing");
        let copy = files.duplicate(&source).unwrap();
        assert_eq!(copy, dir.0.join("report copy 2.pdf"));
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "r");
        assert_eq!(
            std::fs::read_to_string(dir.0.join("report copy.pdf")).unwrap(),
            "existing"
        );
        let folder = dir.0.join("v1.2");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("inner"), "i").unwrap();
        let copy = files.duplicate(&folder).unwrap();
        assert_eq!(copy, dir.0.join("v1.2 copy"));
        assert_eq!(std::fs::read_to_string(copy.join("inner")).unwrap(), "i");
    }

    #[test]
    fn copy_keeps_the_original_name_until_a_collision() {
        let dir = Scratch::new("copy-to");
        let files = dir.transfers();
        let source = dir.file("report.pdf", "original");
        let folder = dir.0.join("destination");
        std::fs::create_dir(&folder).unwrap();
        let folder = folder.canonicalize().unwrap();
        let first = files.copy_to(&source, &folder).unwrap();
        assert_eq!(first, folder.join("report.pdf"));
        std::fs::write(&first, "existing").unwrap();
        std::os::unix::fs::symlink("missing", folder.join("report copy.pdf")).unwrap();
        let next = files.copy_to(&source, &folder).unwrap();
        assert_eq!(next, folder.join("report copy 2.pdf"));
        assert_eq!(std::fs::read_to_string(first).unwrap(), "existing");
        assert_eq!(std::fs::read_to_string(next).unwrap(), "original");
        assert_eq!(std::fs::read_to_string(source).unwrap(), "original");
    }

    #[test]
    fn copy_and_duplicate_preserve_read_only_folders() {
        let dir = Scratch::new("read-only-copy");
        let files = dir.transfers();
        let source = dir.0.join("source");
        let folder = dir.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&folder).unwrap();
        dir.file("source/child.txt", "original");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o555)).unwrap();

        let copied = files.copy_to(&source, &folder).unwrap();
        let duplicated = files.duplicate(&source).unwrap();

        assert_eq!(copied, folder.canonicalize().unwrap().join("source"));
        assert_eq!(duplicated, dir.0.join("source copy"));
        for path in [source, copied, duplicated] {
            assert_eq!(path.metadata().unwrap().mode() & 0o7777, 0o555);
            assert_eq!(fs::read_to_string(path.join("child.txt")).unwrap(), "original");
        }
    }

    #[test]
    fn move_keeps_the_name_and_refuses_existing_entries_and_its_own_folder() {
        let dir = Scratch::new("move-to");
        let files = dir.transfers();
        let source = dir.file("report.pdf", "original");
        let folder = dir.0.join("destination");
        std::fs::create_dir(&folder).unwrap();
        let alias = folder.join("own-folder");
        std::os::unix::fs::symlink(&dir.0, &alias).unwrap();
        assert_eq!(
            files.move_to(&source, &alias).unwrap_err(),
            "The item is already in this folder."
        );
        let occupied = folder.join("report.pdf");
        std::fs::write(&occupied, "existing").unwrap();
        assert!(
            files
                .move_to(&source, &folder)
                .unwrap_err()
                .contains("already exists")
        );
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "original");
        assert_eq!(std::fs::read_to_string(&occupied).unwrap(), "existing");
        std::fs::remove_file(&occupied).unwrap();
        let output = files.move_to(&source, &folder).unwrap();
        assert_eq!(output, occupied.canonicalize().unwrap());
        assert!(!source.exists());
        assert_eq!(std::fs::read_to_string(output).unwrap(), "original");
    }

    #[test]
    fn move_preserves_read_only_folders() {
        let dir = Scratch::new("read-only-move");
        let source = dir.0.join("source");
        let folder = dir.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&folder).unwrap();
        dir.file("source/child.txt", "original");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o555)).unwrap();

        let moved = dir.transfers().move_to(&source, &folder).unwrap();

        assert_eq!(moved, folder.canonicalize().unwrap().join("source"));
        assert!(!source.exists());
        assert_eq!(moved.metadata().unwrap().mode() & 0o7777, 0o555);
        assert_eq!(fs::read_to_string(moved.join("child.txt")).unwrap(), "original");
    }

    #[test]
    fn folder_transfers_refuse_the_source_its_descendants_and_non_folders() {
        let dir = Scratch::new("recursive-transfer");
        let files = dir.transfers();
        let source = dir.0.join("source");
        let child = source.join("child");
        std::fs::create_dir_all(&child).unwrap();
        let alias = dir.0.join("alias");
        std::os::unix::fs::symlink(&child, &alias).unwrap();
        let file = dir.file("file", "contents");
        for operation in [FileTransfers::move_to, FileTransfers::copy_to] {
            for folder in [&source, &child, &alias] {
                assert_eq!(
                    operation(&files, &source, folder).unwrap_err(),
                    "Choose a folder outside the source folder."
                );
            }
            assert_eq!(
                operation(&files, &source, &file).unwrap_err(),
                "The destination is no longer a folder."
            );
        }
        assert_eq!(std::fs::read_dir(&child).unwrap().count(), 0);
    }

    #[test]
    fn competing_moves_cannot_replace_one_another() {
        let dir = Scratch::new("competing-moves");
        let files = dir.transfers();
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
                    files.move_to(&a, &destination)
                });
                barrier.wait();
                let second = files.move_to(&b, &destination);
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
        let files = dir.transfers();
        for name in ["a", "b", "destination"] {
            std::fs::create_dir(dir.0.join(name)).unwrap();
        }
        let destination = dir.0.join("destination").canonicalize().unwrap();
        let a = dir.file("a/report.pdf", "a");
        let b = dir.file("b/report.pdf", "b");
        let (copy, moved) = std::thread::scope(|scope| {
            let copy = scope.spawn(|| files.copy_to(&a, &destination).unwrap());
            let moved = files.move_to(&b, &destination);
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
        assert_eq!(std::fs::read_dir(files.journal.unwrap()).unwrap().count(), 0);
    }

    #[test]
    fn staged_move_preserves_a_read_only_folder_and_removes_it_only_after_publishing() {
        let dir = Scratch::new("staged-move");
        let files = dir.transfers();
        let source = dir.0.join("source");
        let folder = dir.0.join("destination");
        let target = folder.join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&folder).unwrap();
        std::os::unix::fs::symlink("missing", &target).unwrap();
        dir.file("source/child.txt", "original");
        assert_eq!(
            files
                .move_across_volumes(&source, &folder, target.clone())
                .unwrap_err(),
            "“source” already exists in the destination folder."
        );
        assert!(source.join("child.txt").exists());
        assert_eq!(std::fs::read_link(&target).unwrap(), Path::new("missing"));
        assert!(!files.journal.as_ref().unwrap().exists());
        std::fs::remove_file(&target).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o555)).unwrap();
        let snapshot = SourceSnapshot::read(&source).unwrap();
        let staging = files.stage_copy(&source, &folder).unwrap();
        let trash = dir.0.join("trash");
        fs::create_dir(&trash).unwrap();
        let trashed_source = trash.join("source");
        let output = publish_move(&source, &snapshot, &staging.entry(), target.clone(), |path| {
            assert!(target.join("child.txt").exists());
            assert_eq!(target.metadata().unwrap().mode() & 0o7777, 0o555);
            assert_eq!(path.metadata().unwrap().mode() & 0o7777, 0o755);
            fs::rename(path, &trashed_source).map_err(|error| error.to_string())
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(output.join("child.txt")).unwrap(),
            "original"
        );
        assert!(!source.exists());
        assert_eq!(trashed_source.metadata().unwrap().mode() & 0o7777, 0o555);
        assert_eq!(
            fs::read_to_string(trashed_source.join("child.txt")).unwrap(),
            "original"
        );
    }

    #[test]
    fn staged_move_restores_source_permissions_when_trashing_fails() {
        let dir = Scratch::new("staged-move-trash-failure");
        let files = dir.transfers();
        let source = dir.0.join("source");
        let folder = dir.0.join("destination");
        let target = folder.join("source");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&folder).unwrap();
        let child = dir.file("source/child.txt", "original");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o555)).unwrap();
        let snapshot = SourceSnapshot::read(&source).unwrap();
        let staging = files.stage_copy(&source, &folder).unwrap();

        let error = publish_move(&source, &snapshot, &staging.entry(), target.clone(), |path| {
            assert_eq!(path.metadata().unwrap().mode() & 0o7777, 0o755);
            Err("Trash unavailable".into())
        })
        .unwrap_err();

        assert_eq!(
            error,
            "The copy was saved in the destination. Couldn't move the source to the Trash: Trash unavailable"
        );
        assert_eq!(source.metadata().unwrap().mode() & 0o7777, 0o555);
        assert_eq!(fs::read_to_string(child).unwrap(), "original");
        assert_eq!(fs::read_to_string(target.join("child.txt")).unwrap(), "original");
    }

    #[test]
    fn staged_move_keeps_the_original_when_a_child_changes_in_place() {
        let dir = Scratch::new("staged-move-changed");
        let files = dir.transfers();
        let source = dir.0.join("source");
        let folder = dir.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&folder).unwrap();
        let child = dir.file("source/child.txt", "original");

        let snapshot = SourceSnapshot::read(&source).unwrap();
        let staging = files.stage_copy(&source, &folder).unwrap();
        fs::write(&child, "original updated").unwrap();
        let target = folder.join("source");
        let error = publish_move(&source, &snapshot, &staging.entry(), target.clone(), |_| {
            panic!("a changed source must not be moved to the Trash")
        })
        .unwrap_err();

        assert!(error.contains("The source changed during the move."));
        assert_eq!(fs::read_to_string(child).unwrap(), "original updated");
        assert_eq!(fs::read_to_string(target.join("child.txt")).unwrap(), "original");
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
    fn compress_writes_only_the_claimed_file_when_its_name_becomes_a_link() {
        let dir = Scratch::new("compress-link");
        let source = dir.file("report.pdf", "r");
        let victim = dir.file("victim", "keep");
        let destination = dir.0.join("report.pdf.zip");
        let archive = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .unwrap();
        fs::remove_file(&destination).unwrap();
        std::os::unix::fs::symlink(&victim, &destination).unwrap();

        assert!(write_archive(&source, &destination, archive).is_err());
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
        assert!(destination.symlink_metadata().unwrap().is_symlink());
    }

    #[test]
    fn a_failed_compress_leaves_no_archive() {
        let dir = Scratch::new("compress-missing");
        assert!(compress(&dir.0.join("missing")).is_err());
        assert!(dir.names().is_empty());
    }
}
