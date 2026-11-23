use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto::{NONCE_LEN, NamespaceKeys, TAG_LEN, VERSION};
use crate::fs::{Staged, create_private_dir, remove_if_exists, sync_dir};
use crate::key::{DataKey, KeyProvider};
use crate::{Error, MAX_BLOBS_PER_RECORD, MAX_PAGE, hex, secret, unhex, validate_namespace};

const RECORD_MAGIC: &[u8; 4] = b"DUPR";
const BLOB_MAGIC: &[u8; 4] = b"DUPB";
const CHECK_MAGIC: &[u8; 4] = b"DUPK";
const PREFIX_LEN: usize = 4 + 1;
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

/// A record's name: when it was stored (Unix milliseconds) and a sequence number that keeps
/// records stored in the same millisecond in order. IDs sort oldest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecordId {
    millis: u64,
    seq: u32,
}

impl RecordId {
    fn created_at(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.millis)
    }

    fn file_name(&self) -> String {
        format!("{self}.rec")
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}{:08x}", self.millis, self.seq)
    }
}

impl FromStr for RecordId {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        let bytes = unhex::<12>(text).ok_or(Error::NotFound)?;
        let (millis, seq) = bytes.split_at(8);
        Ok(Self {
            millis: u64::from_be_bytes(millis.try_into().expect("8 bytes")),
            seq: u32::from_be_bytes(seq.try_into().expect("4 bytes")),
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
}

impl Record {
    pub fn created_at(&self) -> SystemTime {
        self.id.created_at()
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Page {
    /// Newest first.
    pub records: Vec<Record>,
    /// Pass as `before` to get the next page; `None` when this page reached the oldest.
    pub next: Option<RecordId>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    body_len: u64,
    blobs: Vec<BlobRef>,
}

#[derive(Debug)]
struct Meta {
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

/// The root of all namespaces, usually `<data dir>/storage`. Asks its key provider once.
pub struct Storage {
    dir: PathBuf,
    keys: Box<dyn KeyProvider>,
    key: Mutex<Option<DataKey>>,
    clock: Arc<dyn Clock>,
}

impl Storage {
    pub fn new(dir: impl Into<PathBuf>, keys: impl KeyProvider + 'static) -> Self {
        Self {
            dir: dir.into(),
            keys: Box::new(keys),
            key: Mutex::new(None),
            clock: Arc::new(SystemClock),
        }
    }

    pub fn with_clock(mut self, clock: impl Clock + 'static) -> Self {
        self.clock = Arc::new(clock);
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
        let keys = NamespaceKeys::derive(&self.data_key()?, namespace);
        let mut report = OpenReport {
            abandoned_writes: clear_dir(&paths.tmp)?,
            ..OpenReport::default()
        };

        let mut records = BTreeMap::new();
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
                Ok(meta) => {
                    records.insert(id, meta);
                }
                Err(Error::Corrupt(_)) => unreadable.push(path),
                Err(error) => return Err(error),
            }
        }

        verify_key(
            &keys,
            &dir.join(CHECK),
            records.is_empty() && !unreadable.is_empty(),
        )?;

        report.corrupt_records += unreadable.len();
        for path in unreadable {
            remove_if_exists(&path)?;
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
            let overhead = (PREFIX_LEN + NONCE_LEN + TAG_LEN) as u64;
            match (handle, entry.metadata()?.len().checked_sub(overhead)) {
                (Some(handle), Some(len)) => {
                    blobs.insert(handle, BlobEntry { len, refs: 0 });
                }
                _ => stray_blobs.push(path),
            }
        }

        let mut broken = Vec::new();
        for (id, meta) in &records {
            let intact = meta
                .blobs
                .iter()
                .all(|blob| blobs.get(&blob.handle).is_some_and(|entry| entry.len == blob.len));
            if intact {
                for blob in &meta.blobs {
                    blobs.get_mut(&blob.handle).expect("checked above").refs += 1;
                }
            } else {
                broken.push(*id);
            }
        }
        report.corrupt_records += broken.len();
        for id in broken {
            records.remove(&id);
            remove_if_exists(&paths.records.join(id.file_name()))?;
        }

        blobs.retain(|handle, entry| {
            if entry.refs == 0 {
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

        let total = records.values().map(|meta| meta.body_len).sum::<u64>()
            + blobs.values().map(|entry| entry.len).sum::<u64>();
        let last_id = records.keys().next_back().copied();
        let store = Store {
            namespace: namespace.to_owned(),
            paths,
            keys,
            clock: self.clock.clone(),
            state: Mutex::new(State {
                limits,
                records,
                blobs,
                total,
                last_id,
            }),
            report: Mutex::new(report),
            _lock: lock,
            #[cfg(test)]
            crash_before_rename: std::sync::atomic::AtomicBool::new(false),
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
/// When the check file is missing or damaged, the key passes if at least one record opens,
/// and the check file is written again.
fn verify_key(keys: &NamespaceKeys, path: &Path, nothing_opened: bool) -> Result<(), Error> {
    let aad = keys.aad("check", &[]);
    match fs::read(path) {
        Ok(bytes) => {
            let opened = parse_sealed(CHECK_MAGIC, &bytes)
                .and_then(|(nonce, ciphertext)| keys.open(&nonce, &aad, ciphertext.to_vec()));
            if opened.is_some_and(|plaintext| plaintext.as_slice() == CHECK_PLAINTEXT) {
                return Ok(());
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if nothing_opened {
        return Err(Error::WrongKey);
    }
    let (nonce, ciphertext) = keys.seal(&aad, CHECK_PLAINTEXT)?;
    let file = framed(CHECK_MAGIC, &[&nonce, &ciphertext]);
    let tmp = path
        .parent()
        .expect("the check file is in the namespace")
        .join(TMP);
    Staged::write(&tmp, &file)?.commit(path)?;
    Ok(())
}

fn framed(magic: &[u8; 4], parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(PREFIX_LEN + parts.iter().map(|part| part.len()).sum::<usize>());
    out.extend_from_slice(magic);
    out.push(VERSION);
    for part in parts {
        out.extend_from_slice(part);
    }
    out
}

/// The nonce and ciphertext of a single-part sealed file.
fn parse_sealed<'a>(magic: &[u8; 4], bytes: &'a [u8]) -> Option<([u8; NONCE_LEN], &'a [u8])> {
    let rest = bytes.strip_prefix(magic.as_slice())?.strip_prefix(&[VERSION])?;
    let (nonce, ciphertext) = rest.split_at_checked(NONCE_LEN)?;
    Some((nonce.try_into().ok()?, ciphertext))
}

/// A record file: magic, version, the sealed header's length, then the header's nonce and
/// ciphertext, then the body's. The header lists the body length and the blobs, so opening a
/// namespace reads only headers.
struct RecordFile;

impl RecordFile {
    fn header_aad(keys: &NamespaceKeys, id: &RecordId) -> Vec<u8> {
        keys.aad("record-header", &[id.to_string().as_bytes()])
    }

    /// The body is bound to its own header's nonce, so headers and bodies can't be swapped.
    fn body_aad(keys: &NamespaceKeys, id: &RecordId, header_nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
        keys.aad("record-body", &[id.to_string().as_bytes(), header_nonce])
    }

    fn encode(keys: &NamespaceKeys, id: &RecordId, header: &Header, body: &[u8]) -> Result<Vec<u8>, Error> {
        let header_plain = secret::to_json(header)?;
        let (header_nonce, header_ct) = keys.seal(&Self::header_aad(keys, id), &header_plain)?;
        let (body_nonce, body_ct) = keys.seal(&Self::body_aad(keys, id, &header_nonce), body)?;
        let header_len = u32::try_from(header_ct.len()).expect("headers are small");
        Ok(framed(
            RECORD_MAGIC,
            &[
                &header_len.to_le_bytes(),
                &header_nonce,
                &header_ct,
                &body_nonce,
                &body_ct,
            ],
        ))
    }
}

fn read_header(keys: &NamespaceKeys, path: &Path, id: &RecordId) -> Result<Meta, Error> {
    let corrupt = || Error::Corrupt(format!("record {id}"));
    let truncated = |error: io::Error| match error.kind() {
        io::ErrorKind::UnexpectedEof => corrupt(),
        _ => Error::Io(error),
    };
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    let mut prefix = [0u8; PREFIX_LEN + 4];
    file.read_exact(&mut prefix).map_err(truncated)?;
    if &prefix[..4] != RECORD_MAGIC || prefix[4] != VERSION {
        return Err(corrupt());
    }
    let header_len = u32::from_le_bytes(prefix[PREFIX_LEN..].try_into().expect("4 bytes")) as usize;
    if header_len > MAX_HEADER_LEN {
        return Err(corrupt());
    }
    let mut nonce = [0u8; NONCE_LEN];
    file.read_exact(&mut nonce).map_err(truncated)?;
    let mut ciphertext = vec![0u8; header_len];
    file.read_exact(&mut ciphertext).map_err(truncated)?;
    let plain = keys
        .open(&nonce, &RecordFile::header_aad(keys, id), ciphertext)
        .ok_or_else(corrupt)?;
    let header: Header = serde_json::from_slice(&plain).map_err(|_| corrupt())?;
    let expected = (PREFIX_LEN + 4 + NONCE_LEN + header_len + NONCE_LEN + TAG_LEN) as u64 + header.body_len;
    if file_len != expected || header.blobs.len() > MAX_BLOBS_PER_RECORD {
        return Err(corrupt());
    }
    Ok(Meta {
        body_len: header.body_len,
        blobs: header.blobs,
    })
}

struct State {
    limits: Limits,
    records: BTreeMap<RecordId, Meta>,
    blobs: HashMap<BlobHandle, BlobEntry>,
    total: u64,
    last_id: Option<RecordId>,
}

impl State {
    /// An unused ID stamped with `now`, even when the clock has gone back since the last
    /// record: an ID's time is the record's age, which retention relies on.
    fn next_id(&self, now: u64) -> RecordId {
        let mut millis = now;
        loop {
            let taken = self
                .records
                .range(
                    ..=RecordId {
                        millis,
                        seq: u32::MAX,
                    },
                )
                .next_back()
                .map(|(id, _)| *id)
                .into_iter()
                .chain(self.last_id)
                .filter(|id| id.millis == millis)
                .map(|id| id.seq)
                .max();
            match taken.map(|seq| seq.checked_add(1)) {
                None => return RecordId { millis, seq: 0 },
                Some(Some(seq)) => return RecordId { millis, seq },
                Some(None) => millis += 1,
            }
        }
    }
}

/// One open namespace. Safe to share between threads; calls are serialized and block on IO.
pub struct Store {
    namespace: String,
    paths: Paths,
    keys: NamespaceKeys,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    report: Mutex<OpenReport>,
    _lock: File,
    #[cfg(test)]
    crash_before_rename: std::sync::atomic::AtomicBool,
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

    /// Seals and stores a record and its blobs. Blobs already stored in this namespace are
    /// shared, not written again. The plaintext buffers are zeroized before this returns.
    pub fn put(&self, record: NewRecord) -> Result<Record, Error> {
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
                let aad = self.keys.aad("blob", &[handle.to_string().as_bytes()]);
                let (nonce, ciphertext) = self.keys.seal(&aad, blob)?;
                Staged::write(&self.paths.tmp, &framed(BLOB_MAGIC, &[&nonce, &ciphertext]))?
                    .commit(&self.paths.blobs.join(handle.file_name()))?;
                written.insert(handle, len);
            }
            refs.push(BlobRef { handle, len });
        }
        drop(blobs);

        let id = state.next_id(unix_millis(self.clock.now()));
        let header = Header {
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
        state.total += header.body_len;
        state.records.insert(
            id,
            Meta {
                body_len: header.body_len,
                blobs: refs.clone(),
            },
        );
        state.last_id = Some(id);
        let record = Record {
            id,
            data: serde_json::from_slice(&body)?,
            blobs: refs,
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

    /// Up to `limit` records (at most [`MAX_PAGE`]) older than `before`, newest first.
    /// Records that turn out to be corrupt are dropped and skipped.
    pub fn list(&self, before: Option<&RecordId>, limit: usize) -> Result<Page, Error> {
        let limit = limit.clamp(1, MAX_PAGE);
        let mut state = self.state();
        let mut page = Page::default();
        let mut cursor = before.copied();
        loop {
            let ids: Vec<RecordId> = match cursor {
                Some(before) => state.records.range(..before),
                None => state.records.range(..),
            }
            .rev()
            .take(limit - page.records.len())
            .map(|(id, _)| *id)
            .collect();
            let Some(&oldest) = ids.last() else {
                break;
            };
            for id in ids {
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
            Some(last) if state.records.range(..last.id).next().is_some() => Some(last.id),
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
        let expected = (PREFIX_LEN + NONCE_LEN + TAG_LEN) as u64 + len;
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
        let (nonce, ciphertext) = parse_sealed(BLOB_MAGIC, &bytes).ok_or_else(corrupt)?;
        let aad = self.keys.aad("blob", &[handle.to_string().as_bytes()]);
        self.keys
            .open(&nonce, &aad, ciphertext.to_vec())
            .ok_or_else(corrupt)
    }

    pub fn delete(&self, id: &RecordId) -> Result<bool, Error> {
        let mut state = self.state();
        if !state.records.contains_key(id) {
            return Ok(false);
        }
        self.remove(&mut state, id)?;
        Ok(true)
    }

    pub fn clear(&self) -> Result<(), Error> {
        let mut state = self.state();
        while let Some((&id, _)) = state.records.first_key_value() {
            self.remove(&mut state, &id)?;
        }
        Ok(())
    }

    /// Deletes records past the namespace's limits, oldest first; returns how many.
    pub fn sweep(&self) -> Result<usize, Error> {
        let mut state = self.state();
        self.enforce(&mut state)
    }

    fn enforce(&self, state: &mut State) -> Result<usize, Error> {
        let mut removed = 0;
        if let Some(max_age) = state.limits.max_age {
            let now = unix_millis(self.clock.now());
            let max_age = u64::try_from(max_age.as_millis()).unwrap_or(u64::MAX);
            let cutoff = now.saturating_sub(max_age);
            while let Some((&oldest, _)) = state.records.first_key_value() {
                if oldest.millis >= cutoff {
                    break;
                }
                self.remove(state, &oldest)?;
                removed += 1;
            }
        }
        while state.records.len() > state.limits.max_entries || state.total > state.limits.max_total_bytes {
            let Some((&oldest, _)) = state.records.first_key_value() else {
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
        let rest = bytes
            .strip_prefix(RECORD_MAGIC.as_slice())
            .and_then(|rest| rest.strip_prefix(&[VERSION]))
            .ok_or_else(corrupt)?;
        let (header_len, rest) = rest.split_at_checked(4).ok_or_else(corrupt)?;
        let header_len = u32::from_le_bytes(header_len.try_into().expect("4 bytes")) as usize;
        let (header_nonce, rest) = rest.split_at_checked(NONCE_LEN).ok_or_else(corrupt)?;
        let header_nonce: [u8; NONCE_LEN] = header_nonce.try_into().expect("nonce length");
        let (header_ct, rest) = rest.split_at_checked(header_len).ok_or_else(corrupt)?;
        let header_plain = self
            .keys
            .open(
                &header_nonce,
                &RecordFile::header_aad(&self.keys, id),
                header_ct.to_vec(),
            )
            .ok_or_else(corrupt)?;
        let header: Header = serde_json::from_slice(&header_plain).map_err(|_| corrupt())?;
        if header.body_len != meta.body_len || header.blobs != meta.blobs {
            return Err(corrupt());
        }
        let (body_nonce, body_ct) = rest.split_at_checked(NONCE_LEN).ok_or_else(corrupt)?;
        let body_nonce: [u8; NONCE_LEN] = body_nonce.try_into().expect("nonce length");
        let body = self
            .keys
            .open(
                &body_nonce,
                &RecordFile::body_aad(&self.keys, id, &header_nonce),
                body_ct.to_vec(),
            )
            .ok_or_else(corrupt)?;
        Ok(Record {
            id: *id,
            data: serde_json::from_slice(&body).map_err(|_| corrupt())?,
            blobs: header.blobs,
        })
    }

    /// Removes the record file first: a crash after that leaves only unreferenced blobs, which
    /// the next open deletes.
    fn remove(&self, state: &mut State, id: &RecordId) -> Result<(), Error> {
        let Some(meta) = state.records.remove(id) else {
            return Ok(());
        };
        state.total -= meta.body_len;
        remove_if_exists(&self.paths.records.join(id.file_name()))?;
        for blob in meta.blobs {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryKey;

    #[test]
    fn record_ids_sort_by_time_then_sequence_and_round_trip() {
        let a = RecordId { millis: 5, seq: 9 };
        let b = RecordId { millis: 6, seq: 0 };
        assert!(a < b);
        assert_eq!(a.to_string().parse::<RecordId>().unwrap(), a);
        assert_eq!(a.to_string().len(), 24);
        assert!("zz".parse::<RecordId>().is_err());
    }

    #[test]
    fn a_crash_between_writing_and_renaming_leaves_no_trace() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path(), MemoryKey::random());
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
        assert_eq!(fs::read_dir(dir.path().join("clipboard/tmp")).unwrap().count(), 1);
        assert_eq!(
            fs::read_dir(dir.path().join("clipboard/blobs")).unwrap().count(),
            2
        );
        drop(store);

        let store = storage.open("clipboard", Limits::default()).unwrap();
        let report = store.open_report();
        assert_eq!(report.abandoned_writes, 1);
        assert_eq!(report.orphan_blobs, 1);
        assert_eq!(report.corrupt_records, 0);
        let page = store.list(None, 10).unwrap();
        assert_eq!(page.records, vec![kept.clone()]);
        assert_eq!(fs::read_dir(dir.path().join("clipboard/tmp")).unwrap().count(), 0);
        assert_eq!(
            fs::read_dir(dir.path().join("clipboard/blobs")).unwrap().count(),
            1
        );
        assert_eq!(&**store.read_blob(&kept.blobs[0].handle).unwrap(), b"kept blob");
    }
}
