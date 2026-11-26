//! What the sync engine needs from a namespace opened with [`Storage::open_synced`]: its
//! versions and their sealed files to export as they are, and import of another device's
//! files, which wins only with a newer stamp.

use std::fs;
use std::time::Duration;

use super::{
    BlobHandle, BlobRef, Error, Meta, RecordFile, RecordId, Stamp, State, Store, header_consistent, hex,
    open_blob, open_record, unix_millis,
};
use crate::device::hlc_from_millis;
use crate::fs::Staged;

/// One version a synced namespace holds: live, waiting for a blob, a tombstone or the clear
/// marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub id: RecordId,
    pub stamp: Stamp,
    pub deleted: bool,
    /// Only on the clear marker: every version stamped before this is cleared.
    clear_before: Option<Stamp>,
    pub blobs: Vec<BlobRef>,
    /// Live, but a blob it uses has not arrived.
    pending: bool,
}

impl Version {
    fn new(id: RecordId, meta: &Meta, pending: bool) -> Self {
        Self {
            id,
            stamp: meta.stamp,
            deleted: meta.deleted,
            clear_before: meta.clear_before,
            blobs: meta.blobs.clone(),
            pending,
        }
    }

    pub fn is_clear_marker(&self) -> bool {
        self.id == RecordId::CLEAR_MARKER
    }
}

/// What importing another device's version did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Imported {
    /// It is now this device's version of the record.
    Applied,
    /// It won, but stays hidden until the blobs it uses arrive.
    Pending,
    /// This device holds the same version or a newer one, or a clear marker covers it.
    Stale,
    /// Its stamp is too far ahead of the wall clock; it is neither applied nor observed.
    Held,
    /// The namespace's limits would evict it at once, so it is never shown.
    Expired,
}

impl Store {
    /// The name a synced folder knows this namespace by: a keyed hash of its name, so the
    /// folder never shows which namespaces sync.
    pub fn namespace_id(&self) -> String {
        hex(self.keys.namespace_id())
    }

    /// Every version this namespace holds, the clear marker included.
    pub fn versions(&self) -> Vec<Version> {
        let state = self.state();
        let live = state
            .records
            .iter()
            .map(|(id, meta)| Version::new(*id, meta, false));
        let pending = state
            .pending
            .iter()
            .map(|(id, meta)| Version::new(*id, meta, true));
        let dead = state.dead.iter().map(|(id, meta)| Version::new(*id, meta, false));
        live.chain(pending).chain(dead).collect()
    }

    /// The sealed file of the version held for `id`, byte for byte.
    pub fn raw_record(&self, id: &RecordId) -> Result<Vec<u8>, Error> {
        let state = self.state();
        if state.current(id).is_none() {
            return Err(Error::NotFound);
        }
        Ok(fs::read(self.paths.records.join(id.file_name()))?)
    }

    /// The sealed file of a stored blob, byte for byte.
    pub fn raw_blob(&self, handle: &BlobHandle) -> Result<Vec<u8>, Error> {
        let state = self.state();
        if !state.blobs.contains_key(handle) {
            return Err(Error::NotFound);
        }
        Ok(fs::read(self.paths.blobs.join(handle.file_name()))?)
    }

    pub fn has_blob(&self, handle: &BlobHandle) -> bool {
        self.state().blobs.contains_key(handle)
    }

    /// Opens a version file another device wrote for `id`, in full, without storing it. A file
    /// sealed under another key or in a newer format is [`Error::NewerFormat`]; one that fails
    /// to open is [`Error::Corrupt`].
    pub fn inspect(&self, id: &RecordId, bytes: &[u8]) -> Result<Version, Error> {
        let opened = open_record(&self.keys, id, bytes)?;
        Ok(Version::new(*id, &Meta::from(&opened.header), false))
    }

    /// Blobs that pending versions use and this device does not hold.
    pub fn wanted_blobs(&self) -> Vec<BlobRef> {
        let state = self.state();
        let mut wanted: Vec<BlobRef> = state
            .pending
            .values()
            .flat_map(|meta| &meta.blobs)
            .filter(|blob| !state.blobs.contains_key(&blob.handle))
            .copied()
            .collect();
        wanted.sort_by_key(|blob| blob.handle);
        wanted.dedup();
        wanted
    }

