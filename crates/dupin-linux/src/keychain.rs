//! The data key for encrypted storage. Linux has no keyring support yet, so storage that
//! needs the key can't open.

use dupin_storage::{DataKey, Error, KeyProvider};

#[derive(Debug, Default, Clone, Copy)]
pub struct KeychainKey;

impl KeyProvider for KeychainKey {
    fn data_key(&self) -> Result<DataKey, Error> {
        Err(Error::Key(
            "encrypted storage isn't supported on Linux yet".into(),
        ))
    }
}
