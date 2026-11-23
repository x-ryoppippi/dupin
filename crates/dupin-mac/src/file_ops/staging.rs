use std::fs::{self, DirBuilder, File};
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};

const PREFIX: &str = ".dupin-copy-in-progress-";
const MARKER: &str = ".dupin-copy-receipt";

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
    let stage = fs::read_link(receipt)?;
    let name = receipt.file_name().expect("named receipt");
    if !stage.is_absolute() || stage.file_name() != Some(name) {
        return Err(io::Error::other("invalid copy staging receipt"));
    }
    match stage.symlink_metadata() {
        Ok(metadata) => {
            let marker = stage.join(MARKER);
            if !metadata.is_dir()
                || !marker.symlink_metadata().is_ok_and(|metadata| metadata.is_file())
                || !fs::read(&marker).is_ok_and(|value| value == name.as_encoded_bytes())
            {
                return Err(io::Error::other(
                    "the copy staging folder no longer matches its receipt",
                ));
            }
            // Leave the marker in place if removing the staged copy fails and needs to be retried.
            let entry = stage.join("entry");
            match entry.symlink_metadata() {
                Ok(metadata) if metadata.is_dir() => remove_staged_directory(&entry)?,
                Ok(_) => fs::remove_file(&entry)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            fs::remove_file(&marker)?;
            fs::remove_dir(&stage)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if !stage.parent().is_some_and(Path::is_dir) {
                // Removable volumes can be absent when Dupin starts.
                return Ok(());
            }
        }
        Err(error) => return Err(error),
    }
    fs::remove_file(receipt)
}

fn remove_staged_directory(root: &Path) -> io::Result<()> {
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
        assert!(recovery.is_err());
        assert!(!abandoned.exists());
        assert_eq!(fs::read_to_string(replaced.join("entry")).unwrap(), "user data");
    }
}
