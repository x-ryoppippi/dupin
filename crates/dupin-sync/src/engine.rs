//! The sync engine: exports each synced namespace's versions into this device's directory in
//! the folder, imports every other device's, and keeps the folder tidy by the rules every
//! device follows, so no two devices ever write the same path except identical blobs.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dupin_storage::{
    BlobHandle, Clock, DeviceId, Error as StorageError, Imported, RecordId, Stamp, Storage, Store, VaultKey,
    Version,
};

use crate::descriptor::{self, Ack, Descriptor, MAX_DESCRIPTOR_BYTES};
use crate::error::Error;
use crate::folder::{BLOBS, DEVICES, Folder, NAMESPACES};
use crate::fsx::{Contents, Dir, Kind, id_name};
use crate::vault::{HEADER_NAME, VaultHeader};

/// How far ahead of the wall clock a version may be stamped before it is held.
pub const MAX_DRIFT: Duration = Duration::from_secs(5 * 60);
pub const TOMBSTONE_MIN_AGE: Duration = Duration::from_secs(30 * 24 * 3600);
pub const INACTIVE_AFTER: Duration = Duration::from_secs(90 * 24 * 3600);
pub const BLOB_UNREFERENCED_FOR: Duration = Duration::from_secs(7 * 24 * 3600);
const DESCRIPTOR_INTERVAL: Duration = Duration::from_secs(60);
/// A record file holds a sealed header and a JSON body of at most the 64 MiB sync item limit.
const MAX_RECORD_FILE: u64 = 64 * 1024 * 1024 + 128 * 1024;
const MAX_BLOB_FILE: u64 = 64 * 1024 * 1024 + 1024;
const COUNTER_BITS: u32 = 16;

/// How a namespace's records combine across devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Records keyed by a logical key; the newest version across devices wins.
    Lww,
    /// Each device writes only its own record; the owner reads every device's and merges them.
    PerDevice,
}

/// The owner's merge for a `per_device` namespace, run on the device that folds a leaving
/// device's record into its own.
pub trait Fold: Send + Sync {
    fn fold(&self, store: &Store, own: DeviceId, leaving: DeviceId) -> Result<(), StorageError>;
}

pub struct Namespace<'a> {
    /// The plain namespace name, as the owner opened it.
    pub name: &'a str,
    pub store: &'a Store,
    pub mode: Mode,
    pub fold: Option<&'a dyn Fold>,
}

/// About this Mac, for the other Macs' Settings → Sync.
#[derive(Debug, Clone, Default)]
pub struct DeviceInfo {
    pub name: String,
    pub platform: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStatus {
    pub id: DeviceId,
    pub name: String,
    /// Wall-clock milliseconds of its last full import.
    pub seen_at: u64,
    pub inactive: bool,
    pub retired: bool,
}

/// What one run did and found.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub exported: usize,
    pub imported: usize,
    pub removed: usize,
    /// Devices whose versions are held because their clock is more than five minutes ahead.
    pub clock_ahead: BTreeSet<DeviceId>,
    /// Files in the folder that failed to open; they are retried on the next run.
    pub unreadable: Vec<String>,
    /// Files not downloaded yet.
    pub not_arrived: usize,
    /// Another vault in the folder has a lower ID; this Mac must join it before exporting.
    pub losing_vault: bool,
    /// A file in the folder is in a format this version can't read; nothing was exported.
    pub newer_format: bool,
    /// The vault key was rotated to an epoch this Mac doesn't hold; nothing was exported.
    pub newer_epoch: bool,
    /// This device took a new ID: it was cloned, or removed by another Mac.
    pub reminted: bool,
    /// Namespaces this device has finished leaving.
    pub left: Vec<String>,
    /// Leaving devices whose files can't be taken over until a file of theirs opens.
    pub blocked_adoption: BTreeSet<DeviceId>,
    pub devices: Vec<DeviceStatus>,
}

#[derive(Clone)]
struct Seen {
    len: u64,
    modified: SystemTime,
    version: Version,
}

/// One version file found in the folder.
#[derive(Clone)]
struct Found {
    dir: DeviceId,
    name: String,
    version: Version,
}

/// A device's state for the namespace being synced.
struct Scan {
    files: Vec<Found>,
    /// Device directories with a file that didn't open.
    damaged: BTreeSet<DeviceId>,
    /// Blob files in `blobs/`, by handle.
    blobs: HashSet<BlobHandle>,
    blob_junk: Vec<String>,
    own_conflict: bool,
}

