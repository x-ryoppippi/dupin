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
    /// Loads `storage.state`, or creates it. A state that is well formed but fails to open
    /// may be sealed under another data key, so it is replaced only when `replaceable` says
    /// nothing sealed under another key is left to come back to; otherwise the key is wrong.
    /// A state in a newer format is never this build's to replace.
    pub fn load(
        path: PathBuf,
        keys: StateKeys,
        hardware: Option<String>,
        replaceable: impl FnOnce() -> Result<bool, Error>,
    ) -> Result<Self, Error> {
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        let state = match read(&path, &keys)? {
            Read::Opened(mut state) => {
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
            Read::Newer => return Err(Error::NewerFormat),
            Read::Unopened if !replaceable()? => return Err(Error::WrongKey),
            Read::Damaged | Read::Unopened => {
                let state = StateFile::fresh(hardware, None, 0)?;
                write(&path, &keys, &state)?;
                state
            }
            Read::Missing => {
                let state = StateFile::fresh(hardware, None, 0)?;
                match create_new_private(&path, &encode(&keys, &state)?) {
                    Ok(()) => state,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => match read(&path, &keys)? {
                        Read::Opened(theirs) => theirs,
                        Read::Newer => return Err(Error::NewerFormat),
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

enum Read {
    Missing,
    Opened(StateFile),
    /// Written by a newer Dupin, in a format this build can't read.
    Newer,
    /// Well formed, but it fails to authenticate: damage, or another data key.
    Unopened,
    /// Not a state file of this format, or one that opened but couldn't be parsed.
    Damaged,
}

fn read(path: &Path, keys: &StateKeys) -> Result<Read, Error> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Read::Missing),
        Err(error) => return Err(error.into()),
    };
    let Some((&version, rest)) = bytes
        .strip_prefix(STATE_MAGIC.as_slice())
        .and_then(<[u8]>::split_first)
    else {
        return Ok(Read::Damaged);
    };
    if version > VERSION {
        return Ok(Read::Newer);
    }
    let sealed = (|| {
        if version != VERSION {
            return None;
        }
        let (tag, rest) = rest.split_at_checked(KEY_TAG_LEN)?;
        if KeySource::from_tag(tag.try_into().ok()?)? != KeySource::Device {
            return None;
        }
        let (nonce, ciphertext) = rest.split_at_checked(NONCE_LEN)?;
        Some((<[u8; NONCE_LEN]>::try_from(nonce).ok()?, ciphertext))
    })();
    let Some((nonce, ciphertext)) = sealed else {
        return Ok(Read::Damaged);
    };
    let Some(plain) = keys.open(&nonce, &keys.aad(), ciphertext.to_vec()) else {
        return Ok(Read::Unopened);
    };
    Ok(serde_json::from_slice::<StateFile>(&plain).map_or(Read::Damaged, Read::Opened))
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

    fn refuse() -> Result<bool, Error> {
        Ok(false)
    }

    #[test]
    fn hlc_carries_the_counter_into_milliseconds_and_never_goes_back() {
        let dir = tempfile::tempdir().unwrap();
        let device = Device::load(dir.path().join("storage.state"), keys(), None, refuse).unwrap();
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
        let device = Device::load(path.clone(), keys(), None, refuse).unwrap();
        let stamp = device.stamp(1_000_000).unwrap();
        drop(device);
        let device = Device::load(path, keys(), None, refuse).unwrap();
        assert_eq!(device.id(), stamp.device);
        assert!(device.stamp(1).unwrap() > stamp);
    }

    #[test]
    fn other_hardware_mints_a_new_id_and_remembers_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.state");
        let unrecorded = Device::load(path.clone(), keys(), None, refuse).unwrap().id();
        let here = Some(hardware_hash(b"machine-a"));
        let first = Device::load(path.clone(), keys(), here.clone(), refuse).unwrap();
        assert_eq!(
            first.id(),
            unrecorded,
            "recording the hardware the first time keeps the ID"
        );
        let stamp = first.stamp(500).unwrap();
        drop(first);
        assert_eq!(
            Device::load(path.clone(), keys(), here, refuse).unwrap().id(),
            unrecorded
        );

        let moved = Device::load(path.clone(), keys(), Some(hardware_hash(b"machine-b")), refuse).unwrap();
        assert_ne!(moved.id(), unrecorded);
        assert_eq!(moved.previous_id(), Some(unrecorded));
        assert!(moved.stamp(1).unwrap().hlc > stamp.hlc);
        assert_eq!(Device::load(path, keys(), None, refuse).unwrap().id(), moved.id());
    }

    #[test]
    fn a_damaged_state_is_replaced_and_unknown_fields_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.state");
        std::fs::write(&path, b"garbage").unwrap();
        let device = Device::load(path.clone(), keys(), None, refuse).unwrap();
        let id = device.id();
        drop(device);

        let Read::Opened(mut state) = read(&path, &keys()).unwrap() else {
            panic!("the state was rewritten");
        };
        state
            .newer
            .insert("from_the_future".into(), serde_json::json!(42));
        write(&path, &keys(), &state).unwrap();
        Device::load(path.clone(), keys(), None, refuse)
            .unwrap()
            .stamp(1)
            .unwrap();
        let Read::Opened(state) = read(&path, &keys()).unwrap() else {
            panic!("the state opens");
        };
        assert_eq!(state.device_id, id);
        assert_eq!(state.newer["from_the_future"], 42);
    }

    #[test]
    fn a_state_that_fails_to_open_is_replaced_only_when_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.state");
        let id = Device::load(path.clone(), keys(), None, refuse).unwrap().id();
        let bytes = std::fs::read(&path).unwrap();
        let other = || StateKeys::derive(&DataKey::from_slice(&[8; 32]).unwrap());

        assert!(matches!(
            Device::load(path.clone(), other(), None, refuse),
            Err(Error::WrongKey)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(Device::load(path.clone(), keys(), None, refuse).unwrap().id(), id);

        let replaced = Device::load(path.clone(), other(), None, || Ok(true)).unwrap();
        assert_ne!(replaced.id(), id);
    }

    #[test]
    fn a_state_in_a_newer_format_is_refused_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.state");
        Device::load(path.clone(), keys(), None, refuse).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[STATE_MAGIC.len()] = VERSION + 1;
        std::fs::write(&path, &bytes).unwrap();

        let replace = || -> Result<bool, Error> { panic!("a newer state is never judged") };
        assert!(matches!(
            Device::load(path.clone(), keys(), None, replace),
            Err(Error::NewerFormat)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
