//! What belongs to the device rather than to a namespace: its ID and the hybrid logical clock
//! that stamps every version it writes, kept in `storage.state` beside the storage root.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crypto::{ID_LEN, KEY_TAG_LEN, KeySource, NONCE_LEN, StateKeys, VERSION};
use crate::fs::{Staged, create_new_private, create_private_dir};
use crate::{Error, hex, secret, unhex};

const STATE_MAGIC: &[u8; 4] = b"DUPS";

/// A random 128-bit name for this device. It is not a secret; it orders ties between
/// versions and, with sync, names the device's directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(pub(crate) [u8; ID_LEN]);

impl DeviceId {
    fn generate() -> Result<Self, Error> {
        let mut bytes = [0u8; ID_LEN];
        getrandom::fill(&mut bytes).map_err(|error| Error::Io(io::Error::other(error)))?;
        Ok(Self(bytes))
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}

impl FromStr for DeviceId {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        unhex::<ID_LEN>(text).map(Self).ok_or(Error::NotFound)
    }
}

impl Serialize for DeviceId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <&str>::deserialize(deserializer)?;
        text.parse()
            .map_err(|_| serde::de::Error::custom("invalid device ID"))
    }
}

/// A version's stamp: the hybrid logical clock of the device that wrote it, then that
/// device's ID, which breaks ties so every device picks the same newest version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Stamp {
    /// Unix milliseconds in the upper 48 bits, a counter in the lower 16.
    pub hlc: u64,
    pub device: DeviceId,
}

const COUNTER_BITS: u32 = 16;

fn hlc_from_millis(millis: u64) -> u64 {
    millis
        .checked_shl(COUNTER_BITS)
        .filter(|hlc| hlc >> COUNTER_BITS == millis)
        .unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StateFile {
    device_id: DeviceId,
    /// A hash of the hardware identity the ID was minted on, when the host supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hardware: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_device_id: Option<DeviceId>,
    hlc: u64,
    /// Per namespace, the stamp this device left sync at.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    left_at: BTreeMap<String, Stamp>,
    /// Fields a newer Dupin wrote, kept so rewriting the file never drops them.
    #[serde(flatten)]
    newer: serde_json::Map<String, serde_json::Value>,
}

impl StateFile {
    fn fresh(
        hardware: Option<String>,
        previous_device_id: Option<DeviceId>,
        hlc: u64,
    ) -> Result<Self, Error> {
        Ok(Self {
            device_id: DeviceId::generate()?,
            hardware,
            previous_device_id,
            hlc,
            left_at: BTreeMap::new(),
            newer: serde_json::Map::new(),
        })
    }
}

pub(crate) fn hardware_hash(identity: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"dupin-storage hardware\0");
    hash.update(identity);
    hex(&hash.finalize())
}

/// The device's state, shared by every namespace a [`crate::Storage`] opens.
pub(crate) struct Device {
    path: PathBuf,
    keys: StateKeys,
    state: Mutex<StateFile>,
}

impl Device {
    /// Loads `storage.state`, or creates it. Call only once a namespace's key check has
    /// proved the data key, so a wrong key never replaces the state.
    pub fn load(path: PathBuf, keys: StateKeys, hardware: Option<String>) -> Result<Self, Error> {
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        let state = match read(&path, &keys)? {
            Some(Ok(mut state)) => {
                let moved =
                    matches!((&state.hardware, &hardware), (Some(stored), Some(now)) if stored != now);
                if moved {
                    state = StateFile::fresh(hardware, Some(state.device_id), state.hlc)?;
                    write(&path, &keys, &state)?;
                } else if state.hardware.is_none() && hardware.is_some() {
                    state.hardware = hardware;
                    write(&path, &keys, &state)?;
                }
                state
            }
            Some(Err(Damaged)) => {
                let state = StateFile::fresh(hardware, None, 0)?;
                write(&path, &keys, &state)?;
                state
            }
            None => {
                let state = StateFile::fresh(hardware, None, 0)?;
                match create_new_private(&path, &encode(&keys, &state)?) {
                    Ok(()) => state,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => match read(&path, &keys)? {
                        Some(Ok(theirs)) => theirs,
                        _ => return Err(Error::Corrupt("storage.state".into())),
                    },
                    Err(error) => return Err(error.into()),
                }
            }
        };
        Ok(Self {
            path,
            keys,
            state: Mutex::new(state),
        })
    }

    fn lock(&self) -> MutexGuard<'_, StateFile> {
        self.state.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn id(&self) -> DeviceId {
        self.lock().device_id
    }

    #[cfg(test)]
    fn previous_id(&self) -> Option<DeviceId> {
        self.lock().previous_device_id
    }

    /// A stamp for a local write: later than every stamp this device has written or seen,
    /// and no earlier than the wall clock. The clock is saved before the stamp is used, so a
    /// restart with the wall clock set back never stamps below it.
    pub fn stamp(&self, now_millis: u64) -> Result<Stamp, Error> {
        let mut state = self.lock();
        let hlc = state.hlc.saturating_add(1).max(hlc_from_millis(now_millis));
        let mut next = state.clone();
        next.hlc = hlc;
        write(&self.path, &self.keys, &next)?;
        *state = next;
        Ok(Stamp {
            hlc,
            device: state.device_id,
        })
    }

