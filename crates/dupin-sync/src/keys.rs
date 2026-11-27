//! Where a Mac keeps the vault key: its own local copy, which never leaves it, and, in builds
//! that can, a synchronizable iCloud Keychain item that carries the key to the owner's other
//! Macs. Platform adapters implement these; tests use [`MemoryKeys`].

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::Mutex;

use dupin_storage::DataKey;

use crate::error::Error;
use crate::fsx::hex;

/// This Mac's own copy of each vault key epoch it has joined: `ThisDeviceOnly`, never
/// synchronizable, so losing the shared item later never locks local data.
pub trait LocalKeyStore: Send + Sync {
    fn load(&self, vault: [u8; 16], epoch: u32) -> Result<Option<DataKey>, Error>;
    fn store(&self, vault: [u8; 16], epoch: u32, key: &DataKey) -> Result<(), Error>;
    fn delete(&self, vault: [u8; 16], epoch: u32) -> Result<(), Error>;
}

/// The synchronizable item, `vault-<vault id>-<epoch>`, end-to-end encrypted by iCloud
/// Keychain. Only a build with a keychain access group entitlement has one.
pub trait SharedKeyStore: Send + Sync {
    fn load(&self, vault: [u8; 16], epoch: u32) -> Result<Option<DataKey>, Error>;
    fn store(&self, vault: [u8; 16], epoch: u32, key: &DataKey) -> Result<(), Error>;
    fn delete(&self, vault: [u8; 16], epoch: u32) -> Result<(), Error>;
}

pub fn account(vault: [u8; 16], epoch: u32) -> String {
    format!("vault-{}-{epoch}", hex(&vault))
}

/// Keys in memory for tests; it stands in for either store.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct MemoryKeys(Mutex<BTreeMap<String, DataKey>>);

#[cfg(test)]
impl MemoryKeys {
    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, DataKey>> {
        self.0.lock().unwrap_or_else(|poison| poison.into_inner())
    }
}

#[cfg(test)]
impl LocalKeyStore for MemoryKeys {
    fn load(&self, vault: [u8; 16], epoch: u32) -> Result<Option<DataKey>, Error> {
        Ok(self.map().get(&account(vault, epoch)).cloned())
    }

    fn store(&self, vault: [u8; 16], epoch: u32, key: &DataKey) -> Result<(), Error> {
        self.map().insert(account(vault, epoch), key.clone());
        Ok(())
    }

    fn delete(&self, vault: [u8; 16], epoch: u32) -> Result<(), Error> {
        self.map().remove(&account(vault, epoch));
        Ok(())
    }
}

#[cfg(test)]
impl SharedKeyStore for MemoryKeys {
    fn load(&self, vault: [u8; 16], epoch: u32) -> Result<Option<DataKey>, Error> {
        LocalKeyStore::load(self, vault, epoch)
    }

    fn store(&self, vault: [u8; 16], epoch: u32, key: &DataKey) -> Result<(), Error> {
        LocalKeyStore::store(self, vault, epoch, key)
    }

    fn delete(&self, vault: [u8; 16], epoch: u32) -> Result<(), Error> {
        LocalKeyStore::delete(self, vault, epoch)
    }
}
