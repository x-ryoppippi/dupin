//! Turning sync on: creating a vault in an empty folder, or joining the one already there.

use dupin_storage::{DeviceId, VaultKey};

use crate::error::{Error, Refusal};
use crate::folder::Folder;
use crate::keys::{LocalKeyStore, SharedKeyStore};
use crate::passphrase::Passphrase;
use crate::vault::{KdfParams, KeyMode, VaultHeader, new_vault_key, open_winner};

/// A vault this Mac holds the key to.
#[derive(Debug, Clone)]
pub struct Joined {
    pub key: VaultKey,
    pub header: VaultHeader,
}

/// Creates a vault in a folder that holds none. The key is wrapped under `passphrase`, which
/// must have been generated or checked, and, where the build has iCloud Keychain, also stored
/// as a synchronizable item. This Mac keeps its own local copy either way.
pub fn create(
    folder: &Folder,
    device: DeviceId,
    passphrase: &Passphrase,
    kdf: KdfParams,
    local: &dyn LocalKeyStore,
    shared: Option<&dyn SharedKeyStore>,
) -> Result<Joined, Error> {
    if !passphrase.is_checked() {
        return Err(Error::KeyUnavailable);
    }
    if !folder.parsed_headers()?.is_empty() {
        return Err(Error::Refused(Refusal::HoldsAVault));
    }
    let key = new_vault_key()?;
    let mode = match shared {
        Some(_) => KeyMode::Keychain,
        None => KeyMode::Passphrase,
    };
    let header = VaultHeader::new(&key, mode, Some((passphrase, kdf)))?;
    local.store(key.id(), key.epoch(), key.key())?;
    if let Err(error) = folder.create_header(&header, device) {
        local.delete(key.id(), key.epoch())?;
        return Err(error);
    }
    if let Some(shared) = shared {
        shared.store(key.id(), key.epoch(), key.key())?;
    }
    Ok(Joined { key, header })
}

/// Joins the vault in `folder`, the one with the lowest ID if sync tools left several: with
/// the synchronizable item when it has reached this Mac, otherwise with the passphrase. A
/// wrong passphrase changes nothing.
pub fn join(
    folder: &Folder,
    passphrase: Option<&Passphrase>,
    local: &dyn LocalKeyStore,
    shared: Option<&dyn SharedKeyStore>,
) -> Result<Joined, Error> {
    let headers = folder.parsed_headers()?;
    let (key, header) = open_winner(&headers, passphrase, shared)?;
    local.store(key.id(), key.epoch(), key.key())?;
    Ok(Joined { key, header })
}

/// The vault key this Mac joined earlier, from its local copy, checked against the folder's
/// header when the folder is available.
pub fn resume(
    vault: [u8; 16],
    epoch: u32,
    folder: Option<&Folder>,
    local: &dyn LocalKeyStore,
) -> Result<Joined, Error> {
    let key = local.load(vault, epoch)?.ok_or(Error::KeyUnavailable)?;
    let key = VaultKey::new(vault, epoch, key)?;
    let Some(folder) = folder else {
        return Err(Error::NoVault);
    };
    let header = folder
        .parsed_headers()?
        .into_iter()
        .find(|header| header.verify(&key))
        .ok_or(Error::NoVault)?;
    Ok(Joined { key, header })
}
