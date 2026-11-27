//! Keys as generic password items in the user's Keychain: the storage data key, this Mac's own
//! copy of each sync vault key, and, in builds that can, the synchronizable vault key item
//! iCloud Keychain carries to the owner's other Macs.

use std::ptr::{self, NonNull};

use dupin_storage::{DataKey, Error, KeyProvider};
use dupin_sync::{LocalKeyStore, SharedKeyStore, account};
use objc2_core_foundation::{CFArray, CFBoolean, CFData, CFDictionary, CFRetained, CFString, CFType};
use objc2_security::{
    SecItemAdd, SecItemCopyMatching, SecItemDelete, SecTask, errSecDuplicateItem, errSecItemNotFound,
    errSecSuccess, kSecAttrAccessible, kSecAttrAccessibleAfterFirstUnlock,
    kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, kSecAttrAccount, kSecAttrLabel, kSecAttrService,
    kSecAttrSynchronizable, kSecClass, kSecClassGenericPassword, kSecMatchLimit, kSecMatchLimitOne,
    kSecReturnData, kSecUseDataProtectionKeychain, kSecValueData,
};
use zeroize::Zeroizing;

const STORAGE_SERVICE: &str = "app.dupin.Dupin.storage";
const DATA_KEY_ACCOUNT: &str = "data-key";
const SYNC_SERVICE: &str = "app.dupin.Dupin.sync";
/// The data protection keychain needs an entitlement from a provisioning profile, which an
/// ad-hoc or Developer ID build without one lacks.
const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34018;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Keychain {
    /// Honors the accessibility class, and is the only one that can synchronize.
    DataProtection,
    /// The login keychain, for builds without the entitlement. It ignores the accessibility
    /// class and never synchronizes.
    Login,
}

/// One generic password item.
struct Item<'a> {
    service: &'a str,
    account: &'a str,
    label: &'a str,
    /// Carried by iCloud Keychain; only ever in the data protection keychain.
    synchronizable: bool,
}

