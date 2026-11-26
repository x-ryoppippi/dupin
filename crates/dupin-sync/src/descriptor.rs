//! A device's descriptor, `devices/<device id>.dev`: sealed under the vault key, written only by
//! that device. That it opens proves the device holds the same vault key as this Mac.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use dupin_storage::{DeviceId, RecordId, Stamp, VaultKey};

use crate::seal::{self, NONCE_LEN, length_prefixed};

const MAGIC: &[u8; 4] = b"DUPD";
pub const FORMAT: u32 = 1;
const FILE_FORMAT: u8 = 1;
/// Acknowledgements for many tombstones still fit, and a tampered file can't make Dupin read
/// much more.
pub const MAX_DESCRIPTOR_BYTES: u64 = 8 * 1024 * 1024;

/// A tombstone this device has opened and applied: its record and stamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ack {
    pub id: RecordId,
    pub stamp: Stamp,
}

impl Serialize for Ack {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!(
            "{}:{:016x}:{}",
            self.id, self.stamp.hlc, self.stamp.device
        ))
    }
}

impl<'de> Deserialize<'de> for Ack {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <&str>::deserialize(deserializer)?;
        let invalid = || serde::de::Error::custom("invalid acknowledgement");
        let mut parts = text.split(':');
        let (Some(id), Some(hlc), Some(device), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid());
        };
        if hlc.len() != 16 {
            return Err(invalid());
        }
        Ok(Ack {
            id: id.parse().map_err(|_| invalid())?,
            stamp: Stamp {
                hlc: u64::from_str_radix(hlc, 16).map_err(|_| invalid())?,
                device: device.parse().map_err(|_| invalid())?,
            },
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Descriptor {
    pub format: u32,
    /// The Mac's name, such as "Work MacBook Pro".
    pub name: String,
    pub platform: String,
    pub version: String,
    /// The key epoch the device has re-sealed to.
    pub epoch: u32,
    /// Namespace IDs the device takes part in.
    #[serde(default)]
    pub namespaces: BTreeSet<String>,
    /// The device's hybrid logical clock when it last finished a full import.
    pub seen_at: u64,
    #[serde(default)]
    pub acks: BTreeMap<String, BTreeSet<Ack>>,
    #[serde(default)]
    pub leaving: BTreeSet<String>,
    /// Per namespace ID, the leaving or retired devices whose files this one has taken over.
    #[serde(default)]
    pub adopted: BTreeMap<String, BTreeSet<DeviceId>>,
    /// Devices removed with Remove This Mac, and the clock of the device that removed them.
    #[serde(default)]
    pub retired: BTreeMap<DeviceId, u64>,
    /// Fields a newer Dupin wrote.
    #[serde(flatten)]
    pub newer: serde_json::Map<String, serde_json::Value>,
}

fn aad(vault: &VaultKey, device: DeviceId) -> Vec<u8> {
    let epoch = vault.epoch().to_le_bytes();
    let id = vault.id();
    let device = device.to_string();
    length_prefixed(&[
        b"dupin-descriptor",
        &[FILE_FORMAT],
        &id,
        &epoch,
        device.as_bytes(),
    ])
}

impl Descriptor {
    pub fn seal(&self, vault: &VaultKey, device: DeviceId) -> Vec<u8> {
        let key = seal::derive(vault, "device-descriptor");
        let plain = zeroize::Zeroizing::new(serde_json::to_vec(self).expect("a descriptor serializes"));
        let (nonce, ciphertext) = seal::seal(&key, &aad(vault, device), &plain);
        let mut out = MAGIC.to_vec();
        out.push(FILE_FORMAT);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        out
    }

    /// `None` when the file was not written by `device` under this vault key, or was altered.
    pub fn open(bytes: &[u8], vault: &VaultKey, device: DeviceId) -> Option<Self> {
        let rest = bytes
            .strip_prefix(MAGIC.as_slice())?
            .strip_prefix(&[FILE_FORMAT])?;
        let (nonce, ciphertext) = rest.split_at_checked(NONCE_LEN)?;
        let nonce: [u8; NONCE_LEN] = nonce.try_into().ok()?;
        let key = seal::derive(vault, "device-descriptor");
        let plain = seal::open(&key, &nonce, &aad(vault, device), ciphertext)?;
        serde_json::from_slice(&plain).ok()
    }

    pub fn takes_part(&self, namespace: &str) -> bool {
        self.namespaces.contains(namespace) && !self.leaving.contains(namespace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> VaultKey {
        VaultKey::new([3; 16], 1, dupin_storage::DataKey::generate().unwrap()).unwrap()
    }

    #[test]
    fn opens_only_for_its_device_and_vault() {
        let vault = vault();
        let me: DeviceId = "0123456789abcdef0123456789abcdef".parse().unwrap();
        let other: DeviceId = "fedcba9876543210fedcba9876543210".parse().unwrap();
        let mut descriptor = Descriptor {
            format: FORMAT,
            name: "Mac".into(),
            ..Descriptor::default()
        };
        descriptor.acks.entry("ns".into()).or_default().insert(Ack {
            id: "0".repeat(32).parse().unwrap(),
            stamp: Stamp { hlc: 7, device: me },
        });
        let sealed = descriptor.seal(&vault, me);
        assert_eq!(Descriptor::open(&sealed, &vault, me), Some(descriptor));
        assert_eq!(
            Descriptor::open(&sealed, &vault, other),
            None,
            "renamed to another device"
        );
        let other_vault = self::vault();
        assert_eq!(Descriptor::open(&sealed, &other_vault, me), None);
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(Descriptor::open(&tampered, &vault, me), None);
        assert_eq!(Descriptor::open(&sealed[..sealed.len() - 1], &vault, me), None);
        assert!(!String::from_utf8_lossy(&sealed).contains("Mac"));
    }
}
