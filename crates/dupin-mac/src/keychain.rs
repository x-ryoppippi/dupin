//! The storage data key as a generic password item in the user's Keychain.

use std::ptr::{self, NonNull};

use dupin_storage::{DataKey, Error, KeyProvider};
use objc2_core_foundation::{CFBoolean, CFData, CFDictionary, CFRetained, CFString, CFType};
use objc2_security::{
    SecItemAdd, SecItemCopyMatching, errSecDuplicateItem, errSecItemNotFound, errSecSuccess,
    kSecAttrAccessible, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, kSecAttrAccount, kSecAttrLabel,
    kSecAttrService, kSecAttrSynchronizable, kSecClass, kSecClassGenericPassword, kSecMatchLimit,
    kSecMatchLimitOne, kSecReturnData, kSecUseDataProtectionKeychain, kSecValueData,
};
use zeroize::Zeroizing;

const SERVICE: &str = "app.dupin.Dupin.storage";
const ACCOUNT: &str = "data-key";
const LABEL: &str = "Dupin storage key";
/// The data protection keychain needs an entitlement from a provisioning profile, which an
/// ad-hoc or Developer ID build without one lacks.
const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34018;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Keychain {
    /// Honors "after first unlock, this device only".
    DataProtection,
    /// The login keychain, for builds without the entitlement. It ignores the accessibility
    /// class; the item is still never synchronized.
    Login,
}

/// Reads the data key from the Keychain, creating it only when no item exists. A locked or
/// refused Keychain is an error, never a reason to make a new key.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeychainKey;

impl KeyProvider for KeychainKey {
    fn data_key(&self) -> Result<DataKey, Error> {
        if let Some(key) = find()? {
            return Ok(key);
        }
        let key = DataKey::generate()?;
        match add(&key) {
            Ok(()) => Ok(key),
            // Another process added it between our lookup and our add; use theirs.
            Err(status) if status == errSecDuplicateItem => find()?.ok_or_else(|| failure("find", status)),
            Err(status) => Err(failure("add", status)),
        }
    }
}

fn failure(operation: &str, status: i32) -> Error {
    Error::Key(format!("Keychain {operation} failed with OSStatus {status}"))
}

fn find() -> Result<Option<DataKey>, Error> {
    for keychain in [Keychain::DataProtection, Keychain::Login] {
        match copy_matching(keychain) {
            Ok(Some(bytes)) => return DataKey::from_slice(&bytes).map(Some),
            Ok(None) => {}
            Err(ERR_SEC_MISSING_ENTITLEMENT) => {}
            Err(status) => return Err(failure("read", status)),
        }
    }
    Ok(None)
}

fn add(key: &DataKey) -> Result<(), i32> {
    match add_to(Keychain::DataProtection, key) {
        Err(ERR_SEC_MISSING_ENTITLEMENT) => add_to(Keychain::Login, key),
        other => other,
    }
}

fn query(keychain: Keychain, extra: &[(&CFString, &CFType)]) -> CFRetained<CFDictionary<CFString, CFType>> {
    let service = CFString::from_str(SERVICE);
    let account = CFString::from_str(ACCOUNT);
    // SAFETY: These are immutable constants exported by Security.framework.
    let mut pairs: Vec<(&CFString, &CFType)> = unsafe {
        vec![
            (kSecClass, kSecClassGenericPassword.as_ref()),
            (kSecAttrService, service.as_ref()),
            (kSecAttrAccount, account.as_ref()),
            (kSecAttrSynchronizable, CFBoolean::new(false).as_ref()),
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

fn copy_matching(keychain: Keychain) -> Result<Option<Zeroizing<Vec<u8>>>, i32> {
    // SAFETY: As above.
    let query = unsafe {
        query(
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

fn add_to(keychain: Keychain, key: &DataKey) -> Result<(), i32> {
    let data = CFData::from_bytes(key.as_bytes());
    let label = CFString::from_str(LABEL);
    // SAFETY: As above.
    let attributes = unsafe {
        query(
            keychain,
            &[
                (kSecValueData, data.as_ref()),
                (kSecAttrLabel, label.as_ref()),
                (
                    kSecAttrAccessible,
                    kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly.as_ref(),
                ),
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
