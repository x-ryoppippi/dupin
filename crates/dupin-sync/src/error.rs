use std::fmt;
use std::io;

/// Why a folder can't hold a vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The folder is Dupin's data directory or inside it.
    InsideDataDirectory,
    /// The folder holds Dupin's data directory, which would put the local store and the
    /// plaintext settings file in the synced folder.
    ContainsDataDirectory,
    /// A network file system without atomic rename (SMB, AFP, WebDAV).
    NetworkFileSystem(String),
    /// NFS, refused unless the user overrides it.
    Nfs,
    NotADirectory,
    /// The folder already holds a vault; join it instead of creating another.
    HoldsAVault,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::InsideDataDirectory => f.write_str("the folder is inside Dupin's own data folder"),
            Refusal::ContainsDataDirectory => f.write_str("the folder contains Dupin's own data folder"),
            Refusal::NetworkFileSystem(kind) => write!(
                f,
                "the folder is on a {kind} network share, which can't replace files in one step"
            ),
            Refusal::Nfs => f.write_str("the folder is on NFS; turn on the NFS override to use it anyway"),
            Refusal::NotADirectory => f.write_str("the path is not a folder"),
            Refusal::HoldsAVault => f.write_str("the folder already holds a sync vault; join it instead"),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Storage(dupin_storage::Error),
    Refused(Refusal),
    /// No vault header in the folder, or none has arrived yet.
    NoVault,
    /// A vault header that can't be parsed or doesn't authenticate under the key.
    BadHeader(String),
    /// The header asks for key derivation weaker than Dupin accepts, or absurdly expensive.
    UnacceptableKdf(String),
    WrongPassphrase,
    /// Neither the shared Keychain item nor a passphrase can open the vault.
    KeyUnavailable,
    /// The vault or a file in it was written by a newer Dupin.
    NewerFormat,
    /// A platform key store failed.
    KeyStore(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(error) => write!(f, "sync folder I/O failed: {error}"),
            Error::Storage(error) => write!(f, "{error}"),
            Error::Refused(why) => write!(f, "can't sync to this folder: {why}"),
            Error::NoVault => f.write_str("the folder holds no sync vault yet"),
            Error::BadHeader(why) => write!(f, "the vault header is damaged or tampered with: {why}"),
            Error::UnacceptableKdf(why) => write!(f, "the vault header's key derivation is refused: {why}"),
            Error::WrongPassphrase => f.write_str("the passphrase doesn't open this vault"),
            Error::KeyUnavailable => f.write_str("the vault key isn't available on this Mac yet"),
            Error::NewerFormat => f.write_str("the sync folder was written by a newer version of Dupin"),
            Error::KeyStore(why) => write!(f, "couldn't use the vault key store: {why}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}

impl From<dupin_storage::Error> for Error {
    fn from(error: dupin_storage::Error) -> Self {
        Error::Storage(error)
    }
}
