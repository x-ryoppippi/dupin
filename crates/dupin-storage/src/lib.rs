//! Host-owned encrypted storage: each owner (a native feature now, an extension package
//! later) gets a namespace of JSON records and content-addressed blobs, sealed at rest with
//! XChaCha20-Poly1305 under a key from a [`KeyProvider`], and pruned to its [`Limits`].
//!
//! Every call blocks on file IO, so call it from a worker thread, never the UI thread. See
//! `docs/features/storage.md` for the on-disk format and the threat model.

mod crypto;
mod device;
mod error;
mod fs;
mod key;
mod secret;
mod store;
mod sweeper;

pub use device::DeviceId;
pub use error::Error;
pub use fs::scratch_process;
pub use key::{DataKey, FileKey, KeyProvider, MemoryKey};
pub use store::{
    BlobHandle, BlobRef, Clock, Cursor, Limits, NewRecord, OpenReport, Page, Record, RecordId, Storage,
    Store, SystemClock, Usage,
};
pub use sweeper::Sweeper;

/// The largest page [`Store::list`] returns.
pub(crate) const MAX_PAGE: usize = 200;
/// The most blobs one record may reference.
pub const MAX_BLOBS_PER_RECORD: usize = 64;

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0xf)] as char);
    }
    out
}

fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.as_bytes();
    if text.len() != N * 2 {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    for (index, [high, low]) in text.as_chunks::<2>().0.iter().enumerate() {
        out[index] = (digit(*high)? << 4) | digit(*low)?;
    }
    Some(out)
}

/// A namespace names one directory, so it is kept to a safe, portable file name. The broker
/// maps an extension's package ID onto one.
pub(crate) fn validate_namespace(namespace: &str) -> Result<(), Error> {
    let bytes = namespace.as_bytes();
    let valid = (1..=64).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidNamespace(namespace.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let bytes = [0x00, 0x7f, 0xa5, 0xff];
        assert_eq!(hex(&bytes), "007fa5ff");
        assert_eq!(unhex::<4>("007fa5ff"), Some(bytes));
        assert_eq!(unhex::<4>("007FA5FF"), None);
        assert_eq!(unhex::<4>("007fa5f"), None);
    }

    #[test]
    fn namespaces_are_plain_file_names() {
        for good in ["clipboard", "ext.github.user.repo", "a", "0-9_x"] {
            assert!(validate_namespace(good).is_ok(), "{good}");
        }
        for bad in ["", ".", "..", "-x", "Clip", "a/b", "a\\b", "a b", &"x".repeat(65)] {
            assert!(validate_namespace(bad).is_err(), "{bad}");
        }
    }
}
