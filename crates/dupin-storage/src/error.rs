use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The key provider couldn't supply the data key.
    Key(String),
    /// The namespace was written with another data key. Nothing was read or removed.
    WrongKey,
    /// The stored data was written by a newer Dupin in a format this build can't read.
    /// Nothing was read or removed.
    NewerFormat,
    /// Another `Store` for this namespace is open, in this process or another.
    Busy,
    InvalidNamespace(String),
    InvalidLimits(&'static str),
    /// A record, or one of its blobs, is larger than the namespace allows.
    TooLarge {
        bytes: u64,
        limit: u64,
    },
    TooManyBlobs {
        count: usize,
        limit: usize,
    },
    /// The item failed authentication or couldn't be parsed; it has been dropped.
    Corrupt(String),
    NotFound,
    Json(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(error) => write!(f, "storage I/O failed: {error}"),
            Error::Key(error) => write!(f, "couldn't get the storage key: {error}"),
            Error::WrongKey => f.write_str("the stored data was sealed with a different key"),
            Error::NewerFormat => f.write_str("the stored data was written by a newer version of Dupin"),
            Error::Busy => f.write_str("the storage namespace is already open"),
            Error::InvalidNamespace(namespace) => write!(f, "{namespace:?} is not a valid storage namespace"),
            Error::InvalidLimits(why) => write!(f, "invalid storage limits: {why}"),
            Error::TooLarge { bytes, limit } => write!(f, "{bytes} bytes is over the {limit}-byte limit"),
            Error::TooManyBlobs { count, limit } => write!(f, "{count} blobs is over the limit of {limit}"),
            Error::Corrupt(what) => write!(f, "{what} is corrupt and was dropped"),
            Error::NotFound => f.write_str("no such stored item"),
            Error::Json(error) => write!(f, "couldn't encode the record: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(error) => Some(error),
            Error::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Error::Json(error)
    }
}
