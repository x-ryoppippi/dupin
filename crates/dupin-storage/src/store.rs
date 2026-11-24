use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto::{ID_LEN, KEY_TAG_LEN, KeySource, NONCE_LEN, NamespaceKeys, StateKeys, TAG_LEN, VERSION};
use crate::device::{Device, DeviceId, Stamp, hardware_hash};
use crate::fs::{Staged, create_private_dir, remove_if_exists, sync_dir};
use crate::key::{DataKey, KeyProvider};
use crate::{Error, MAX_BLOBS_PER_RECORD, MAX_PAGE, hex, secret, unhex, validate_namespace};

const RECORD_MAGIC: &[u8; 4] = b"DUPR";
const BLOB_MAGIC: &[u8; 4] = b"DUPB";
const CHECK_MAGIC: &[u8; 4] = b"DUPK";
/// Magic, format version and key tag: what every sealed file starts with.
const PREFIX_LEN: usize = 4 + 1 + KEY_TAG_LEN;
/// The prefix, the origin device's ID and the sealed header's length.
const RECORD_PREFIX_LEN: usize = PREFIX_LEN + ID_LEN + 4;
const BLOB_OVERHEAD: u64 = (PREFIX_LEN + NONCE_LEN + TAG_LEN) as u64;
const MAX_HEADER_LEN: usize = 64 * 1024;
const CHECK_PLAINTEXT: &[u8] = b"dupin-storage key check";

const RECORDS: &str = "records";
const BLOBS: &str = "blobs";
const TMP: &str = "tmp";
const LOCK: &str = "lock";
const CHECK: &str = "key.check";

pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

fn unix_millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

/// What a namespace may keep. The sweeper deletes the oldest records first until all hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Records older than this are deleted; `None` keeps them until another limit applies.
    pub max_age: Option<Duration>,
    pub max_entries: usize,
    /// Record data plus each distinct blob, counted once however many records share it.
    pub max_total_bytes: u64,
    /// One record's data plus its blobs. A larger `put` is refused, never truncated.
    pub max_item_bytes: u64,
}

impl Limits {
    const MAX_ENTRIES: usize = 100_000;
    pub const MAX_ITEM_BYTES: u64 = 512 * 1024 * 1024;
    pub const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;

    fn validate(&self) -> Result<(), Error> {
        if !(1..=Self::MAX_ENTRIES).contains(&self.max_entries) {
            return Err(Error::InvalidLimits("max_entries must be between 1 and 100000"));
        }
        if !(1..=Self::MAX_TOTAL_BYTES).contains(&self.max_total_bytes) {
            return Err(Error::InvalidLimits(
                "max_total_bytes must be between 1 byte and 16 GiB",
            ));
        }
        if !(1..=Self::MAX_ITEM_BYTES).contains(&self.max_item_bytes) {
            return Err(Error::InvalidLimits(
                "max_item_bytes must be between 1 byte and 512 MiB",
            ));
        }
        if self.max_item_bytes > self.max_total_bytes {
            return Err(Error::InvalidLimits(
                "max_item_bytes can't exceed max_total_bytes",
            ));
        }
        if self.max_age == Some(Duration::ZERO) {
            return Err(Error::InvalidLimits("max_age must be longer than zero"));
        }
        Ok(())
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_age: None,
            max_entries: 1000,
            max_total_bytes: 256 * 1024 * 1024,
            max_item_bytes: 16 * 1024 * 1024,
        }
    }
}

/// A record's stable 128-bit name. It stays the same when the record is replaced: random for
/// [`Store::put`], derived from the owner's key for [`Store::upsert`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecordId([u8; ID_LEN]);

impl RecordId {
    /// Reserved for the marker [`Store::clear`] writes.
    const CLEAR_MARKER: RecordId = RecordId([0; ID_LEN]);

    fn random() -> Result<Self, Error> {
        loop {
            let mut bytes = [0u8; ID_LEN];
            getrandom::fill(&mut bytes).map_err(|error| Error::Io(io::Error::other(error)))?;
            if bytes != Self::CLEAR_MARKER.0 {
                return Ok(Self(bytes));
            }
        }
    }

    fn file_name(&self) -> String {
        format!("{self}.rec")
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}

impl FromStr for RecordId {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        unhex::<ID_LEN>(text).map(Self).ok_or(Error::NotFound)
    }
}

/// Where [`Store::list`] continues: the version stamp and ID of the last record a page
/// returned. It survives that record being replaced or deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cursor {
    stamp: Stamp,
    id: RecordId,
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}{}{}", self.stamp.hlc, self.stamp.device, self.id)
    }
}

impl FromStr for Cursor {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        let bytes = unhex::<{ 8 + 2 * ID_LEN }>(text).ok_or(Error::NotFound)?;
        let (hlc, rest) = bytes.split_at(8);
        let (device, id) = rest.split_at(ID_LEN);
        Ok(Self {
            stamp: Stamp {
                hlc: u64::from_be_bytes(hlc.try_into().expect("8 bytes")),
                device: DeviceId(device.try_into().expect("ID length")),
            },
            id: RecordId(id.try_into().expect("ID length")),
        })
    }
}

/// A stored blob's name within its namespace, safe to hand to the UI or a guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobHandle([u8; 32]);

impl BlobHandle {
    fn file_name(&self) -> String {
        format!("{self}.blob")
    }
}

impl fmt::Display for BlobHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}

impl FromStr for BlobHandle {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        unhex::<32>(text).map(Self).ok_or(Error::NotFound)
    }
}

