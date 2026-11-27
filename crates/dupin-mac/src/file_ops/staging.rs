use std::ffi::OsStr;
use std::fs::{self, DirBuilder, File, Metadata};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const PREFIX: &str = ".dupin-copy-in-progress-";
const MARKER: &str = ".dupin-copy-receipt";
const REMOVE_PHASE: &str = ".remove-";

pub(super) struct Staging {
    receipt: Receipt,
    _lock: File,
}

struct Receipt {
    path: PathBuf,
    stage: PathBuf,
}

impl Staging {
    pub(super) fn new(journal: &Path, folder: &Path) -> io::Result<Self> {
        DirBuilder::new().recursive(true).mode(0o700).create(journal)?;
        let journal = journal.canonicalize()?;
        let lock = File::open(&journal)?;
        lock.lock_shared()?;
        let receipt = Receipt::register(&journal, folder)?;
        Ok(Self { receipt, _lock: lock })
    }

    pub(super) fn entry(&self) -> PathBuf {
        self.receipt.stage.join("entry")
    }
}

impl Receipt {
    fn register(journal: &Path, folder: &Path) -> io::Result<Self> {
        // Staging must be on the destination volume for an exclusive, atomic publish.
        let staging = tempfile::Builder::new()
            .prefix(PREFIX)
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(folder.canonicalize()?)?;
        let name = staging.path().file_name().expect("named staging directory");
        let receipt = journal.join(name);
        // A startup sweep must be able to find the staging directory before any data is copied.
        fs::write(staging.path().join(MARKER), name.as_encoded_bytes())?;
        symlink(staging.path(), &receipt)?;
        Ok(Self {
            path: receipt,
            stage: staging.keep(),
        })
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = reap(&self.receipt.path);
    }
}

