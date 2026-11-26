use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dupin_storage::{
    Clock, DataKey, Error, Imported, Limits, MemoryKey, NewRecord, Storage, Store, VaultKey,
};
use serde_json::json;

const DRIFT: Duration = Duration::from_secs(300);
const START: u64 = 1_800_000_000_000;

#[derive(Clone)]
struct FakeClock(Arc<AtomicU64>);

impl FakeClock {
    fn new() -> Self {
        Self(Arc::new(AtomicU64::new(START)))
    }

    fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }

    fn set(&self, millis: u64) {
        self.0.store(millis, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

fn vault(epoch: u32) -> VaultKey {
    VaultKey::new([7; 16], epoch, DataKey::from_slice(&[42; 32]).unwrap()).unwrap()
}

struct Device {
    _dir: tempfile::TempDir,
    root: PathBuf,
    storage: Storage,
    clock: FakeClock,
}

impl Device {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("storage");
        let clock = FakeClock::new();
        let storage = Storage::new(&root, MemoryKey::random()).with_clock(clock.clone());
        Self {
            _dir: dir,
            root,
            storage,
            clock,
        }
    }

    fn open(&self) -> Store {
        self.storage
            .open_synced("notes", Limits::default(), &vault(1))
            .unwrap()
    }
}

fn text(value: &str) -> NewRecord {
    NewRecord {
        data: json!(value),
        blobs: Vec::new(),
    }
}

fn texts(store: &Store) -> Vec<String> {
    store
        .list(None, 200)
        .unwrap()
        .records
        .into_iter()
        .map(|record| record.data.as_str().unwrap().to_owned())
        .collect()
}

/// Copies every version `from` holds into `to`, as the sync engine would through a folder.
fn ship(from: &Store, to: &Store) -> Vec<Imported> {
    let mut outcomes = Vec::new();
    for version in from.versions() {
        let bytes = from.raw_record(&version.id).unwrap();
        outcomes.push(to.import_record(&version.id, &bytes, DRIFT).unwrap());
    }
    for blob in to.wanted_blobs() {
        if from.has_blob(&blob.handle) {
            to.import_blob(&blob.handle, &from.raw_blob(&blob.handle).unwrap())
                .unwrap();
        }
    }
    outcomes
}

#[test]
fn a_synced_namespace_is_sealed_under_the_vault_and_two_devices_agree() {
    let (a, b) = (Device::new(), Device::new());
    let (sa, sb) = (a.open(), b.open());
    assert_eq!(
        sa.namespace_id(),
        sb.namespace_id(),
        "namespace IDs come from the vault"
    );
    assert_eq!(sa.key_id("k"), sb.key_id("k"));
    sa.upsert("k", text("from a")).unwrap();
    assert_eq!(ship(&sa, &sb), [Imported::Applied]);
    assert_eq!(texts(&sb), ["from a"]);
    assert_eq!(
        ship(&sa, &sb),
        [Imported::Stale],
        "the same version again is stale"
    );

    b.clock.advance(Duration::from_secs(1));
    sb.upsert("k", text("from b")).unwrap();
    assert_eq!(ship(&sb, &sa), [Imported::Applied]);
    assert_eq!(texts(&sa), ["from b"]);
    let record = sa.get(&sa.key_id("k")).unwrap().unwrap();
    assert_eq!(record.device(), sb.device_id());
}

#[test]
fn the_newer_stamp_wins_whichever_order_versions_arrive_in() {
    let (a, b) = (Device::new(), Device::new());
    let (sa, sb) = (a.open(), b.open());
    sa.upsert("k", text("older")).unwrap();
    let older = sa.raw_record(&sa.key_id("k")).unwrap();
    a.clock.advance(Duration::from_secs(5));
    sa.upsert("k", text("newer")).unwrap();
    let newer = sa.raw_record(&sa.key_id("k")).unwrap();
    let id = sa.key_id("k");
    assert_eq!(sb.import_record(&id, &newer, DRIFT).unwrap(), Imported::Applied);
    assert_eq!(
        sb.import_record(&id, &older, DRIFT).unwrap(),
        Imported::Stale,
        "a replayed older version never wins"
    );
    assert_eq!(texts(&sb), ["newer"]);
}

#[test]
fn an_import_moves_the_clock_so_the_next_local_write_wins() {
    let (a, b) = (Device::new(), Device::new());
    let (sa, sb) = (a.open(), b.open());
    a.clock.advance(Duration::from_secs(60));
    sa.upsert("k", text("a, clock ahead")).unwrap();
    ship(&sa, &sb);
    sb.upsert("k", text("b, edited after")).unwrap();
    assert_eq!(ship(&sb, &sa), [Imported::Applied]);
    assert_eq!(texts(&sa), ["b, edited after"]);
}

#[test]
fn a_version_far_ahead_is_held_and_never_moves_the_clock() {
    let (a, b) = (Device::new(), Device::new());
    let (sa, sb) = (a.open(), b.open());
    a.clock.advance(Duration::from_secs(3600));
    sa.upsert("k", text("from the future")).unwrap();
    let before = b.storage.clock_hlc().unwrap();
    assert_eq!(ship(&sa, &sb), [Imported::Held]);
    assert_eq!(b.storage.clock_hlc().unwrap(), before);
    assert!(texts(&sb).is_empty());

    // A's clock is corrected: it resets and re-stamps, and the version then applies.
    a.clock.set(START);
    assert!(a.storage.reset_clock_if_ahead(DRIFT).unwrap());
    assert_eq!(sa.restamp_ahead(DRIFT).unwrap(), [sa.key_id("k")]);
    assert_eq!(ship(&sa, &sb), [Imported::Applied]);
    assert_eq!(texts(&sb), ["from the future"]);
}

#[test]
fn tombstones_delete_everywhere_and_beat_stale_copies() {
    let (a, b) = (Device::new(), Device::new());
    let (sa, sb) = (a.open(), b.open());
    let id = sa.upsert("k", text("doomed")).unwrap().id;
    let live = sa.raw_record(&id).unwrap();
    ship(&sa, &sb);
    a.clock.advance(Duration::from_secs(1));
    assert!(sa.delete(&id).unwrap());
    let versions = sa.versions();
    assert_eq!(versions.len(), 1);
    assert!(versions[0].deleted, "a synced namespace keeps its tombstone");
    assert_eq!(ship(&sa, &sb), [Imported::Applied]);
    assert!(texts(&sb).is_empty());
    assert_eq!(sb.import_record(&id, &live, DRIFT).unwrap(), Imported::Stale);
    assert!(texts(&sb).is_empty());

    let stamp = sb.versions()[0].stamp;
    assert!(sb.forget_version(&id, stamp).unwrap());
    assert!(sb.versions().is_empty());
}

#[test]
fn a_clear_marker_clears_older_versions_and_spares_later_ones() {
    let (a, b) = (Device::new(), Device::new());
    let (sa, sb) = (a.open(), b.open());
    sb.put(text("old on b")).unwrap();
    let old_on_a = sa.put(text("old on a")).unwrap();
    let old_bytes = sa.raw_record(&old_on_a.id).unwrap();
    a.clock.advance(Duration::from_secs(1));
    b.clock.advance(Duration::from_secs(1));
    sa.clear().unwrap();
    assert_eq!(sa.versions().len(), 1, "only the marker is left");
    b.clock.advance(Duration::from_secs(1));
    sb.put(text("new on b")).unwrap();
    ship(&sa, &sb);
    assert_eq!(texts(&sb), ["new on b"]);
    assert_eq!(
        sb.import_record(&old_on_a.id, &old_bytes, DRIFT).unwrap(),
        Imported::Stale,
        "a cleared version never comes back"
    );
    ship(&sb, &sa);
    assert_eq!(texts(&sa), ["new on b"]);
}

#[test]
fn a_record_waits_for_its_blob_and_a_forged_blob_is_refused() {
    let (a, b) = (Device::new(), Device::new());
    let (sa, sb) = (a.open(), b.open());
    let record = sa
        .put(NewRecord {
            data: json!("image"),
            blobs: vec![b"pixels".to_vec()],
        })
        .unwrap();
    let bytes = sa.raw_record(&record.id).unwrap();
    assert_eq!(
        sb.import_record(&record.id, &bytes, DRIFT).unwrap(),
        Imported::Pending
    );
    assert!(texts(&sb).is_empty(), "hidden until the blob arrives");
    let wanted = sb.wanted_blobs();
    assert_eq!(wanted.len(), 1);

    // Another blob of the same vault, renamed to the wanted handle, is refused.
    let other = sa
        .put(NewRecord {
            data: json!("other"),
            blobs: vec![b"other pixels".to_vec()],
        })
        .unwrap();
    let forged = sa.raw_blob(&other.blobs[0].handle).unwrap();
    assert!(matches!(
        sb.import_blob(&wanted[0].handle, &forged),
        Err(Error::Corrupt(_))
    ));
    drop(sb);
    let sb = b.open();
    assert_eq!(sb.wanted_blobs().len(), 1, "a pending record survives reopening");

    let genuine = sa.raw_blob(&wanted[0].handle).unwrap();
    assert!(sb.import_blob(&wanted[0].handle, &genuine).unwrap());
    assert_eq!(texts(&sb), ["image"]);
    assert_eq!(&**sb.read_blob(&wanted[0].handle).unwrap(), b"pixels");
}

#[test]
fn equal_blobs_seal_to_equal_bytes_on_every_device() {
    let (a, b) = (Device::new(), Device::new());
    let (sa, sb) = (a.open(), b.open());
    let blob = || NewRecord {
        data: json!("x"),
        blobs: vec![b"same image".to_vec()],
    };
    let ra = sa.put(blob()).unwrap();
    let rb = sb.put(blob()).unwrap();
    assert_eq!(ra.blobs[0].handle, rb.blobs[0].handle);
    assert_eq!(
        sa.raw_blob(&ra.blobs[0].handle).unwrap(),
        sb.raw_blob(&rb.blobs[0].handle).unwrap()
    );
}

#[test]
fn tampered_truncated_and_misplaced_files_are_refused() {
    let (a, b) = (Device::new(), Device::new());
    let (sa, sb) = (a.open(), b.open());
    let first = sa.upsert("first", text("one")).unwrap();
    let second = sa.upsert("second", text("two")).unwrap();
    let bytes = sa.raw_record(&first.id).unwrap();
    for at in [5, 30, 60, bytes.len() - 1] {
        let mut tampered = bytes.clone();
        tampered[at] ^= 1;
        assert!(
            sb.import_record(&first.id, &tampered, DRIFT).is_err(),
            "flipping byte {at}"
        );
    }
    for len in [0, 10, bytes.len() - 1] {
        assert!(sb.import_record(&first.id, &bytes[..len], DRIFT).is_err());
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(sb.import_record(&first.id, &longer, DRIFT).is_err());
    assert!(
        sb.import_record(&second.id, &bytes, DRIFT).is_err(),
        "a file moved to another record's name fails"
    );
    assert!(sb.import_record(&first.id, b"garbage", DRIFT).is_err());
    assert!(sb.versions().is_empty());

    // A namespace of another vault, or the device key, never opens here.
    let other_vault = VaultKey::new([8; 16], 1, DataKey::from_slice(&[1; 32]).unwrap()).unwrap();
    let c = Device::new();
    let sc = c
        .storage
        .open_synced("notes", Limits::default(), &other_vault)
        .unwrap();
    let foreign = sc.upsert("first", text("other vault")).unwrap();
    assert!(matches!(
        sb.import_record(&foreign.id, &sc.raw_record(&foreign.id).unwrap(), DRIFT),
        Err(Error::NewerFormat)
    ));
}

#[test]
fn turning_sync_on_and_off_reseals_without_losing_anything() {
    let device = Device::new();
    let local = device.storage.open("notes", Limits::default()).unwrap();
    local.upsert("kept", text("keyed")).unwrap();
    let image = local
        .put(NewRecord {
            data: json!("random id"),
            blobs: vec![b"blob".to_vec()],
        })
        .unwrap();
    drop(local);

    let synced = device.open();
    assert_eq!(texts(&synced), ["random id", "keyed"]);
    let keyed = synced.get(&synced.key_id("kept")).unwrap().unwrap();
    assert_eq!(keyed.data, json!("keyed"));
    let moved = synced.get(&image.id).unwrap().unwrap();
    assert_eq!(&**synced.read_blob(&moved.blobs[0].handle).unwrap(), b"blob");
    assert_eq!(synced.open_report().skipped_records, 0);
    for entry in std::fs::read_dir(device.root.join("notes/records")).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert_eq!(bytes[5], 1, "every record is tagged with the vault");
    }
    synced.upsert("kept", text("edited")).unwrap();
    assert_eq!(synced.usage().entries, 2, "the upsert finds the moved record");
    drop(synced);

    device.storage.hold_vault_key(vault(1));
    let local = device.storage.open("notes", Limits::default()).unwrap();
    assert_eq!(texts(&local), ["edited", "random id"]);
    assert_eq!(local.usage().entries, 2);
}

#[test]
fn without_the_vault_key_held_vault_files_are_skipped_not_deleted() {
    let device = Device::new();
    let synced = device.open();
    synced.upsert("k", text("v")).unwrap();
    drop(synced);
    let local = device.storage.open("notes", Limits::default()).unwrap();
    assert!(texts(&local).is_empty());
    assert_eq!(local.open_report().skipped_records, 1);
    drop(local);
    assert_eq!(texts(&device.open()), ["v"]);
}

#[test]
fn a_synced_item_limit_is_at_most_64_mib() {
    let device = Device::new();
    let limits = Limits {
        max_item_bytes: Limits::MAX_SYNC_ITEM_BYTES + 1,
        max_total_bytes: Limits::MAX_TOTAL_BYTES,
        ..Limits::default()
    };
    assert!(matches!(
        device.storage.open_synced("notes", limits, &vault(1)),
        Err(Error::InvalidLimits(_))
    ));
}

#[test]
fn an_imported_version_past_the_age_limit_is_never_shown() {
    let (a, b) = (Device::new(), Device::new());
    let sa = a.open();
    sa.put(text("old")).unwrap();
    b.clock.advance(Duration::from_secs(10 * 24 * 3600));
    let sb = b
        .storage
        .open_synced(
            "notes",
            Limits {
                max_age: Some(Duration::from_secs(24 * 3600)),
                ..Limits::default()
            },
            &vault(1),
        )
        .unwrap();
    assert_eq!(ship(&sa, &sb), [Imported::Expired]);
    assert!(sb.versions().is_empty());
}

#[test]
fn a_reminted_device_stamps_with_its_new_id() {
    let device = Device::new();
    let store = device.open();
    let old = store.device_id();
    let new = device.storage.remint_device_id().unwrap();
    assert_ne!(old, new);
    assert_eq!(device.storage.previous_device_id().unwrap(), Some(old));
    assert_eq!(store.put(text("x")).unwrap().device(), new);
}