pub struct Engine {
    folder: Folder,
    storage: Arc<Storage>,
    vault: VaultKey,
    header: VaultHeader,
    info: DeviceInfo,
    clock: Arc<dyn Clock>,
    own: Descriptor,
    own_loaded: bool,
    descriptor_written: Option<(u64, Descriptor)>,
    cache: HashMap<(String, DeviceId, String), Seen>,
    unreferenced_since: HashMap<(String, BlobHandle), u64>,
    dataless: Option<DownloadRequest>,
    staging_cleared: Option<DeviceId>,
}

/// Asks the platform to download a placeholder, given its path inside the folder.
type DownloadRequest = Box<dyn Fn(&str) + Send + Sync>;

fn millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn rec_name(id: &RecordId) -> String {
    format!("{id}.rec")
}

impl Engine {
    /// An engine for a vault this Mac has joined, whose header is `header`.
    pub fn new(
        folder: Folder,
        storage: Arc<Storage>,
        vault: VaultKey,
        header: VaultHeader,
        info: DeviceInfo,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, Error> {
        if !header.verify(&vault) {
            return Err(Error::BadHeader("the header is not this vault key's".into()));
        }
        storage.hold_vault_key(vault.clone());
        Ok(Self {
            folder,
            storage,
            vault,
            header,
            info,
            clock,
            own: Descriptor::default(),
            own_loaded: false,
            descriptor_written: None,
            cache: HashMap::new(),
            unreferenced_since: HashMap::new(),
            dataless: None,
            staging_cleared: None,
        })
    }

    /// Asks the platform to download a file that is only a placeholder, given its path inside
    /// the folder.
    pub fn on_dataless(&mut self, request: impl Fn(&str) + Send + Sync + 'static) {
        self.dataless = Some(Box::new(request));
    }

    pub fn folder_path(&self) -> &std::path::Path {
        self.folder.path()
    }

    /// How the vault key travels: iCloud Keychain, or only the passphrase.
    pub fn key_mode(&self) -> crate::vault::KeyMode {
        self.header.mode
    }

    #[cfg(test)]
    pub(crate) fn vault(&self) -> &VaultKey {
        &self.vault
    }

    /// Removes another Mac from sync, as Remove This Mac does: the others take over its files,
    /// then its directories leave the folder.
    pub fn retire(&mut self, device: DeviceId) -> Result<(), Error> {
        let hlc = self.storage.clock_hlc()?;
        self.own.retired.insert(device, hlc);
        Ok(())
    }

    fn now(&self) -> u64 {
        millis(self.clock.now())
    }

    fn me(&self) -> Result<DeviceId, Error> {
        Ok(self.storage.device_id()?)
    }

    /// One full pass: every namespace in `namespaces` is imported and exported, and every name
    /// in `leaving` is left.
    pub fn run(&mut self, namespaces: &[Namespace<'_>], leaving: &[&str]) -> Result<Report, Error> {
        let mut report = Report::default();
        if !self.check_vault(&mut report)? {
            return Ok(report);
        }
        let mut me = self.me()?;
        if self.staging_cleared != Some(me) {
            self.folder
                .root()
                .create_dir(crate::folder::STAGING)?
                .remove_tree(&me.to_string())?;
            self.staging_cleared = Some(me);
        }
        let devices_dir = self.folder.root().create_dir(DEVICES)?;
        let mut others = self.read_descriptors(&devices_dir, me, &mut report)?;

        let retired_by_others: Option<u64> = others
            .values()
            .filter_map(|descriptor| descriptor.retired.get(&me).copied())
            .max();
        if let Some(hlc) = retired_by_others {
            me = self.rejoin_as_new_device(me, hlc, namespaces)?;
            report.reminted = true;
            others.remove(&me);
        }

        if self.storage.reset_clock_if_ahead(MAX_DRIFT)? {
            for namespace in namespaces {
                namespace.store.restamp_ahead(MAX_DRIFT)?;
            }
        }

        let mut retired: BTreeMap<DeviceId, u64> = self.own.retired.clone();
        for descriptor in others.values() {
            for (device, hlc) in &descriptor.retired {
                retired.entry(*device).or_insert(*hlc);
            }
        }
        retired.remove(&me);
        self.own.retired = retired.clone();

        for namespace in namespaces {
            let nsid = namespace.store.namespace_id();
            self.own.namespaces.insert(nsid.clone());
            self.own.leaving.remove(&nsid);
            self.sync_namespace(namespace, &nsid, me, &others, &retired, &mut report)?;
            if report.reminted {
                break;
            }
        }
        if report.reminted {
            // The next run exports everything under the new ID.
            return Ok(report);
        }
        for name in leaving {
            self.leave(name, me, &others, &mut report)?;
        }
        self.collect_retired(me, &others, &retired, &devices_dir)?;

        self.own.format = descriptor::FORMAT;
        self.own.name = self.info.name.clone();
        self.own.platform = self.info.platform.clone();
        self.own.version = self.info.version.clone();
        self.own.epoch = self.vault.epoch();
        self.own.seen_at = self.storage.clock_hlc()?;
        self.write_descriptor(&devices_dir, me)?;

        let now = self.now();
        let mut devices: Vec<DeviceStatus> = others
            .iter()
            .map(|(id, descriptor)| {
                let seen = descriptor.seen_at >> COUNTER_BITS;
                DeviceStatus {
                    id: *id,
                    name: descriptor.name.clone(),
                    seen_at: seen,
                    inactive: now.saturating_sub(seen) > duration_ms(INACTIVE_AFTER),
                    retired: retired.contains_key(id),
                }
            })
            .collect();
        devices.sort_by_key(|device| device.id);
        report.devices = devices;
        Ok(report)
    }

    /// Settles competing vaults: the lowest vault ID wins. Returns whether this Mac's vault
    /// is the winner and may export.
    fn check_vault(&mut self, report: &mut Report) -> Result<bool, Error> {
        let headers = self.folder.headers()?;
        let parsed: Vec<(&String, &VaultHeader)> = headers
            .iter()
            .filter_map(|(name, header)| header.as_ref().ok().map(|header| (name, header)))
            .collect();
        if headers
            .iter()
            .any(|(_, header)| matches!(header, Err(Error::NewerFormat)))
        {
            report.newer_format = true;
            return Ok(false);
        }
        let winner = parsed.iter().map(|(_, header)| header.id).min();
        if winner.is_some_and(|winner| winner < self.vault.id()) {
            report.losing_vault = true;
            return Ok(false);
        }
        if parsed
            .iter()
            .any(|(_, header)| header.id == self.vault.id() && header.epoch > self.vault.epoch())
        {
            // Another Mac rotated the key; this one needs the new epoch before it writes.
            report.newer_epoch = true;
            return Ok(false);
        }
        let canonical_ours = parsed
            .iter()
            .any(|(name, header)| name.as_str() == HEADER_NAME && header.verify(&self.vault));
        let me = self.me()?;
        if !canonical_ours {
            let staging = self.folder.staging(me)?;
            self.folder
                .root()
                .write_atomic(HEADER_NAME, &self.header.encode(), &staging)?;
        }
        for (name, _) in &parsed {
            if name.as_str() != HEADER_NAME {
                self.folder.root().remove(name)?;
            }
        }
        Ok(true)
    }

    fn read_descriptors(
        &mut self,
        devices: &Dir,
        me: DeviceId,
        report: &mut Report,
    ) -> Result<BTreeMap<DeviceId, Descriptor>, Error> {
        let mut found = BTreeMap::new();
        for entry in devices.entries()? {
            let Some(id) = id_name::<16>(&entry.name, ".dev") else {
                continue;
            };
            if entry.kind != Kind::File {
                continue;
            }
            let device: DeviceId = dupin_storage_device(id);
            match devices.read(&entry.name, MAX_DESCRIPTOR_BYTES)? {
                Contents::Bytes(bytes) => match Descriptor::open(&bytes, &self.vault, device) {
                    Some(descriptor) if device == me => {
                        if !self.own_loaded {
                            self.own = descriptor;
                        }
                    }
                    Some(descriptor) => {
                        if descriptor.format > descriptor::FORMAT {
                            report.newer_format = true;
                        }
                        found.insert(device, descriptor);
                    }
                    None => report.unreadable.push(format!("{DEVICES}/{}", entry.name)),
                },
                Contents::Missing => {}
                Contents::Unavailable => {
                    report.not_arrived += 1;
                    self.request_download(&format!("{DEVICES}/{}", entry.name));
                }
            }
        }
        self.own_loaded = true;
        Ok(found)
    }

    fn request_download(&self, path: &str) {
        if let Some(request) = &self.dataless {
            request(path);
        }
    }

    fn write_descriptor(&mut self, devices: &Dir, me: DeviceId) -> Result<(), Error> {
        let now = self.now();
        // What other devices act on goes out at once; the clock and acknowledgements at most
        // once a minute.
        let due = match &self.descriptor_written {
            None => true,
            Some((at, written)) => {
                let routine = |descriptor: &Descriptor| Descriptor {
                    seen_at: 0,
                    acks: BTreeMap::new(),
                    ..descriptor.clone()
                };
                routine(written) != routine(&self.own)
                    || (*written != self.own && now.saturating_sub(*at) >= duration_ms(DESCRIPTOR_INTERVAL))
            }
        };
        if !due {
            return Ok(());
        }
        let staging = self.folder.staging(me)?;
        devices.write_atomic(&format!("{me}.dev"), &self.own.seal(&self.vault, me), &staging)?;
        self.descriptor_written = Some((now, self.own.clone()));
        Ok(())
    }

    /// Another Mac removed this one: it takes a new device ID and rejoins every namespace,
    /// keeping what the folder still holds and its own versions written since.
    fn rejoin_as_new_device(
        &mut self,
        old: DeviceId,
        retired_at: u64,
        namespaces: &[Namespace<'_>],
    ) -> Result<DeviceId, Error> {
        let new = self.storage.remint_device_id()?;
        for namespace in namespaces {
            self.storage.set_left_at(
                namespace.name,
                Some(Stamp {
                    hlc: retired_at,
                    device: old,
                }),
            )?;
        }
        self.own = Descriptor::default();
        self.descriptor_written = None;
        self.cache.retain(|(_, dir, _), _| *dir != old);
        Ok(new)
    }

    fn scan(
        &mut self,
        nsid: &str,
        ns_dir: &Dir,
        me: DeviceId,
        store: &Store,
        report: &mut Report,
    ) -> Result<Scan, Error> {
        let mut scan = Scan {
            files: Vec::new(),
            damaged: BTreeSet::new(),
            blobs: HashSet::new(),
            blob_junk: Vec::new(),
            own_conflict: false,
        };
        for entry in ns_dir.entries()? {
            if entry.kind != Kind::Dir {
                continue;
            }
            if entry.name == BLOBS {
                let Some(blobs) = ns_dir.open_dir(BLOBS)? else {
                    continue;
                };
                for blob in blobs.entries()? {
                    if blob.kind != Kind::File {
                        continue;
                    }
                    match id_name::<32>(&blob.name, ".blob") {
                        Some(handle) => {
                            scan.blobs.insert(handle_from(handle));
                        }
                        None if blob
                            .name
                            .get(..64)
                            .is_some_and(|prefix| id_name::<32>(prefix, "").is_some()) =>
                        {
                            scan.blob_junk.push(blob.name);
                        }
                        None => {}
                    }
                }
                continue;
            }
            let Some(device) = id_name::<16>(&entry.name, "").map(dupin_storage_device) else {
                continue;
            };
            let Some(dir) = ns_dir.open_dir(&entry.name)? else {
                continue;
            };
            for file in dir.entries()? {
                if file.kind != Kind::File {
                    continue;
                }
                let Some(id) = id_name::<16>(&file.name, ".rec").map(record_id) else {
                    if device == me && looks_like_a_copy(&file.name) {
                        scan.own_conflict = true;
                        let canonical = format!("{}.rec", file.name.get(..32).unwrap_or_default());
                        if dir.stat(&canonical)?.is_some() {
                            dir.remove(&file.name)?;
                        }
                    }
                    continue;
                };
                let key = (nsid.to_owned(), device, file.name.clone());
                if let Some(seen) = self.cache.get(&key)
                    && seen.len == file.len
                    && seen.modified == file.modified
                {
                    scan.files.push(Found {
                        dir: device,
                        name: file.name,
                        version: seen.version.clone(),
                    });
                    continue;
                }
                if file.dataless {
                    report.not_arrived += 1;
                    self.request_download(&format!("{NAMESPACES}/{nsid}/{device}/{}", file.name));
                    continue;
                }
                let bytes = match dir.read(&file.name, MAX_RECORD_FILE)? {
                    Contents::Bytes(bytes) => bytes,
                    Contents::Missing => continue,
                    Contents::Unavailable => {
                        report.not_arrived += 1;
                        continue;
                    }
                };
                match store.inspect(&id, &bytes) {
                    Ok(version) => {
                        self.cache.insert(
                            key,
                            Seen {
                                len: file.len,
                                modified: file.modified,
                                version: version.clone(),
                            },
                        );
                        scan.files.push(Found {
                            dir: device,
                            name: file.name,
                            version,
                        });
                    }
                    Err(StorageError::NewerFormat) => {
                        report.newer_format = true;
                        scan.damaged.insert(device);
                    }
                    Err(_) => {
                        report
                            .unreadable
                            .push(format!("{NAMESPACES}/{nsid}/{device}/{}", file.name));
                        scan.damaged.insert(device);
                    }
                }
            }
        }
        Ok(scan)
    }

    fn sync_namespace(
        &mut self,
        namespace: &Namespace<'_>,
        nsid: &str,
        me: DeviceId,
        others: &BTreeMap<DeviceId, Descriptor>,
        retired: &BTreeMap<DeviceId, u64>,
        report: &mut Report,
    ) -> Result<(), Error> {
        let store = namespace.store;
        let ns_root = self.folder.root().create_dir(NAMESPACES)?;
        let ns_dir = ns_root.create_dir(nsid)?;
        let own_dir = ns_dir.create_dir(&me.to_string())?;
        let blobs_dir = ns_dir.create_dir(BLOBS)?;
        let staging = self.folder.staging(me)?;
        let previous = self.storage.previous_device_id()?;

        let scan = self.scan(nsid, &ns_dir, me, store, report)?;
        let participants: BTreeSet<DeviceId> = others
            .iter()
            .filter(|(id, descriptor)| descriptor.takes_part(nsid) && !retired.contains_key(id))
            .map(|(id, _)| *id)
            .collect();
        let leavers: BTreeSet<DeviceId> = others
            .iter()
            .filter(|(_, descriptor)| descriptor.leaving.contains(nsid))
            .map(|(id, _)| *id)
            .chain(retired.keys().copied())
            .filter(|id| *id != me)
            .collect();
        let adopted_by_all = |device: &DeviceId, own: &Descriptor| {
            let mine = own.adopted.get(nsid).is_some_and(|set| set.contains(device));
            mine && participants.iter().all(|participant| {
                others[participant]
                    .adopted
                    .get(nsid)
                    .is_some_and(|set| set.contains(device))
            })
        };
        let ignored: BTreeSet<DeviceId> = leavers
            .iter()
            .filter(|device| adopted_by_all(device, &self.own))
            .copied()
            .collect();
        let files: Vec<&Found> = scan
            .files
            .iter()
            .filter(|found| !ignored.contains(&found.dir))
            .collect();

        // A version in this device's own directory that it neither wrote nor adopted, or a
        // sync tool's copy of one of its files, means another machine writes as this device.
        let local: HashMap<RecordId, Stamp> = store.versions().into_iter().map(|v| (v.id, v.stamp)).collect();
        let cloned = scan.own_conflict
            || files.iter().any(|found| {
                found.dir == me
                    && found.version.stamp.device == me
                    && local
                        .get(&found.version.id)
                        .is_some_and(|held| *held < found.version.stamp)
            });
        if cloned {
            let old = me;
            self.storage.remint_device_id()?;
            self.storage
                .set_left_at(namespace.name, Some(Stamp { hlc: 0, device: old }))?;
            self.own = Descriptor::default();
            self.descriptor_written = None;
            report.reminted = true;
            return Ok(());
        }

        // Import every other device's versions.
        let mut acks = BTreeSet::new();
        for found in &files {
            if found.dir == me {
                continue;
            }
            let version = &found.version;
            if version.deleted && !version.is_clear_marker() {
                acks.insert(Ack {
                    id: version.id,
                    stamp: version.stamp,
                });
            }
            if local.get(&version.id).is_some_and(|held| *held >= version.stamp) {
                continue;
            }
            let dir = ns_dir.open_dir(&found.dir.to_string())?;
            let Some(Contents::Bytes(bytes)) = dir
                .map(|dir| dir.read(&found.name, MAX_RECORD_FILE))
                .transpose()?
            else {
                continue;
            };
            match store.import_record(&version.id, &bytes, MAX_DRIFT) {
                Ok(Imported::Applied | Imported::Pending) => report.imported += 1,
                Ok(Imported::Held) => {
                    report.clock_ahead.insert(version.stamp.device);
                }
                Ok(Imported::Stale | Imported::Expired) => {}
                Err(_) => report
                    .unreadable
                    .push(format!("{NAMESPACES}/{nsid}/{}/{}", found.dir, found.name)),
            }
        }
        self.own.acks.insert(nsid.to_owned(), acks);
        for blob in store.wanted_blobs() {
            if !scan.blobs.contains(&blob.handle) {
                continue;
            }
            let name = format!("{}.blob", blob.handle);
            match blobs_dir.read(&name, MAX_BLOB_FILE)? {
                Contents::Bytes(bytes) => {
                    if store.import_blob(&blob.handle, &bytes).is_err() {
                        report
                            .unreadable
                            .push(format!("{NAMESPACES}/{nsid}/{BLOBS}/{name}"));
                    }
                }
                Contents::Unavailable => {
                    report.not_arrived += 1;
                    self.request_download(&format!("{NAMESPACES}/{nsid}/{BLOBS}/{name}"));
                }
                Contents::Missing => {}
            }
        }

        // The folder's newest version of each record, among directories still read.
        let mut newest_in_folder: HashMap<RecordId, Stamp> = HashMap::new();
        for found in &files {
            let entry = newest_in_folder
                .entry(found.version.id)
                .or_insert(found.version.stamp);
            *entry = (*entry).max(found.version.stamp);
        }

        // Rejoining after leaving: keep what the folder still holds and this device's own
        // versions written since it left; the rest was deleted while it was away.
        if let Some(left) = self.storage.left_at(namespace.name)? {
            for version in store.versions() {
                let held = newest_in_folder
                    .get(&version.id)
                    .is_some_and(|newest| *newest >= version.stamp);
                let own_since = (version.stamp.device == me || version.stamp.device == left.device)
                    && version.stamp.hlc > left.hlc;
                if !held && !own_since {
                    store.forget_version(&version.id, version.stamp)?;
                }
            }
            self.storage.set_left_at(namespace.name, None)?;
        }

        if report.newer_format {
            return Ok(());
        }

        // Export: this device's directory holds the winning version of every record it wrote
        // or carries for a device that left.
        let own_files: HashMap<RecordId, &Found> = files
            .iter()
            .filter(|found| found.dir == me)
            .map(|found| (found.version.id, *found))
            .collect();
        let local_versions = store.versions();
        let mut carried: HashSet<RecordId> = HashSet::new();
        for version in &local_versions {
            let ours = version.stamp.device == me;
            let adopted = own_files
                .get(&version.id)
                .is_some_and(|found| found.version.stamp == version.stamp);
            let orphaned_previous = previous == Some(version.stamp.device)
                && newest_in_folder
                    .get(&version.id)
                    .is_none_or(|newest| *newest < version.stamp);
            if !(ours || adopted || orphaned_previous) {
                continue;
            }
            carried.insert(version.id);
            if adopted {
                continue;
            }
            let bytes = store.raw_record(&version.id)?;
            if bytes.len() as u64 > MAX_RECORD_FILE {
                continue;
            }
            own_dir.write_atomic(&rec_name(&version.id), &bytes, &staging)?;
            report.exported += 1;
        }
        for found in own_files.values() {
            if !carried.contains(&found.version.id) {
                own_dir.remove(&found.name)?;
                report.removed += 1;
            }
        }
        let damaged_own: Vec<String> = report
            .unreadable
            .iter()
            .filter_map(|path| path.strip_prefix(&format!("{NAMESPACES}/{nsid}/{me}/")))
            .map(str::to_owned)
            .collect();
        for name in damaged_own {
            if id_name::<16>(&name, ".rec")
                .map(record_id)
                .is_none_or(|id| !carried.contains(&id))
            {
                own_dir.remove(&name)?;
            }
        }

        // Blobs: upload what this device's versions use, and any a live version needs that is
        // missing from the folder.
        let mut referenced: HashSet<BlobHandle> = HashSet::new();
        for version in &local_versions {
            for blob in &version.blobs {
                referenced.insert(blob.handle);
                if !scan.blobs.contains(&blob.handle) && store.has_blob(&blob.handle) {
                    let name = format!("{}.blob", blob.handle);
                    match blobs_dir.create_new(&name, &store.raw_blob(&blob.handle)?, &staging) {
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        other => other?,
                    }
                }
            }
        }
        for found in &files {
            referenced.extend(found.version.blobs.iter().map(|blob| blob.handle));
        }
        let now = self.now();
        for handle in &scan.blobs {
            let key = (nsid.to_owned(), *handle);
            if referenced.contains(handle) {
                self.unreferenced_since.remove(&key);
                continue;
            }
            let since = *self.unreferenced_since.entry(key.clone()).or_insert(now);
            if now.saturating_sub(since) >= duration_ms(BLOB_UNREFERENCED_FOR) {
                blobs_dir.remove(&format!("{handle}.blob"))?;
                self.unreferenced_since.remove(&key);
            }
        }
        for junk in &scan.blob_junk {
            if scan
                .blobs
                .iter()
                .any(|handle| junk.starts_with(&handle.to_string()))
            {
                blobs_dir.remove(junk)?;
            }
        }

        // Collect this device's tombstones that every other device has acknowledged, once no
        // directory holds an older version of their record.
        for found in own_files.values() {
            let version = &found.version;
            if !version.deleted || version.is_clear_marker() || !carried.contains(&version.id) {
                continue;
            }
            let old_enough =
                now.saturating_sub(version.stamp.hlc >> COUNTER_BITS) >= duration_ms(TOMBSTONE_MIN_AGE);
            let ack = Ack {
                id: version.id,
                stamp: version.stamp,
            };
            let acknowledged = participants.iter().all(|participant| {
                others[participant]
                    .acks
                    .get(nsid)
                    .is_some_and(|acks| acks.contains(&ack))
            });
            let no_older = files
                .iter()
                .filter(|other| other.version.id == version.id)
                .all(|other| other.version.stamp >= version.stamp);
            if old_enough && acknowledged && no_older {
                own_dir.remove(&found.name)?;
                store.forget_version(&version.id, version.stamp)?;
                report.removed += 1;
            }
        }

        // Take over the files of devices that are leaving or were removed.
        let remaining_lowest = participants.iter().copied().chain([me]).min();
        for leaver in &leavers {
            if self.own.adopted.get(nsid).is_some_and(|set| set.contains(leaver)) {
                continue;
            }
            if scan.damaged.contains(leaver) {
                report.blocked_adoption.insert(*leaver);
                continue;
            }
            let theirs: Vec<&Found> = scan.files.iter().filter(|found| found.dir == *leaver).collect();
            let current: HashMap<RecordId, Stamp> =
                store.versions().into_iter().map(|v| (v.id, v.stamp)).collect();
            let mut complete = true;
            match namespace.mode {
                Mode::PerDevice => {
                    if remaining_lowest == Some(me)
                        && let Some(fold) = namespace.fold
                    {
                        fold.fold(store, me, *leaver)?;
                    }
                }
                Mode::Lww => {
                    for found in theirs {
                        let version = &found.version;
                        match current.get(&version.id) {
                            Some(held) if *held == version.stamp => {}
                            Some(held) if *held > version.stamp => continue,
                            _ if version.is_clear_marker() => continue,
                            _ => {
                                complete = false;
                                continue;
                            }
                        }
                        let elsewhere = files.iter().any(|other| {
                            other.dir != *leaver
                                && !leavers.contains(&other.dir)
                                && other.version.id == version.id
                                && other.version.stamp >= version.stamp
                        });
                        if elsewhere {
                            continue;
                        }
                        let Some(dir) = ns_dir.open_dir(&leaver.to_string())? else {
                            complete = false;
                            continue;
                        };
                        match dir.read(&found.name, MAX_RECORD_FILE)? {
                            Contents::Bytes(bytes) => {
                                own_dir.write_atomic(&found.name, &bytes, &staging)?;
                                report.exported += 1;
                            }
                            _ => complete = false,
                        }
                    }
                }
            }
            if complete {
                self.own
                    .adopted
                    .entry(nsid.to_owned())
                    .or_default()
                    .insert(*leaver);
            } else {
                report.blocked_adoption.insert(*leaver);
            }
        }

        // A per-device namespace drops a departed device's record once its directory is gone
        // from what is read: the lowest remaining device has folded it into its own.
        if namespace.mode == Mode::PerDevice {
            for version in store.versions() {
                if ignored.contains(&version.stamp.device) {
                    store.forget_version(&version.id, version.stamp)?;
                }
            }
        }

        // A device that left deletes its own directory once everyone took its files over;
        // a retired device's goes when the lowest live device sees the same.
        for device in &ignored {
            let ours_to_delete = match retired.contains_key(device) {
                true => remaining_lowest == Some(me),
                false => false,
            };
            if ours_to_delete {
                ns_dir.remove_tree(&device.to_string())?;
            }
        }
        Ok(())
    }

    fn leave(
        &mut self,
        name: &str,
        me: DeviceId,
        others: &BTreeMap<DeviceId, Descriptor>,
        report: &mut Report,
    ) -> Result<(), Error> {
        let nsid = self.vault.namespace_id(name);
        let Some(ns_dir) = self
            .folder
            .root()
            .open_dir(NAMESPACES)?
            .and_then(|ns| ns.open_dir(&nsid).ok().flatten())
        else {
            self.own.namespaces.remove(&nsid);
            self.own.leaving.remove(&nsid);
            report.left.push(name.to_owned());
            return Ok(());
        };
        if ns_dir.open_dir(&me.to_string())?.is_none() {
            self.own.namespaces.remove(&nsid);
            self.own.leaving.remove(&nsid);
            report.left.push(name.to_owned());
            return Ok(());
        }
        if !self.own.leaving.contains(&nsid) {
            self.own.leaving.insert(nsid.clone());
            if self.storage.left_at(name)?.is_none() {
                let hlc = self.storage.clock_hlc()?;
                self.storage.set_left_at(name, Some(Stamp { hlc, device: me }))?;
            }
        }
        let remaining: Vec<&Descriptor> = others
            .iter()
            .filter(|(id, descriptor)| descriptor.takes_part(&nsid) && !self.own.retired.contains_key(id))
            .map(|(_, descriptor)| descriptor)
            .collect();
        // With no other Mac to take its files over, they stay in the folder for one that
        // joins later, and this Mac is done leaving.
        let taken_over = remaining
            .iter()
            .all(|descriptor| descriptor.adopted.get(&nsid).is_some_and(|set| set.contains(&me)));
        if taken_over {
            if !remaining.is_empty() {
                ns_dir.remove_tree(&me.to_string())?;
                report.removed += 1;
            }
            self.own.namespaces.remove(&nsid);
            self.own.leaving.remove(&nsid);
            self.own.acks.remove(&nsid);
            report.left.push(name.to_owned());
        }
        Ok(())
    }

    /// Deletes the descriptor of a retired device once none of its directories is left.
    fn collect_retired(
        &mut self,
        me: DeviceId,
        others: &BTreeMap<DeviceId, Descriptor>,
        retired: &BTreeMap<DeviceId, u64>,
        devices: &Dir,
    ) -> Result<(), Error> {
        let live_lowest = others
            .keys()
            .filter(|id| !retired.contains_key(id))
            .copied()
            .chain([me])
            .min();
        if live_lowest != Some(me) {
            return Ok(());
        }
        let Some(ns_root) = self.folder.root().open_dir(NAMESPACES)? else {
            return Ok(());
        };
        for device in retired.keys() {
            let mut left = false;
            for entry in ns_root.entries()? {
                if entry.kind == Kind::Dir
                    && let Some(ns) = ns_root.open_dir(&entry.name)?
                    && ns.open_dir(&device.to_string())?.is_some()
                {
                    left = true;
                }
            }
            if !left {
                devices.remove(&format!("{device}.dev"))?;
            }
        }
        Ok(())
    }
}

/// A sync tool's copy of a record file: `<id> 2.rec`, `<id> (… conflicted copy …).rec`,
/// `<id>.sync-conflict-….rec`.
fn looks_like_a_copy(name: &str) -> bool {
    name.len() > 36
        && name
            .get(..32)
            .is_some_and(|prefix| id_name::<16>(prefix, "").is_some())
        && name.ends_with(".rec")
}

fn dupin_storage_device(bytes: [u8; 16]) -> DeviceId {
    crate::fsx::hex(&bytes)
        .parse()
        .expect("32 hex digits are a device ID")
}

fn record_id(bytes: [u8; 16]) -> RecordId {
    crate::fsx::hex(&bytes)
        .parse()
        .expect("32 hex digits are a record ID")
}

fn handle_from(bytes: [u8; 32]) -> BlobHandle {
    crate::fsx::hex(&bytes)
        .parse()
        .expect("64 hex digits are a blob handle")
}
