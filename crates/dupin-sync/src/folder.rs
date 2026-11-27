//! Choosing and opening the sync folder.

use std::io;
use std::path::{Path, PathBuf};

use dupin_storage::DeviceId;

use crate::error::{Error, Refusal};
use crate::fsx::{Contents, Dir, Kind};
use crate::vault::{HEADER_NAME, MAX_HEADER_BYTES, VaultHeader};

pub const DEVICES: &str = "devices";
pub const NAMESPACES: &str = "ns";
pub const BLOBS: &str = "blobs";
pub const STAGING: &str = "tmp.nosync";

/// File systems whose rename isn't atomic, or isn't promised to be.
const NETWORK: &[&str] = &["smbfs", "afpfs", "webdav", "cifs", "smb3", "ftp"];

#[derive(Debug, Clone, Copy, Default)]
pub struct FolderOptions {
    /// Accept a folder on NFS, which is refused unless the user overrides it.
    pub allow_nfs: bool,
}

/// The iCloud Drive folder Settings offers first, when iCloud Drive exists.
pub fn icloud_drive_default(home: &Path) -> Option<PathBuf> {
    let drive = home.join("Library/Mobile Documents/com~apple~CloudDocs");
    drive.is_dir().then(|| drive.join("Dupin"))
}

/// A header file's name and what parsing it gave.
pub type FoundHeader = (String, Result<VaultHeader, Error>);

/// The sync folder, checked and open.
#[derive(Debug)]
pub struct Folder {
    path: PathBuf,
    root: Dir,
}

impl Folder {
    /// Creates the folder if it is missing, then refuses it when it is, or holds, Dupin's
    /// data directory, or sits on a network file system without atomic rename.
    pub fn open(path: &Path, data_dir: &Path, options: FolderOptions) -> Result<Self, Error> {
        if !path.is_absolute() {
            return Err(Error::Refused(Refusal::NotADirectory));
        }
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut builder = std::fs::DirBuilder::new();
                std::os::unix::fs::DirBuilderExt::mode(builder.recursive(true), 0o700);
                builder.create(path)?;
            }
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        let canonical = std::fs::canonicalize(path)?;
        if !canonical.is_dir() {
            return Err(Error::Refused(Refusal::NotADirectory));
        }
        let data = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_owned());
        if canonical.starts_with(&data) {
            return Err(Error::Refused(Refusal::InsideDataDirectory));
        }
        if data.starts_with(&canonical) {
            return Err(Error::Refused(Refusal::ContainsDataDirectory));
        }
        let root = Dir::open_root(&canonical)?;
        let kind = root.fs_type()?;
        if NETWORK.contains(&kind.as_str()) {
            return Err(Error::Refused(Refusal::NetworkFileSystem(kind)));
        }
        if kind == "nfs" && !options.allow_nfs {
            return Err(Error::Refused(Refusal::Nfs));
        }
        Ok(Self {
            path: canonical,
            root,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn root(&self) -> &Dir {
        &self.root
    }

    /// Every vault header in the folder root: `vault.dupin` and the copies sync tools make of
    /// it, the one place Dupin reads names outside its grammar.
    pub(crate) fn headers(&self) -> Result<Vec<FoundHeader>, Error> {
        let mut headers = Vec::new();
        for entry in self.root.entries()? {
            if entry.kind != Kind::File || !entry.name.starts_with("vault") || !entry.name.ends_with(".dupin")
            {
                continue;
            }
            match self.root.read(&entry.name, MAX_HEADER_BYTES)? {
                Contents::Bytes(bytes) => headers.push((entry.name, VaultHeader::parse(&bytes))),
                Contents::Missing => {}
                Contents::Unavailable => headers.push((
                    entry.name,
                    Err(Error::BadHeader("not downloaded, or too large".into())),
                )),
            }
        }
        headers.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(headers)
    }

    /// Headers that parse.
    pub(crate) fn parsed_headers(&self) -> Result<Vec<VaultHeader>, Error> {
        Ok(self
            .headers()?
            .into_iter()
            .filter_map(|(_, header)| header.ok())
            .collect())
    }

    /// This device's staging directory, `tmp.nosync/<device id>/`, which iCloud Drive never
    /// uploads. Writes are staged here so their rename stays on the folder's volume.
    pub(crate) fn staging(&self, device: DeviceId) -> Result<Dir, Error> {
        Ok(self.root.create_dir(STAGING)?.create_dir(&device.to_string())?)
    }

    /// Writes the first vault header; fails if any header is already there.
    pub(crate) fn create_header(&self, header: &VaultHeader, device: DeviceId) -> Result<(), Error> {
        if !self.parsed_headers()?.is_empty() {
            return Err(Error::Refused(Refusal::HoldsAVault));
        }
        let staging = self.staging(device)?;
        match self.root.create_new(HEADER_NAME, &header.encode(), &staging) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(Error::Refused(Refusal::HoldsAVault))
            }
            other => Ok(other?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_the_data_directory_and_anything_around_it() {
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join("Application Support/Dupin");
        std::fs::create_dir_all(data.join("storage")).unwrap();
        let options = FolderOptions::default();
        for (inside, why) in [
            (data.clone(), Refusal::InsideDataDirectory),
            (data.join("storage"), Refusal::InsideDataDirectory),
            (data.join("storage/sync"), Refusal::InsideDataDirectory),
            (home.path().to_owned(), Refusal::ContainsDataDirectory),
        ] {
            assert!(
                matches!(Folder::open(&inside, &data, options), Err(Error::Refused(r)) if r == why),
                "{}",
                inside.display()
            );
        }
        let link = home.path().join("sneaky");
        std::os::unix::fs::symlink(data.join("storage"), &link).unwrap();
        assert!(matches!(
            Folder::open(&link, &data, options),
            Err(Error::Refused(Refusal::InsideDataDirectory))
        ));
        assert!(Folder::open(Path::new("relative"), &data, options).is_err());
        let file = home.path().join("file");
        std::fs::write(&file, b"").unwrap();
        assert!(Folder::open(&file, &data, options).is_err());
        let folder = Folder::open(&home.path().join("Sync/Dupin"), &data, options).unwrap();
        assert!(folder.path().is_dir());
    }

    #[test]
    fn offers_icloud_drive_only_where_it_exists() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(icloud_drive_default(home.path()), None);
        let drive = home.path().join("Library/Mobile Documents/com~apple~CloudDocs");
        std::fs::create_dir_all(&drive).unwrap();
        assert_eq!(icloud_drive_default(home.path()), Some(drive.join("Dupin")));
    }
}