impl Serialize for BlobHandle {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for BlobHandle {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <&str>::deserialize(deserializer)?;
        text.parse()
            .map_err(|_| serde::de::Error::custom("invalid blob handle"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRef {
    pub handle: BlobHandle,
    pub len: u64,
}

/// What to store: JSON data and the blobs it refers to, in order.
#[derive(Debug, Default)]
pub struct NewRecord {
    pub data: serde_json::Value,
    pub blobs: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub id: RecordId,
    pub data: serde_json::Value,
    /// In the order they were given to `put`.
    pub blobs: Vec<BlobRef>,
    stamp: Stamp,
    created_at: u64,
}

impl Record {
    /// When the record was first stored, from the wall clock. Replacing it keeps this time.
    pub fn created_at(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.created_at)
    }

    /// The device that wrote this version.
    pub fn device(&self) -> DeviceId {
        self.stamp.device
    }

    fn cursor(&self) -> Cursor {
        Cursor {
            stamp: self.stamp,
            id: self.id,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Page {
    /// Newest version first.
    pub records: Vec<Record>,
    /// Pass as `before` to get the next page; `None` when this page reached the oldest.
    pub next: Option<Cursor>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub entries: usize,
    pub bytes: u64,
}

/// What opening a namespace found and repaired.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OpenReport {
    /// Temporary files left by a write that never finished.
    abandoned_writes: usize,
    /// Records that failed authentication, couldn't be parsed or lost a blob.
    pub corrupt_records: usize,
    /// Blobs no record referred to.
    orphan_blobs: usize,
    /// Records the limits removed.
    pub expired: usize,
    /// Records sealed under a key this device doesn't hold, or in a newer format. They are
    /// left on disk untouched and not listed.
    pub skipped_records: usize,
}

/// A record version's sealed header. Unknown fields are ignored, so a later version can add
/// fields without older builds taking its files for damage.
#[derive(Debug, Serialize, Deserialize)]
struct Header {
    stamp: Stamp,
    /// Wall-clock milliseconds when the record was first stored; age limits use it.
    created_at: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    deleted: bool,
    /// Only on the clear marker: every version stamped before this is cleared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clear_before: Option<Stamp>,
    body_len: u64,
    blobs: Vec<BlobRef>,
}

#[derive(Debug)]
struct Meta {
    stamp: Stamp,
    created_at: u64,
    deleted: bool,
    clear_before: Option<Stamp>,
    body_len: u64,
    blobs: Vec<BlobRef>,
}

#[derive(Debug)]
struct BlobEntry {
    len: u64,
    refs: usize,
}

struct Paths {
    records: PathBuf,
    blobs: PathBuf,
    tmp: PathBuf,
}

impl Paths {
    fn new(dir: &Path) -> Self {
        Self {
            records: dir.join(RECORDS),
            blobs: dir.join(BLOBS),
            tmp: dir.join(TMP),
        }
    }
}

/// The root of all namespaces, usually `<data dir>/storage`. Asks its key provider once, and
/// keeps the device's state in `storage.state` beside the root.
pub struct Storage {
    dir: PathBuf,
    keys: Box<dyn KeyProvider>,
    key: Mutex<Option<DataKey>>,
    clock: Arc<dyn Clock>,
    hardware: Option<String>,
    device: Mutex<Option<Arc<Device>>>,
}

impl Storage {
    pub fn new(dir: impl Into<PathBuf>, keys: impl KeyProvider + 'static) -> Self {
        Self {
            dir: dir.into(),
            keys: Box::new(keys),
            key: Mutex::new(None),
            clock: Arc::new(SystemClock),
            hardware: None,
            device: Mutex::new(None),
        }
    }

    pub fn with_clock(mut self, clock: impl Clock + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    /// The machine's hardware identity (`IOPlatformUUID`, `/etc/machine-id`). When it differs
    /// from the one the device ID was minted on, the data directory has moved to another
    /// machine, which then gets a device ID of its own.
    pub fn with_hardware_id(mut self, identity: &[u8]) -> Self {
        self.hardware = Some(hardware_hash(identity));
        self
    }

    fn data_key(&self) -> Result<DataKey, Error> {
        let mut cached = self.key.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(key) = &*cached {
            return Ok(key.clone());
        }
        let key = self.keys.data_key()?;
        *cached = Some(key.clone());
        Ok(key)
    }

    fn state_path(&self) -> PathBuf {
        match self.dir.file_name() {
            Some(name) => {
                let mut name = name.to_owned();
                name.push(".state");
                self.dir.with_file_name(name)
            }
            // A root with no name of its own; `.state` can never be a namespace.
            None => self.dir.join(".state"),
        }
    }

    fn device(&self, data_key: &DataKey) -> Result<Arc<Device>, Error> {
        let mut device = self.device.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(device) = &*device {
            return Ok(device.clone());
        }
        let loaded = Arc::new(Device::load(
            self.state_path(),
            StateKeys::derive(data_key),
            self.hardware.clone(),
        )?);
        *device = Some(loaded.clone());
        Ok(loaded)
    }

    /// Opens a namespace, repairs what an interrupted write left behind and applies `limits`.
    /// Only one `Store` per namespace may be open at a time; share it with an `Arc`.
    pub fn open(&self, namespace: &str, limits: Limits) -> Result<Store, Error> {
        validate_namespace(namespace)?;
        limits.validate()?;
        let dir = self.dir.join(namespace);
        let paths = Paths::new(&dir);
        create_private_dir(&self.dir)?;
        for path in [&dir, &paths.records, &paths.blobs, &paths.tmp] {
            create_private_dir(path)?;
        }
        let lock = lock_namespace(&dir)?;
        let data_key = self.data_key()?;
        let keys = NamespaceKeys::derive(&data_key, KeySource::Device, namespace);
        let mut report = OpenReport {
            abandoned_writes: clear_dir(&paths.tmp)?,
            ..OpenReport::default()
        };

        let mut found = Vec::new();
        let mut unreadable = Vec::new();
        for entry in fs::read_dir(&paths.records)? {
            let path = entry?.path();
            let id = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".rec"))
                .and_then(|stem| stem.parse::<RecordId>().ok());
            let Some(id) = id else {
                unreadable.push(path);
                continue;
            };
            match read_header(&keys, &path, &id) {
                Ok(Some(meta)) => found.push((id, meta)),
                Ok(None) => report.skipped_records += 1,
                Err(Error::Corrupt(_)) => unreadable.push(path),
                Err(error) => return Err(error),
            }
        }

        verify_key(&keys, &dir.join(CHECK), !found.is_empty(), !unreadable.is_empty())?;
        let device = self.device(&data_key)?;
        if let Some(newest) = found.iter().map(|(_, meta)| meta.stamp.hlc).max() {
            device.observe(newest)?;
        }

        report.corrupt_records += unreadable.len();
        for path in unreadable {
            remove_if_exists(&path)?;
        }

        // A clear or delete that stopped part way finishes now, the cleared records before
        // the marker and tombstones that stand for them.
        let clear_before = found.iter().filter_map(|(_, meta)| meta.clear_before).max();
        let (gone, live): (Vec<_>, Vec<_>) = found
            .into_iter()
            .partition(|(_, meta)| meta.deleted || clear_before.is_some_and(|before| meta.stamp < before));
        let (markers, cleared): (Vec<_>, Vec<_>) = gone.into_iter().partition(|(_, meta)| meta.deleted);
        for (id, _) in cleared.iter().chain(&markers) {
            remove_if_exists(&paths.records.join(id.file_name()))?;
        }

        let mut blobs = HashMap::new();
        let mut stray_blobs = Vec::new();
        for entry in fs::read_dir(&paths.blobs)? {
            let entry = entry?;
            let path = entry.path();
            let handle = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".blob"))
                .and_then(|stem| stem.parse::<BlobHandle>().ok());
            let Some(handle) = handle else {
                stray_blobs.push(path);
                continue;
            };
            match read_prefix(BLOB_MAGIC, &path, keys.source())? {
                Prefix::Ours => match entry.metadata()?.len().checked_sub(BLOB_OVERHEAD) {
                    Some(len) => {
                        blobs.insert(handle, BlobEntry { len, refs: 0 });
                    }
                    None => stray_blobs.push(path),
                },
                Prefix::Skip => {}
                Prefix::Damaged => stray_blobs.push(path),
            }
        }

        let mut state = State {
            limits,
            records: HashMap::new(),
            order: BTreeSet::new(),
            blobs,
            total: 0,
        };
        for (id, meta) in live {
            let intact = meta.blobs.iter().all(|blob| {
                state
                    .blobs
                    .get(&blob.handle)
                    .is_some_and(|entry| entry.len == blob.len)
            });
            if !intact {
                report.corrupt_records += 1;
                remove_if_exists(&paths.records.join(id.file_name()))?;
                continue;
            }
            for blob in &meta.blobs {
                state.blobs.get_mut(&blob.handle).expect("checked above").refs += 1;
            }
            state.insert(id, meta);
        }

        // A skipped record's blobs can't be told apart from orphans, so none is removed.
        let keep_unreferenced = report.skipped_records > 0;
        state.blobs.retain(|handle, entry| {
            if entry.refs == 0 && !keep_unreferenced {
                stray_blobs.push(paths.blobs.join(handle.file_name()));
            }
            entry.refs > 0
        });
        report.orphan_blobs = stray_blobs.len();
        for path in stray_blobs {
            remove_if_exists(&path)?;
        }
        sync_dir(&paths.records)?;
        sync_dir(&paths.blobs)?;

        state.total += state.blobs.values().map(|entry| entry.len).sum::<u64>();
        let store = Store {
            namespace: namespace.to_owned(),
            paths,
            keys,
            device,
            clock: self.clock.clone(),
            state: Mutex::new(state),
            report: Mutex::new(report),
            _lock: lock,
            #[cfg(test)]
            crash_before_rename: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            crash_after_marking: std::sync::atomic::AtomicBool::new(false),
        };
        let expired = store.sweep()?;
        store
            .report
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .expired = expired;
        Ok(store)
    }

    /// Deletes a namespace that is not open, for example one sealed with a lost key.
    pub fn discard(&self, namespace: &str) -> Result<(), Error> {
        validate_namespace(namespace)?;
        let dir = self.dir.join(namespace);
        if !dir.exists() {
            return Ok(());
        }
        let _lock = lock_namespace(&dir)?;
        fs::remove_dir_all(&dir)?;
        sync_dir(&self.dir)?;
        Ok(())
    }
}

fn lock_namespace(dir: &Path) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(dir.join(LOCK))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(fs::TryLockError::WouldBlock) => Err(Error::Busy),
        Err(fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

fn clear_dir(dir: &Path) -> io::Result<usize> {
    let mut removed = 0;
    for entry in fs::read_dir(dir)? {
        remove_if_exists(&entry?.path())?;
        removed += 1;
    }
    Ok(removed)
}

/// Confirms the namespace was sealed with this key before anything is deleted as corrupt.
/// A damaged check file is rewritten only when a record proves the key; a missing one is also
/// written for a fresh namespace, where no record exists to contradict the key. A check
/// tagged with another key is never this key's to judge or replace.
fn verify_key(
    keys: &NamespaceKeys,
    path: &Path,
    any_opened: bool,
    any_unreadable: bool,
) -> Result<(), Error> {
    let aad = keys.aad("check", &[]);
    match fs::read(path) {
        Ok(bytes) => {
            let parsed = parse_sealed(CHECK_MAGIC, &bytes);
            if let Some((Some(source), _, _)) = parsed
                && source != keys.source()
            {
                return Err(Error::WrongKey);
            }
            let opened =
                parsed.and_then(|(_, nonce, ciphertext)| keys.open(&nonce, &aad, ciphertext.to_vec()));
            if opened.is_some_and(|plaintext| plaintext.as_slice() == CHECK_PLAINTEXT) {
                return Ok(());
            }
            if !any_opened {
                return Err(Error::WrongKey);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if !any_opened && any_unreadable {
                return Err(Error::WrongKey);
            }
        }
        Err(error) => return Err(error.into()),
    }
    let (nonce, ciphertext) = keys.seal(&aad, CHECK_PLAINTEXT)?;
    let file = framed(CHECK_MAGIC, keys.source(), &[&nonce, &ciphertext]);
    let tmp = path
        .parent()
        .expect("the check file is in the namespace")
        .join(TMP);
    Staged::write(&tmp, &file)?.commit(path)?;
    Ok(())
}

fn framed(magic: &[u8; 4], source: KeySource, parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(PREFIX_LEN + parts.iter().map(|part| part.len()).sum::<usize>());
    out.extend_from_slice(magic);
    out.push(VERSION);
    out.extend_from_slice(&source.tag());
    for part in parts {
        out.extend_from_slice(part);
    }
    out
}

/// The key tag (`None` when it names no key), nonce and ciphertext of a single-part sealed
/// file in this format.
fn parse_sealed<'a>(
    magic: &[u8; 4],
    bytes: &'a [u8],
) -> Option<(Option<KeySource>, [u8; NONCE_LEN], &'a [u8])> {
    let rest = bytes.strip_prefix(magic.as_slice())?.strip_prefix(&[VERSION])?;
    let (tag, rest) = rest.split_at_checked(KEY_TAG_LEN)?;
    let (nonce, ciphertext) = rest.split_at_checked(NONCE_LEN)?;
    Some((
        KeySource::from_tag(tag.try_into().ok()?),
        nonce.try_into().ok()?,
        ciphertext,
    ))
}

enum Prefix {
    /// This format, sealed under the namespace's key.
    Ours,
    /// A newer format, or sealed under another key: neither this build's to read nor damage.
    Skip,
    Damaged,
}

fn classify(magic: &[u8; 4], prefix: &[u8], source: KeySource) -> Prefix {
    let Some(rest) = prefix.strip_prefix(magic.as_slice()) else {
        return Prefix::Damaged;
    };
    match rest[0] {
        version if version > VERSION => return Prefix::Skip,
        VERSION => {}
        _ => return Prefix::Damaged,
    }
    let tag: &[u8; KEY_TAG_LEN] = rest[1..1 + KEY_TAG_LEN].try_into().expect("prefix length");
    match KeySource::from_tag(tag) {
        Some(tagged) if tagged == source => Prefix::Ours,
        Some(_) => Prefix::Skip,
        None => Prefix::Damaged,
    }
}

fn read_prefix(magic: &[u8; 4], path: &Path, source: KeySource) -> Result<Prefix, Error> {
    let mut prefix = [0u8; PREFIX_LEN];
    match File::open(path)?.read_exact(&mut prefix) {
        Ok(()) => Ok(classify(magic, &prefix, source)),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(Prefix::Damaged),
        Err(error) => Err(error.into()),
    }
}

/// A record file: the prefix with its key tag, the ID of the device that wrote the version,
/// the sealed header's length, then the header's nonce and ciphertext, then, unless the
/// version is a tombstone, the body's. The header holds the version stamp and lists the body
/// length and the blobs, so opening a namespace reads only headers. The associated data names
/// the version's identity (namespace, origin device, record ID), never its directory, so the
/// same bytes open wherever the file is copied under its own name.
struct RecordFile;

impl RecordFile {
    fn header_aad(keys: &NamespaceKeys, origin: &DeviceId, id: &RecordId) -> Vec<u8> {
        keys.aad("record-header", &[&origin.0, &id.0])
    }

    /// The body is bound to its own header's nonce, so headers and bodies can't be swapped.
    fn body_aad(
        keys: &NamespaceKeys,
        origin: &DeviceId,
        id: &RecordId,
        header_nonce: &[u8; NONCE_LEN],
    ) -> Vec<u8> {
        keys.aad("record-body", &[&origin.0, &id.0, header_nonce])
    }

    fn encode(keys: &NamespaceKeys, id: &RecordId, header: &Header, body: &[u8]) -> Result<Vec<u8>, Error> {
        let origin = header.stamp.device;
        let header_plain = secret::to_json(header)?;
        let (header_nonce, header_ct) = keys.seal(&Self::header_aad(keys, &origin, id), &header_plain)?;
        let header_len = u32::try_from(header_ct.len()).expect("headers are small");
        let body = match header.deleted {
            true => None,
            false => Some(keys.seal(&Self::body_aad(keys, &origin, id, &header_nonce), body)?),
        };
        let header_len = header_len.to_le_bytes();
        let mut parts: Vec<&[u8]> = vec![&origin.0, &header_len, &header_nonce, &header_ct];
        if let Some((body_nonce, body_ct)) = &body {
            parts.extend([body_nonce.as_slice(), body_ct]);
        }
        Ok(framed(RECORD_MAGIC, keys.source(), &parts))
    }
}

/// A record's header, or `None` for a file this build must leave alone.
fn read_header(keys: &NamespaceKeys, path: &Path, id: &RecordId) -> Result<Option<Meta>, Error> {
    let corrupt = || Error::Corrupt(format!("record {id}"));
    let truncated = |error: io::Error| match error.kind() {
        io::ErrorKind::UnexpectedEof => corrupt(),
        _ => Error::Io(error),
    };
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    let mut prefix = [0u8; RECORD_PREFIX_LEN];
    file.read_exact(&mut prefix).map_err(truncated)?;
    match classify(RECORD_MAGIC, &prefix, keys.source()) {
        Prefix::Ours => {}
        Prefix::Skip => return Ok(None),
        Prefix::Damaged => return Err(corrupt()),
    }
    let origin = DeviceId(
        prefix[PREFIX_LEN..PREFIX_LEN + ID_LEN]
            .try_into()
            .expect("ID length"),
    );
    let header_len = u32::from_le_bytes(prefix[PREFIX_LEN + ID_LEN..].try_into().expect("4 bytes")) as usize;
    if header_len > MAX_HEADER_LEN {
        return Err(corrupt());
    }
    let mut nonce = [0u8; NONCE_LEN];
    file.read_exact(&mut nonce).map_err(truncated)?;
    let mut ciphertext = vec![0u8; header_len];
    file.read_exact(&mut ciphertext).map_err(truncated)?;
    let plain = keys
        .open(&nonce, &RecordFile::header_aad(keys, &origin, id), ciphertext)
        .ok_or_else(corrupt)?;
    let header: Header = serde_json::from_slice(&plain).map_err(|_| corrupt())?;
    let body = if header.deleted {
        0
    } else {
        (NONCE_LEN + TAG_LEN) as u64 + header.body_len
    };
    let expected = (RECORD_PREFIX_LEN + NONCE_LEN + header_len) as u64 + body;
    let marker = *id == RecordId::CLEAR_MARKER;
    let consistent = header.stamp.device == origin
        && header.blobs.len() <= MAX_BLOBS_PER_RECORD
        && (!header.deleted || (header.body_len == 0 && header.blobs.is_empty()))
        && marker == header.clear_before.is_some()
        && (!marker || header.deleted);
    if file_len != expected || !consistent {
        return Err(corrupt());
    }
    Ok(Some(Meta {
        stamp: header.stamp,
        created_at: header.created_at,
        deleted: header.deleted,
        clear_before: header.clear_before,
        body_len: header.body_len,
        blobs: header.blobs,
    }))
}

struct State {
    limits: Limits,
    records: HashMap<RecordId, Meta>,
    /// Every record by `(stamp, id)`: the order `list` pages in and limits evict from.
    order: BTreeSet<(Stamp, RecordId)>,
    blobs: HashMap<BlobHandle, BlobEntry>,
    total: u64,
}

impl State {
    fn insert(&mut self, id: RecordId, meta: Meta) {
        self.total += meta.body_len;
        self.order.insert((meta.stamp, id));
        self.records.insert(id, meta);
    }

    fn forget(&mut self, id: &RecordId) -> Option<Meta> {
        let meta = self.records.remove(id)?;
        self.order.remove(&(meta.stamp, *id));
        self.total -= meta.body_len;
        Some(meta)
    }

    fn oldest(&self) -> Option<RecordId> {
        self.order.first().map(|(_, id)| *id)
    }
}

/// One open namespace. Safe to share between threads; calls are serialized and block on IO.
pub struct Store {
    namespace: String,
    paths: Paths,
    keys: NamespaceKeys,
    device: Arc<Device>,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    report: Mutex<OpenReport>,
    _lock: File,
    #[cfg(test)]
    crash_before_rename: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    crash_after_marking: std::sync::atomic::AtomicBool,
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl Store {
    pub fn open_report(&self) -> OpenReport {
        *self.report.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// The device this store stamps its writes with.
    pub fn device_id(&self) -> DeviceId {
        self.device.id()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// Applies new limits at once; returns how many records they removed.
    pub fn set_limits(&self, limits: Limits) -> Result<usize, Error> {
        limits.validate()?;
        let mut state = self.state();
        state.limits = limits;
        self.enforce(&mut state)
    }

    pub fn usage(&self) -> Usage {
        let state = self.state();
        Usage {
            entries: state.records.len(),
            bytes: state.total,
        }
    }

    /// The ID [`Store::upsert`] stores `key` under in this namespace.
    pub fn key_id(&self, key: &str) -> RecordId {
        RecordId(self.keys.key_id(key))
    }

    /// Seals and stores a new record under a random ID. Blobs already stored in this
    /// namespace are shared, not written again. The plaintext buffers are zeroized before
    /// this returns.
    pub fn put(&self, record: NewRecord) -> Result<Record, Error> {
        self.write(RecordId::random()?, record)
    }

    /// Stores `record` as the record for the owner's logical `key`, replacing the one stored
    /// before, which keeps its creation time. The key itself is never written.
    pub fn upsert(&self, key: &str, record: NewRecord) -> Result<Record, Error> {
        self.write(self.key_id(key), record)
    }

    fn write(&self, id: RecordId, record: NewRecord) -> Result<Record, Error> {
        let NewRecord { data, blobs } = record;
        let blobs: Vec<Zeroizing<Vec<u8>>> = blobs.into_iter().map(Zeroizing::new).collect();
        if blobs.len() > MAX_BLOBS_PER_RECORD {
            return Err(Error::TooManyBlobs {
                count: blobs.len(),
                limit: MAX_BLOBS_PER_RECORD,
            });
        }
        let body = secret::to_json(&data)?;
        drop(data);
        let mut state = self.state();
        let limit = state.limits.max_item_bytes;
        let bytes = body.len() as u64 + blobs.iter().map(|blob| blob.len() as u64).sum::<u64>();
        if bytes > limit {
            return Err(Error::TooLarge { bytes, limit });
        }

        let mut refs = Vec::with_capacity(blobs.len());
        let mut written = HashMap::new();
        for blob in &blobs {
            let handle = BlobHandle(self.keys.blob_handle(blob));
            let len = blob.len() as u64;
            if !state.blobs.contains_key(&handle) && !written.contains_key(&handle) {
                let aad = self.keys.aad("blob", &[&handle.0]);
                let (nonce, ciphertext) = self.keys.seal_blob(&handle.0, &aad, blob)?;
                Staged::write(
                    &self.paths.tmp,
                    &framed(BLOB_MAGIC, self.keys.source(), &[&nonce, &ciphertext]),
                )?
                .commit(&self.paths.blobs.join(handle.file_name()))?;
                written.insert(handle, len);
            }
            refs.push(BlobRef { handle, len });
        }
        drop(blobs);

        let now = unix_millis(self.clock.now());
        let stamp = self.device.stamp(now)?;
        let created_at = state.records.get(&id).map_or(now, |meta| meta.created_at);
        let header = Header {
            stamp,
            created_at,
            deleted: false,
            clear_before: None,
            body_len: body.len() as u64,
            blobs: refs.clone(),
        };
        let file = RecordFile::encode(&self.keys, &id, &header, &body)?;
        let staged = Staged::write(&self.paths.tmp, &file)?;
        #[cfg(test)]
        if self.crash_before_rename.load(std::sync::atomic::Ordering::SeqCst) {
            std::mem::forget(staged);
            return Err(Error::Io(io::Error::other("simulated crash before rename")));
        }
        staged.commit(&self.paths.records.join(id.file_name()))?;

        for (handle, len) in written {
            state.blobs.insert(handle, BlobEntry { len, refs: 0 });
            state.total += len;
        }
        for blob in &refs {
            state
                .blobs
                .get_mut(&blob.handle)
                .expect("written or already stored")
                .refs += 1;
        }
        if let Some(replaced) = state.forget(&id) {
            self.release(&mut state, replaced.blobs)?;
        }
        state.insert(
            id,
            Meta {
                stamp,
                created_at,
                deleted: false,
                clear_before: None,
                body_len: header.body_len,
                blobs: refs.clone(),
            },
        );
        let record = Record {
            id,
            data: serde_json::from_slice(&body)?,
            blobs: refs,
            stamp,
            created_at,
        };
        self.enforce(&mut state)?;
        Ok(record)
    }

    pub fn get(&self, id: &RecordId) -> Result<Option<Record>, Error> {
        let mut state = self.state();
        if !state.records.contains_key(id) {
            return Ok(None);
        }
        self.read_record(&mut state, id).map(Some)
    }

    /// Up to `limit` records (at most [`MAX_PAGE`]) older than `before`, newest version
    /// first. Records that turn out to be corrupt are dropped and skipped.
    pub fn list(&self, before: Option<&Cursor>, limit: usize) -> Result<Page, Error> {
        let limit = limit.clamp(1, MAX_PAGE);
        let mut state = self.state();
        let mut page = Page::default();
        let mut cursor = before.map(|cursor| (cursor.stamp, cursor.id));
        loop {
            let keys: Vec<(Stamp, RecordId)> = match cursor {
                Some(before) => state.order.range(..before),
                None => state.order.range(..),
            }
            .rev()
            .take(limit - page.records.len())
            .copied()
            .collect();
            let Some(&oldest) = keys.last() else {
                break;
            };
            for (_, id) in keys {
                match self.read_record(&mut state, &id) {
                    Ok(record) => page.records.push(record),
                    Err(Error::Corrupt(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            cursor = Some(oldest);
            if page.records.len() == limit {
                break;
            }
        }
        page.next = match page.records.last() {
            Some(last) if state.order.range(..(last.stamp, last.id)).next().is_some() => Some(last.cursor()),
            _ => None,
        };
        Ok(page)
    }

    /// A blob's contents, if a record in this namespace still refers to it. A corrupt blob is
    /// dropped with every record that refers to it, so the next `put` of its content stores
    /// it again.
    pub fn read_blob(&self, handle: &BlobHandle) -> Result<Zeroizing<Vec<u8>>, Error> {
        let mut state = self.state();
        let len = state.blobs.get(handle).ok_or(Error::NotFound)?.len;
        match self.decode_blob(handle, len) {
            Err(Error::Corrupt(what)) => {
                let dependents: Vec<RecordId> = state
                    .records
                    .iter()
                    .filter(|(_, meta)| meta.blobs.iter().any(|blob| blob.handle == *handle))
                    .map(|(id, _)| *id)
                    .collect();
                for id in dependents {
                    self.remove(&mut state, &id)?;
                }
                Err(Error::Corrupt(what))
            }
            other => other,
        }
    }

    fn decode_blob(&self, handle: &BlobHandle, len: u64) -> Result<Zeroizing<Vec<u8>>, Error> {
        let path = self.paths.blobs.join(handle.file_name());
        let expected = BLOB_OVERHEAD + len;
        let corrupt = || Error::Corrupt(format!("blob {handle}"));
        let mut file = match File::open(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(corrupt()),
            other => other?,
        };
        if file.metadata()?.len() != expected {
            return Err(corrupt());
        }
        let mut bytes = Vec::with_capacity(expected as usize);
        file.read_to_end(&mut bytes)?;
        let (source, nonce, ciphertext) = parse_sealed(BLOB_MAGIC, &bytes).ok_or_else(corrupt)?;
        if source != Some(self.keys.source()) {
            return Err(corrupt());
        }
        let aad = self.keys.aad("blob", &[&handle.0]);
        self.keys
            .open(&nonce, &aad, ciphertext.to_vec())
            .ok_or_else(corrupt)
    }

    /// Writes a tombstone in the record's place, then, as this namespace doesn't sync, drops
    /// it as soon as it is durable.
    pub fn delete(&self, id: &RecordId) -> Result<bool, Error> {
        let mut state = self.state();
        let Some(created_at) = state.records.get(id).map(|meta| meta.created_at) else {
            return Ok(false);
        };
        self.mark(id, created_at, false)?;
        #[cfg(test)]
        if self.crash_after_marking.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(true);
        }
        self.remove(&mut state, id)?;
        Ok(true)
    }

    /// Writes a clear marker that every version stored so far is older than, deletes those
    /// versions, then, as this namespace doesn't sync, drops the marker.
    pub fn clear(&self) -> Result<(), Error> {
        let mut state = self.state();
        let now = unix_millis(self.clock.now());
        let marker = RecordId::CLEAR_MARKER;
        let before = self.mark(&marker, now, true)?;
        #[cfg(test)]
        if self.crash_after_marking.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        while let Some(id) = state.oldest().filter(|id| state.records[id].stamp < before) {
            self.remove(&mut state, &id)?;
        }
        remove_if_exists(&self.paths.records.join(marker.file_name()))?;
        sync_dir(&self.paths.records)?;
        Ok(())
    }

    /// Durably writes a tombstone, or with `clear` the clear marker, at `id`; returns its
    /// stamp.
    fn mark(&self, id: &RecordId, created_at: u64, clear: bool) -> Result<Stamp, Error> {
        let stamp = self.device.stamp(unix_millis(self.clock.now()))?;
        let header = Header {
            stamp,
            created_at,
            deleted: true,
            clear_before: clear.then_some(stamp),
            body_len: 0,
            blobs: Vec::new(),
        };
        let file = RecordFile::encode(&self.keys, id, &header, &[])?;
        Staged::write(&self.paths.tmp, &file)?.commit(&self.paths.records.join(id.file_name()))?;
        Ok(stamp)
    }

    /// Deletes records past the namespace's limits, oldest first; returns how many.
    pub fn sweep(&self) -> Result<usize, Error> {
        let mut state = self.state();
        self.enforce(&mut state)
    }

    /// Retention deletes files outright and never writes tombstones.
    fn enforce(&self, state: &mut State) -> Result<usize, Error> {
        let mut removed = 0;
        if let Some(max_age) = state.limits.max_age {
            let now = unix_millis(self.clock.now());
            let max_age = u64::try_from(max_age.as_millis()).unwrap_or(u64::MAX);
            let cutoff = now.saturating_sub(max_age);
            let expired: Vec<RecordId> = state
                .records
                .iter()
                .filter(|(_, meta)| meta.created_at < cutoff)
                .map(|(id, _)| *id)
                .collect();
            for id in expired {
                self.remove(state, &id)?;
                removed += 1;
            }
        }
        while state.records.len() > state.limits.max_entries || state.total > state.limits.max_total_bytes {
            let Some(oldest) = state.oldest() else {
                break;
            };
            self.remove(state, &oldest)?;
            removed += 1;
        }
        Ok(removed)
    }

    fn read_record(&self, state: &mut State, id: &RecordId) -> Result<Record, Error> {
        match self.decode_record(state, id) {
            Err(Error::Corrupt(what)) => {
                self.remove(state, id)?;
                Err(Error::Corrupt(what))
            }
            other => other,
        }
    }

    fn decode_record(&self, state: &State, id: &RecordId) -> Result<Record, Error> {
        let meta = state.records.get(id).ok_or(Error::NotFound)?;
        let corrupt = || Error::Corrupt(format!("record {id}"));
        let bytes = match fs::read(self.paths.records.join(id.file_name())) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(corrupt()),
            other => other?,
        };
        let (prefix, rest) = bytes.split_at_checked(RECORD_PREFIX_LEN).ok_or_else(corrupt)?;
        if !matches!(classify(RECORD_MAGIC, prefix, self.keys.source()), Prefix::Ours) {
            return Err(corrupt());
        }
        let origin = DeviceId(
            prefix[PREFIX_LEN..PREFIX_LEN + ID_LEN]
                .try_into()
                .expect("ID length"),
        );
        let header_len =
            u32::from_le_bytes(prefix[PREFIX_LEN + ID_LEN..].try_into().expect("4 bytes")) as usize;
        let (header_nonce, rest) = rest.split_at_checked(NONCE_LEN).ok_or_else(corrupt)?;
        let header_nonce: [u8; NONCE_LEN] = header_nonce.try_into().expect("nonce length");
        let (header_ct, rest) = rest.split_at_checked(header_len).ok_or_else(corrupt)?;
        let header_plain = self
            .keys
            .open(
                &header_nonce,
                &RecordFile::header_aad(&self.keys, &origin, id),
                header_ct.to_vec(),
            )
            .ok_or_else(corrupt)?;
        let header: Header = serde_json::from_slice(&header_plain).map_err(|_| corrupt())?;
        if header.deleted
            || header.stamp != meta.stamp
            || header.body_len != meta.body_len
            || header.blobs != meta.blobs
        {
            return Err(corrupt());
        }
        let (body_nonce, body_ct) = rest.split_at_checked(NONCE_LEN).ok_or_else(corrupt)?;
        let body_nonce: [u8; NONCE_LEN] = body_nonce.try_into().expect("nonce length");
        let body = self
            .keys
            .open(
                &body_nonce,
                &RecordFile::body_aad(&self.keys, &origin, id, &header_nonce),
                body_ct.to_vec(),
            )
            .ok_or_else(corrupt)?;
        Ok(Record {
            id: *id,
            data: serde_json::from_slice(&body).map_err(|_| corrupt())?,
            blobs: header.blobs,
            stamp: header.stamp,
            created_at: header.created_at,
        })
    }

    /// Removes the record file first: a crash after that leaves only unreferenced blobs, which
    /// the next open deletes.
    fn remove(&self, state: &mut State, id: &RecordId) -> Result<(), Error> {
        let Some(meta) = state.forget(id) else {
            return Ok(());
        };
        remove_if_exists(&self.paths.records.join(id.file_name()))?;
        self.release(state, meta.blobs)
    }

    fn release(&self, state: &mut State, blobs: Vec<BlobRef>) -> Result<(), Error> {
        for blob in blobs {
            let Some(entry) = state.blobs.get_mut(&blob.handle) else {
                continue;
            };
            entry.refs -= 1;
            if entry.refs == 0 {
                state.total -= entry.len;
                state.blobs.remove(&blob.handle);
                remove_if_exists(&self.paths.blobs.join(blob.handle.file_name()))?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn simulate_crash_before_rename(&self) {
        self.crash_before_rename
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    fn simulate_crash_after_marking(&self) {
        self.crash_after_marking
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryKey;

    fn text(value: &str) -> NewRecord {
        NewRecord {
            data: serde_json::json!(value),
            blobs: Vec::new(),
        }
    }

    #[test]
    fn record_ids_and_cursors_round_trip() {
        let id = RecordId::random().unwrap();
        assert_eq!(id.to_string().parse::<RecordId>().unwrap(), id);
        assert_eq!(id.to_string().len(), 32);
        assert!("zz".parse::<RecordId>().is_err());
        let cursor = Cursor {
            stamp: Stamp {
                hlc: 0x0123_4567_89ab_cdef,
                device: DeviceId([5; ID_LEN]),
            },
            id,
        };
        assert_eq!(cursor.to_string().parse::<Cursor>().unwrap(), cursor);
    }

    #[test]
    fn a_crash_between_writing_and_renaming_leaves_no_trace() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path().join("storage"), MemoryKey::random());
        let store = storage.open("clipboard", Limits::default()).unwrap();
        let kept = store
            .put(NewRecord {
                data: serde_json::json!("kept"),
                blobs: vec![b"kept blob".to_vec()],
            })
            .unwrap();
        store.simulate_crash_before_rename();
        assert!(
            store
                .put(NewRecord {
                    data: serde_json::json!("lost"),
                    blobs: vec![b"lost blob".to_vec()],
                })
                .is_err()
        );
        let ns = dir.path().join("storage/clipboard");
        assert_eq!(fs::read_dir(ns.join("tmp")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(ns.join("blobs")).unwrap().count(), 2);
        drop(store);

        let store = storage.open("clipboard", Limits::default()).unwrap();
        let report = store.open_report();
        assert_eq!(report.abandoned_writes, 1);
        assert_eq!(report.orphan_blobs, 1);
        assert_eq!(report.corrupt_records, 0);
        let page = store.list(None, 10).unwrap();
        assert_eq!(page.records, vec![kept.clone()]);
        assert_eq!(fs::read_dir(ns.join("tmp")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(ns.join("blobs")).unwrap().count(), 1);
        assert_eq!(&**store.read_blob(&kept.blobs[0].handle).unwrap(), b"kept blob");
    }

    #[test]
    fn a_tombstone_left_by_a_crash_keeps_the_record_deleted_and_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path().join("storage"), MemoryKey::random());
        let store = storage.open("clipboard", Limits::default()).unwrap();
        let gone = store
            .put(NewRecord {
                data: serde_json::json!("gone"),
                blobs: vec![b"gone blob".to_vec()],
            })
            .unwrap();
        let kept = store.put(text("kept")).unwrap();
        store.simulate_crash_after_marking();
        assert!(store.delete(&gone.id).unwrap());
        let records = dir.path().join("storage/clipboard/records");
        assert_eq!(
            fs::read_dir(&records).unwrap().count(),
            2,
            "the tombstone is on disk"
        );
        drop(store);

        let store = storage.open("clipboard", Limits::default()).unwrap();
        assert_eq!(store.open_report().corrupt_records, 0);
        assert_eq!(store.open_report().orphan_blobs, 1);
        assert_eq!(store.list(None, 10).unwrap().records, vec![kept]);
        assert_eq!(fs::read_dir(&records).unwrap().count(), 1);
        assert_eq!(store.get(&gone.id).unwrap(), None);
    }

    #[test]
    fn a_clear_marker_left_by_a_crash_finishes_the_clear() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path().join("storage"), MemoryKey::random());
        let store = storage.open("clipboard", Limits::default()).unwrap();
        for value in ["one", "two"] {
            store.put(text(value)).unwrap();
        }
        store.simulate_crash_after_marking();
        store.clear().unwrap();
        let records = dir.path().join("storage/clipboard/records");
        assert!(records.join(RecordId::CLEAR_MARKER.file_name()).exists());
        drop(store);

        let store = storage.open("clipboard", Limits::default()).unwrap();
        assert_eq!(store.usage(), Usage::default());
        assert_eq!(fs::read_dir(&records).unwrap().count(), 0);
        let after = store.put(text("after")).unwrap();
        drop(store);
        let store = storage.open("clipboard", Limits::default()).unwrap();
        assert_eq!(store.list(None, 10).unwrap().records, vec![after]);
    }

    #[test]
    fn one_clock_orders_writes_across_namespaces_and_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let key = MemoryKey::random();
        let storage = Storage::new(dir.path().join("storage"), key.clone());
        let first = storage
            .open("a", Limits::default())
            .unwrap()
            .put(text("a"))
            .unwrap();
        drop(storage);

        let storage = Storage::new(dir.path().join("storage"), key);
        let store = storage.open("b", Limits::default()).unwrap();
        let second = store.put(text("b")).unwrap();
        assert!(second.stamp > first.stamp);
        assert_eq!(second.device(), first.device());
        assert_eq!(store.device_id(), first.device());
        assert!(dir.path().join("storage.state").exists());
        assert!(!dir.path().join("storage").join("storage.state").exists());
    }
}
