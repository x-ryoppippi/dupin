use std::fmt;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::Error;

pub(crate) const KEY_LEN: usize = 32;

/// The 256-bit key every namespace's keys are derived from. It is zeroized when dropped.
#[derive(Clone)]
pub struct DataKey(Zeroizing<[u8; KEY_LEN]>);

impl DataKey {
    pub fn generate() -> Result<Self, Error> {
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        getrandom::fill(bytes.as_mut()).map_err(|error| Error::Key(error.to_string()))?;
        Ok(Self(bytes))
    }

    /// Takes a key that was stored earlier; the slice must be exactly 32 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, Error> {
        let bytes: [u8; KEY_LEN] = bytes
            .try_into()
            .map_err(|_| Error::Key(format!("a stored data key must be {KEY_LEN} bytes")))?;
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// For a [`KeyProvider`] that has to hand the key to its own store.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for DataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DataKey(..)")
    }
}

/// Where the data key lives. It is asked once per [`crate::Storage`], and must create the key
/// only when none exists: a provider that answers a locked or refused key store with a fresh
/// key would make everything already stored unreadable.
pub trait KeyProvider: Send + Sync {
    fn data_key(&self) -> Result<DataKey, Error>;
}

/// A key that lives only in memory, for tests and fixtures.
#[derive(Debug, Clone)]
pub struct MemoryKey(DataKey);

impl MemoryKey {
    pub fn new(key: DataKey) -> Self {
        Self(key)
    }

    pub fn random() -> Self {
        Self(DataKey::generate().expect("the system random source is available"))
    }
}

impl KeyProvider for MemoryKey {
    fn data_key(&self) -> Result<DataKey, Error> {
        Ok(self.0.clone())
    }
}

/// A key kept in a file only the user can read, for systems without a usable OS key store.
/// Anyone who can read the file, or a backup that includes it alongside the data, can decrypt
/// everything, so this is weaker than an OS key store.
#[derive(Debug, Clone)]
pub struct FileKey {
    path: PathBuf,
}

impl FileKey {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl KeyProvider for FileKey {
    fn data_key(&self) -> Result<DataKey, Error> {
        match read_key_file(&self.path) {
            Ok(key) => return Ok(key),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let key = DataKey::generate()?;
        if let Some(parent) = self.path.parent() {
            crate::fs::create_private_dir(parent)?;
        }
        match crate::fs::create_new_private(&self.path, key.as_bytes()) {
            Ok(()) => Ok(key),
            // Another process created it first; theirs is the key.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_key_file(&self.path),
            Err(error) => Err(error.into()),
        }
    }
}

fn read_key_file(path: &Path) -> Result<DataKey, Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(Error::Key(format!(
                "{} is readable by other users (mode {:o}); make it 0600",
                path.display(),
                mode & 0o777
            )));
        }
    }
    let bytes = Zeroizing::new(std::fs::read(path)?);
    DataKey::from_slice(&bytes)
}
