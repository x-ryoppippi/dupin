//! The content hash the lock file pins: BLAKE3 over every file's relative path, executable bit
//! and bytes, in path order.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::SystemTime;

const PREFIX: &str = "blake3:";

/// A regular file of a package, relative to its folder.
pub(crate) struct File {
    pub path: String,
    pub executable: bool,
    pub len: u64,
    pub modified: SystemTime,
}

/// Every regular file under `folder`, sorted by path. A symlink or any other kind of entry is an
/// error, because an installed package never contains one.
pub(crate) fn files(folder: &Path) -> io::Result<Vec<File>> {
    let mut files = Vec::new();
    walk(folder, "", &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn walk(dir: &Path, prefix: &str, files: &mut Vec<File>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a file name isn't UTF-8"))?;
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let metadata = entry.path().symlink_metadata()?;
        if metadata.is_dir() {
            walk(&entry.path(), &path, files)?;
        } else if metadata.is_file() {
            files.push(File {
                path,
                executable: metadata.permissions().mode() & 0o111 != 0,
                len: metadata.len(),
                modified: metadata.modified()?,
            });
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{path} is not a regular file"),
            ));
        }
    }
    Ok(())
}

pub fn content_hash(folder: &Path) -> io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    for file in files(folder)? {
        let bytes = std::fs::read(folder.join(&file.path))?;
        hasher.update(&(file.path.len() as u64).to_le_bytes());
        hasher.update(file.path.as_bytes());
        hasher.update(&[u8::from(file.executable)]);
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(format!("{PREFIX}{}", hasher.finalize().to_hex()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_follows_paths_bytes_and_modes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("lib")).unwrap();
        std::fs::write(dir.path().join("main.js"), "export {}").unwrap();
        std::fs::write(dir.path().join("lib/a.js"), "a").unwrap();
        let first = content_hash(dir.path()).unwrap();
        assert!(first.starts_with(PREFIX));
        assert_eq!(content_hash(dir.path()).unwrap(), first);

        std::fs::write(dir.path().join("lib/a.js"), "b").unwrap();
        let edited = content_hash(dir.path()).unwrap();
        assert_ne!(edited, first);

        std::fs::set_permissions(
            dir.path().join("lib/a.js"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert_ne!(content_hash(dir.path()).unwrap(), edited);

        std::os::unix::fs::symlink("/etc/hosts", dir.path().join("link")).unwrap();
        assert!(content_hash(dir.path()).is_err());
    }
}