    /// Stores another device's version of `id` when it is newer than the one held here. The
    /// file is checked in full and kept byte for byte. A version stamped more than
    /// `max_drift` ahead of the wall clock is held: neither stored nor allowed to move this
    /// device's clock.
    pub fn import_record(&self, id: &RecordId, bytes: &[u8], max_drift: Duration) -> Result<Imported, Error> {
        let opened = open_record(&self.keys, id, bytes)?;
        if !header_consistent(&opened.header, &opened.origin, id) {
            return Err(Error::Corrupt(format!("record {id}")));
        }
        let meta = Meta::from(&opened.header);
        drop(opened);
        let mut state = self.state();
        if !state.synced {
            return Err(Error::InvalidLimits("only a synced namespace imports versions"));
        }
        let now = unix_millis(self.clock.now());
        let horizon =
            hlc_from_millis(now.saturating_add(u64::try_from(max_drift.as_millis()).unwrap_or(u64::MAX)));
        if meta.stamp.hlc > horizon {
            return Ok(Imported::Held);
        }
        if state.current(id).is_some_and(|held| held.stamp >= meta.stamp) {
            return Ok(Imported::Stale);
        }
        let marker = *id == RecordId::CLEAR_MARKER;
        if !marker && state.clear_before().is_some_and(|before| meta.stamp < before) {
            return Ok(Imported::Stale);
        }
        if !meta.deleted && !marker && self.evicted_at_once(&state, id, &meta) {
            return Ok(Imported::Expired);
        }

        Staged::write(&self.paths.tmp, bytes)?.commit(&self.paths.records.join(id.file_name()))?;
        self.device.observe(meta.stamp.hlc)?;
        let outcome = if meta.deleted {
            self.drop_version(&mut state, id)?;
            let clear_before = meta.clear_before;
            state.dead.insert(*id, meta);
            if let Some(before) = clear_before {
                self.apply_clear(&mut state, before)?;
            }
            Imported::Applied
        } else {
            let placed = meta;
            for blob in &placed.blobs {
                if let Some(entry) = state.blobs.get_mut(&blob.handle) {
                    entry.refs += 1;
                }
            }
            self.drop_version(&mut state, id)?;
            let pending = state.missing_blobs(&placed);
            if pending {
                state.pending.insert(*id, placed);
                Imported::Pending
            } else {
                state.insert(*id, placed);
                Imported::Applied
            }
        };
        self.enforce(&mut state)?;
        Ok(outcome)
    }

    fn evicted_at_once(&self, state: &State, id: &RecordId, meta: &Meta) -> bool {
        if self
            .age_cutoff(state)
            .is_some_and(|cutoff| Self::age_start(state, meta) < cutoff)
        {
            return true;
        }
        let replaces = state.records.contains_key(id);
        state.records.len() >= state.limits.max_entries
            && !replaces
            && state
                .order
                .first()
                .is_some_and(|oldest| (meta.stamp, *id) < *oldest)
    }

    /// Stores a blob a pending version is waiting for, after checking that it opens and holds
    /// the content its handle names. Returns whether it was wanted and stored.
    pub fn import_blob(&self, handle: &BlobHandle, bytes: &[u8]) -> Result<bool, Error> {
        let mut state = self.state();
        let wanted = state
            .pending
            .values()
            .flat_map(|meta| &meta.blobs)
            .find(|blob| blob.handle == *handle)
            .map(|blob| blob.len);
        let Some(len) = wanted else {
            return Ok(false);
        };
        if state.blobs.contains_key(handle) {
            return Ok(false);
        }
        let plain = open_blob(&self.keys, handle, bytes)?;
        if plain.len() as u64 != len {
            return Err(Error::Corrupt(format!("blob {handle}")));
        }
        drop(plain);
        Staged::write(&self.paths.tmp, bytes)?.commit(&self.paths.blobs.join(handle.file_name()))?;
        state.blobs.insert(*handle, super::BlobEntry { len, refs: 0 });
        state.total += len;
        self.recount_pending_refs(&mut state, handle);
        self.promote_pending(&mut state);
        self.enforce(&mut state)?;
        Ok(true)
    }

    /// Drops the version held for `id` without a tombstone, if it is still the one stamped
    /// `stamp`: a collected tombstone, or a version deleted elsewhere while this device was
    /// away. Returns whether it did.
    pub fn forget_version(&self, id: &RecordId, stamp: Stamp) -> Result<bool, Error> {
        let mut state = self.state();
        if state.current(id).is_none_or(|meta| meta.stamp != stamp) {
            return Ok(false);
        }
        self.remove(&mut state, id)?;
        Ok(true)
    }

    /// Re-stamps this device's own versions stamped more than `max_drift` ahead of the wall
    /// clock, after the clock that stamped them was found wrong and corrected, so the other
    /// devices, which hold those versions back, can apply them. Returns their IDs.
    pub fn restamp_ahead(&self, max_drift: Duration) -> Result<Vec<RecordId>, Error> {
        let mut state = self.state();
        let now = unix_millis(self.clock.now());
        let horizon =
            hlc_from_millis(now.saturating_add(u64::try_from(max_drift.as_millis()).unwrap_or(u64::MAX)));
        let me = self.device.id();
        let ahead: Vec<RecordId> = state
            .records
            .iter()
            .chain(&state.pending)
            .chain(&state.dead)
            .filter(|(_, meta)| meta.stamp.device == me && meta.stamp.hlc > horizon)
            .map(|(id, _)| *id)
            .collect();
        for id in &ahead {
            let path = self.paths.records.join(id.file_name());
            let opened = open_record(&self.keys, id, &fs::read(&path)?)?;
            let mut header = opened.header;
            header.stamp = self.device.stamp(now)?;
            if header.clear_before.is_some() {
                header.clear_before = Some(header.stamp);
            }
            let file = RecordFile::encode(&self.keys, id, &header, &opened.body)?;
            Staged::write(&self.paths.tmp, &file)?.commit(&path)?;
            let meta = Meta::from(&header);
            if let Some(old) = state.forget(id) {
                state.insert(
                    *id,
                    Meta {
                        blobs: old.blobs,
                        ..meta
                    },
                );
            } else if let Some(old) = state.pending.get_mut(id) {
                old.stamp = meta.stamp;
            } else if let Some(old) = state.dead.get_mut(id) {
                *old = meta;
            }
        }
        Ok(ahead)
    }
}