pub(super) fn recover(journal: &Path) -> io::Result<()> {
    let journal = match journal.canonicalize() {
        Ok(journal) => journal,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    // Other Dupin processes may still be copying while this one is starting.
    let lock = File::open(&journal)?;
    lock.lock()?;
    let mut failure = Ok(());
    for receipt in fs::read_dir(journal)? {
        let result = receipt.and_then(|receipt| {
            if receipt
                .file_name()
                .as_encoded_bytes()
                .starts_with(PREFIX.as_bytes())
            {
                reap(&receipt.path())
            } else {
                Ok(())
            }
        });
        if result.is_err() {
            failure = result;
        }
    }
    failure
}

fn reap(receipt: &Path) -> io::Result<()> {
    let stage = match fs::read_link(receipt) {
        Ok(stage) => stage,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let invalid = || io::Error::other("invalid copy staging receipt");
    let name = receipt.file_name().and_then(OsStr::to_str).ok_or_else(invalid)?;
    let (name, identity) = name
        .split_once(REMOVE_PHASE)
        .map_or((name, None), |(name, identity)| (name, Some(identity)));
    if !stage.is_absolute() || stage.file_name() != Some(OsStr::new(name)) {
        return Err(invalid());
    }
    let metadata = match stage.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if !stage.parent().is_some_and(Path::is_dir) {
                // Removable volumes can be absent when Dupin starts.
                return Ok(());
            }
            return fs::remove_file(receipt);
        }
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() || identity.is_some_and(|id| id != directory_identity(&metadata)) {
        return fs::remove_file(receipt);
    }
    let marker = stage.join(MARKER);
    let marked = match marker.symlink_metadata() {
        Ok(metadata) if metadata.is_file() => fs::read(&marker)? == name.as_bytes(),
        Ok(_) => false,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    let pending;
    let receipt = if identity.is_some() {
        receipt
    } else {
        if !marked {
            return fs::remove_file(receipt);
        }
        let entry = stage.join("entry");
        match entry.symlink_metadata() {
            Ok(metadata) if metadata.is_dir() => remove_staged_directory(&entry)?,
            Ok(_) => fs::remove_file(&entry)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        // Record ownership before deleting the marker so an interrupted removal can resume.
        pending = receipt.with_file_name(format!("{name}{REMOVE_PHASE}{}", directory_identity(&metadata)));
        fs::rename(receipt, &pending)?;
        &pending
    };
    if marked {
        fs::remove_file(marker)?;
    }
    fs::remove_dir(stage)?;
    fs::remove_file(receipt)
}

fn directory_identity(metadata: &Metadata) -> String {
    let (device, inode) = (metadata.dev(), metadata.ino());
    let created = metadata
        .created()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
    match created {
        Some(time) => format!("{device}-{inode}-{}-{}", time.as_secs(), time.subsec_nanos()),
        None => format!("{device}-{inode}"),
    }
}

pub(super) fn remove_staged_directory(root: &Path) -> io::Result<()> {
    // The copy can inherit read-only directories, which would prevent removing their children.
    let mut directories = vec![root.to_owned()];
    while let Some(path) = directories.pop() {
        let metadata = path.symlink_metadata()?;
        if !metadata.is_dir() {
            continue;
        }
        let mode = metadata.permissions().mode();
        if mode & 0o700 != 0o700 {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode | 0o700))?;
        }
        for child in fs::read_dir(path)? {
            let child = child?;
            if child.file_type()?.is_dir() {
                directories.push(child.path());
            }
        }
    }
    fs::remove_dir_all(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_retries_removing_a_stage_after_its_marker_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let journal = dir.path().join("journal");
        fs::create_dir(&journal).unwrap();
        let stage = Receipt::register(&journal, dir.path()).unwrap().stage;
        let blocker = stage.join("blocker");
        fs::write(&blocker, "keep until the next recovery").unwrap();

        assert!(recover(&journal).is_err());
        assert!(!stage.join(MARKER).exists());
        assert_eq!(fs::read_dir(&journal).unwrap().count(), 1);
        fs::remove_file(blocker).unwrap();

        recover(&journal).unwrap();
        assert!(!stage.exists());
        assert_eq!(fs::read_dir(journal).unwrap().count(), 0);
    }

    #[test]
    fn startup_leaves_an_empty_replacement_when_retrying_stage_removal() {
        let dir = tempfile::tempdir().unwrap();
        let journal = dir.path().join("journal");
        fs::create_dir(&journal).unwrap();
        let stage = Receipt::register(&journal, dir.path()).unwrap().stage;
        fs::write(stage.join("blocker"), "").unwrap();
        assert!(recover(&journal).is_err());

        fs::rename(&stage, dir.path().join("original")).unwrap();
        fs::create_dir(&stage).unwrap();

        recover(&journal).unwrap();
        assert!(stage.is_dir());
        assert_eq!(fs::read_dir(journal).unwrap().count(), 0);
    }

    #[test]
    fn startup_removes_abandoned_copy_but_leaves_a_replaced_staging_folder() {
        let dir = tempfile::tempdir().unwrap();
        let journal = dir.path().join("journal");
        fs::create_dir(&journal).unwrap();
        let abandoned = Receipt::register(&journal, dir.path()).unwrap().stage;
        let replaced = Receipt::register(&journal, dir.path()).unwrap().stage;
        assert_eq!(abandoned.metadata().unwrap().permissions().mode() & 0o077, 0);
        let partial = abandoned.join("entry");
        fs::create_dir(&partial).unwrap();
        fs::write(partial.join("data"), "partial copy").unwrap();
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o500)).unwrap();
        fs::remove_dir_all(&replaced).unwrap();
        fs::create_dir(&replaced).unwrap();
        fs::write(replaced.join("entry"), "user data").unwrap();

        let recovery = recover(&journal);
        let _ = fs::set_permissions(&partial, fs::Permissions::from_mode(0o700));
        recovery.unwrap();
        assert!(!abandoned.exists());
        assert_eq!(fs::read_to_string(replaced.join("entry")).unwrap(), "user data");
        assert_eq!(fs::read_dir(journal).unwrap().count(), 0);
    }
}