impl Item<'_> {
    fn keychains(&self) -> &'static [Keychain] {
        match self.synchronizable {
            true => &[Keychain::DataProtection],
            false => &[Keychain::DataProtection, Keychain::Login],
        }
    }

    fn query(
        &self,
        keychain: Keychain,
        extra: &[(&CFString, &CFType)],
    ) -> CFRetained<CFDictionary<CFString, CFType>> {
        let service = CFString::from_str(self.service);
        let account = CFString::from_str(self.account);
        // SAFETY: These are immutable constants exported by Security.framework.
        let mut pairs: Vec<(&CFString, &CFType)> = unsafe {
            vec![
                (kSecClass, kSecClassGenericPassword.as_ref()),
                (kSecAttrService, service.as_ref()),
                (kSecAttrAccount, account.as_ref()),
                (
                    kSecAttrSynchronizable,
                    CFBoolean::new(self.synchronizable).as_ref(),
                ),
            ]
        };
        if keychain == Keychain::DataProtection {
            // SAFETY: As above.
            pairs.push(unsafe { (kSecUseDataProtectionKeychain, CFBoolean::new(true).as_ref()) });
        }
        pairs.extend_from_slice(extra);
        let (keys, values): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
        CFDictionary::from_slices(&keys, &values)
    }

    fn find(&self) -> Result<Option<Zeroizing<Vec<u8>>>, i32> {
        for &keychain in self.keychains() {
            match self.copy_matching(keychain) {
                Ok(Some(bytes)) => return Ok(Some(bytes)),
                Ok(None) | Err(ERR_SEC_MISSING_ENTITLEMENT) => {}
                Err(status) => return Err(status),
            }
        }
        Ok(None)
    }

    fn copy_matching(&self, keychain: Keychain) -> Result<Option<Zeroizing<Vec<u8>>>, i32> {
        // SAFETY: As above.
        let query = unsafe {
            self.query(
                keychain,
                &[
                    (kSecReturnData, CFBoolean::new(true).as_ref()),
                    (kSecMatchLimit, kSecMatchLimitOne.as_ref()),
                ],
            )
        };
        let mut result: *const CFType = ptr::null();
        // SAFETY: The query is a valid dictionary and `result` a valid out pointer; with
        // kSecReturnData and kSecMatchLimitOne the result is a CFData we own (Create rule).
        let status = unsafe { SecItemCopyMatching(query.as_opaque(), &mut result) };
        if status == errSecItemNotFound {
            return Ok(None);
        }
        if status != errSecSuccess {
            return Err(status);
        }
        let Some(result) = NonNull::new(result.cast_mut()) else {
            return Ok(None);
        };
        // SAFETY: Owned per the Create rule, as above.
        let result = unsafe { CFRetained::from_raw(result) };
        let data = result.downcast_ref::<CFData>().ok_or(errSecItemNotFound)?;
        Ok(Some(Zeroizing::new(data.to_vec())))
    }

    fn add(&self, bytes: &[u8]) -> Result<(), i32> {
        let mut last = ERR_SEC_MISSING_ENTITLEMENT;
        for &keychain in self.keychains() {
            match self.add_to(keychain, bytes) {
                Err(ERR_SEC_MISSING_ENTITLEMENT) => last = ERR_SEC_MISSING_ENTITLEMENT,
                other => return other,
            }
        }
        Err(last)
    }

    fn add_to(&self, keychain: Keychain, bytes: &[u8]) -> Result<(), i32> {
        let data = CFData::from_bytes(bytes);
        let label = CFString::from_str(self.label);
        // SAFETY: As above.
        let accessible = unsafe {
            match self.synchronizable {
                true => kSecAttrAccessibleAfterFirstUnlock,
                false => kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            }
        };
        // SAFETY: As above.
        let attributes = unsafe {
            self.query(
                keychain,
                &[
                    (kSecValueData, data.as_ref()),
                    (kSecAttrLabel, label.as_ref()),
                    (kSecAttrAccessible, accessible.as_ref()),
                ],
            )
        };
        // SAFETY: A valid attributes dictionary; no result is requested.
        let status = unsafe { SecItemAdd(attributes.as_opaque(), ptr::null_mut()) };
        if status == errSecSuccess {
            Ok(())
        } else {
            Err(status)
        }
    }

    fn delete(&self) -> Result<(), i32> {
        for &keychain in self.keychains() {
            let query = self.query(keychain, &[]);
            // SAFETY: A valid query dictionary.
            let status = unsafe { SecItemDelete(query.as_opaque()) };
            match status {
                _ if status == errSecSuccess || status == errSecItemNotFound => {}
                ERR_SEC_MISSING_ENTITLEMENT => {}
                status => return Err(status),
            }
        }
        Ok(())
    }
}

fn failure(operation: &str, status: i32) -> Error {
    Error::Key(format!("Keychain {operation} failed with OSStatus {status}"))
}

fn sync_failure(operation: &str, status: i32) -> dupin_sync::Error {
    dupin_sync::Error::KeyStore(format!("Keychain {operation} failed with OSStatus {status}"))
}

/// Reads the data key from the Keychain, creating it only when no item exists. A locked or
/// refused Keychain is an error, never a reason to make a new key.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeychainKey;

const DATA_KEY: Item<'static> = Item {
    service: STORAGE_SERVICE,
    account: DATA_KEY_ACCOUNT,
    label: "Dupin storage key",
    synchronizable: false,
};

impl KeyProvider for KeychainKey {
    fn data_key(&self) -> Result<DataKey, Error> {
        if let Some(bytes) = DATA_KEY.find().map_err(|status| failure("read", status))? {
            return DataKey::from_slice(&bytes);
        }
        let key = DataKey::generate()?;
        match DATA_KEY.add(key.as_bytes()) {
            Ok(()) => Ok(key),
            // Another process added it between our lookup and our add; use theirs.
            Err(status) if status == errSecDuplicateItem => DATA_KEY
                .find()
                .map_err(|status| failure("read", status))?
                .ok_or_else(|| failure("find", status))
                .and_then(|bytes| DataKey::from_slice(&bytes)),
            Err(status) => Err(failure("add", status)),
        }
    }
}