    /// Moves the clock past a stamp found on disk.
    pub fn observe(&self, hlc: u64) -> Result<(), Error> {
        let mut state = self.lock();
        if hlc <= state.hlc {
            return Ok(());
        }
        let mut next = state.clone();
        next.hlc = hlc;
        write(&self.path, &self.keys, &next)?;
        *state = next;
        Ok(())
    }
}

struct Damaged;

fn read(path: &Path, keys: &StateKeys) -> Result<Option<Result<StateFile, Damaged>>, Error> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let opened = (|| {
        let rest = bytes
            .strip_prefix(STATE_MAGIC.as_slice())?
            .strip_prefix(&[VERSION])?;
        let (tag, rest) = rest.split_at_checked(KEY_TAG_LEN)?;
        if KeySource::from_tag(tag.try_into().ok()?)? != KeySource::Device {
            return None;
        }
        let (nonce, ciphertext) = rest.split_at_checked(NONCE_LEN)?;
        let plain = keys.open(nonce.try_into().ok()?, &keys.aad(), ciphertext.to_vec())?;
        serde_json::from_slice::<StateFile>(&plain).ok()
    })();
    Ok(Some(opened.ok_or(Damaged)))
}

fn encode(keys: &StateKeys, state: &StateFile) -> Result<Vec<u8>, Error> {
    let plain = secret::to_json(state)?;
    let (nonce, ciphertext) = keys.seal(&keys.aad(), &plain)?;
    let mut out = Vec::with_capacity(5 + KEY_TAG_LEN + NONCE_LEN + ciphertext.len());
    out.extend_from_slice(STATE_MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&KeySource::Device.tag());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn write(path: &Path, keys: &StateKeys, state: &StateFile) -> Result<(), Error> {
    let dir = path.parent().unwrap_or(Path::new("."));
    Staged::write(dir, &encode(keys, state)?)?.commit(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataKey;

    fn keys() -> StateKeys {
        StateKeys::derive(&DataKey::from_slice(&[9; 32]).unwrap())
    }

    #[test]
    fn hlc_carries_the_counter_into_milliseconds_and_never_goes_back() {
        let dir = tempfile::tempdir().unwrap();
        let device = Device::load(dir.path().join("storage.state"), keys(), None).unwrap();
        let first = device.stamp(1_000).unwrap();
        assert_eq!(first.hlc, 1_000 << COUNTER_BITS);
        let second = device.stamp(1_000).unwrap();
        assert_eq!(second.hlc, first.hlc + 1);
        assert_eq!(device.stamp(10).unwrap().hlc, first.hlc + 2);
        device.observe(5_000 << COUNTER_BITS | 0xffff).unwrap();
        assert_eq!(device.stamp(10).unwrap().hlc, 5_001 << COUNTER_BITS);
        assert_eq!(device.stamp(9_000).unwrap().hlc, 9_000 << COUNTER_BITS);
        assert_eq!(hlc_from_millis(u64::MAX), u64::MAX);
    }

    #[test]
    fn the_clock_and_id_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.state");
        let device = Device::load(path.clone(), keys(), None).unwrap();
        let stamp = device.stamp(1_000_000).unwrap();
        drop(device);
        let device = Device::load(path, keys(), None).unwrap();
        assert_eq!(device.id(), stamp.device);
        assert!(device.stamp(1).unwrap() > stamp);
    }

    #[test]
    fn other_hardware_mints_a_new_id_and_remembers_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.state");
        let unrecorded = Device::load(path.clone(), keys(), None).unwrap().id();
        let here = Some(hardware_hash(b"machine-a"));
        let first = Device::load(path.clone(), keys(), here.clone()).unwrap();
        assert_eq!(
            first.id(),
            unrecorded,
            "recording the hardware the first time keeps the ID"
        );
        let stamp = first.stamp(500).unwrap();
        drop(first);
        assert_eq!(Device::load(path.clone(), keys(), here).unwrap().id(), unrecorded);

        let moved = Device::load(path.clone(), keys(), Some(hardware_hash(b"machine-b"))).unwrap();
        assert_ne!(moved.id(), unrecorded);
        assert_eq!(moved.previous_id(), Some(unrecorded));
        assert!(moved.stamp(1).unwrap().hlc > stamp.hlc);
        assert_eq!(Device::load(path, keys(), None).unwrap().id(), moved.id());
    }

    #[test]
    fn a_damaged_state_is_replaced_and_unknown_fields_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.state");
        std::fs::write(&path, b"garbage").unwrap();
        let device = Device::load(path.clone(), keys(), None).unwrap();
        let id = device.id();
        drop(device);

        let mut state = match read(&path, &keys()).unwrap() {
            Some(Ok(state)) => state,
            _ => panic!("the state was rewritten"),
        };
        state
            .newer
            .insert("from_the_future".into(), serde_json::json!(42));
        write(&path, &keys(), &state).unwrap();
        Device::load(path.clone(), keys(), None)
            .unwrap()
            .stamp(1)
            .unwrap();
        let Some(Ok(state)) = read(&path, &keys()).unwrap() else {
            panic!("the state opens");
        };
        assert_eq!(state.device_id, id);
        assert_eq!(state.newer["from_the_future"], 42);
    }
}