/// This Mac's own copy of each vault key epoch, `vault-<vault id>-<epoch>.local`: never
/// synchronizable, this device only where the build can say so.
#[derive(Debug, Default, Clone, Copy)]
pub struct VaultKeychain;

fn put(item: &Item<'_>, key: &DataKey) -> Result<(), dupin_sync::Error> {
    match item.add(key.as_bytes()) {
        Ok(()) => Ok(()),
        Err(status) if status == errSecDuplicateItem => {
            let stored = item.find().map_err(|status| sync_failure("read", status))?;
            match stored {
                Some(bytes) if same_key(&bytes, key.as_bytes()) => Ok(()),
                _ => Err(sync_failure("add", status)),
            }
        }
        Err(status) => Err(sync_failure("add", status)),
    }
}

/// Compares two keys in time that doesn't depend on where they differ.
fn same_key(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn get(item: &Item<'_>) -> Result<Option<DataKey>, dupin_sync::Error> {
    match item.find().map_err(|status| sync_failure("read", status))? {
        Some(bytes) => Ok(Some(DataKey::from_slice(&bytes)?)),
        None => Ok(None),
    }
}

impl LocalKeyStore for VaultKeychain {
    fn load(&self, vault: [u8; 16], epoch: u32) -> Result<Option<DataKey>, dupin_sync::Error> {
        get(&local_item(&format!("{}.local", account(vault, epoch))))
    }

    fn store(&self, vault: [u8; 16], epoch: u32, key: &DataKey) -> Result<(), dupin_sync::Error> {
        put(&local_item(&format!("{}.local", account(vault, epoch))), key)
    }

    fn delete(&self, vault: [u8; 16], epoch: u32) -> Result<(), dupin_sync::Error> {
        local_item(&format!("{}.local", account(vault, epoch)))
            .delete()
            .map_err(|status| sync_failure("delete", status))
    }
}

fn local_item(account: &str) -> Item<'_> {
    Item {
        service: SYNC_SERVICE,
        account,
        label: "Dupin sync key (this Mac)",
        synchronizable: false,
    }
}

fn shared_item(account: &str) -> Item<'_> {
    Item {
        service: SYNC_SERVICE,
        account,
        label: "Dupin sync key",
        synchronizable: true,
    }
}

/// The synchronizable vault key item, `vault-<vault id>-<epoch>`, end-to-end encrypted by
/// iCloud Keychain. Only a build signed with a keychain access group, which needs a
/// provisioning profile, can keep one; [`ICloudKeychain::available`] says whether this one is.
#[derive(Debug, Clone, Copy)]
pub struct ICloudKeychain(());

impl ICloudKeychain {
    /// `Some` when this process carries the `keychain-access-groups` entitlement. Ad hoc, Nix
    /// and Developer ID builds without a provisioning profile don't, and use a passphrase.
    pub fn available() -> Option<Self> {
        // SAFETY: Creates a task reference for this process; `None` on failure.
        let task = unsafe { SecTask::from_self(None) }?;
        let name = CFString::from_str("keychain-access-groups");
        // SAFETY: A valid task and string; a null error pointer is allowed.
        let value = unsafe { task.value_for_entitlement(&name, ptr::null_mut()) }?;
        let groups = value.downcast_ref::<CFArray>()?;
        (groups.count() > 0).then_some(Self(()))
    }
}

impl SharedKeyStore for ICloudKeychain {
    fn load(&self, vault: [u8; 16], epoch: u32) -> Result<Option<DataKey>, dupin_sync::Error> {
        get(&shared_item(&account(vault, epoch)))
    }

    fn store(&self, vault: [u8; 16], epoch: u32, key: &DataKey) -> Result<(), dupin_sync::Error> {
        put(&shared_item(&account(vault, epoch)), key)
    }

    fn delete(&self, vault: [u8; 16], epoch: u32) -> Result<(), dupin_sync::Error> {
        shared_item(&account(vault, epoch))
            .delete()
            .map_err(|status| sync_failure("delete", status))
    }
}
