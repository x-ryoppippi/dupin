use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dupin_storage::{
    Clock, DataKey, Error, FileKey, KeyProvider, Limits, MemoryKey, NewRecord, Storage, Store, Sweeper,
};
use serde_json::json;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// Where a record file's sealed header starts: magic, format version, key tag, origin device
/// ID, header length and header nonce come first.
const HEADER_CIPHERTEXT: usize = 4 + 1 + 21 + 16 + 4 + 24;
const KEY_TAG: usize = 5;

#[derive(Clone, Default)]
struct FakeClock(Arc<AtomicU64>);

impl FakeClock {
    fn at(millis: u64) -> Self {
        Self(Arc::new(AtomicU64::new(millis)))
    }

    fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }

    fn set(&self, time: SystemTime) {
        let millis = time.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        self.0.store(millis, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    key: DataKey,
    clock: FakeClock,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            key: DataKey::generate().unwrap(),
            clock: FakeClock::at(1_700_000_000_000),
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("storage")
    }

    fn storage(&self) -> Storage {
        Storage::new(self.root(), MemoryKey::new(self.key.clone())).with_clock(self.clock.clone())
    }

    fn other_key(&self) -> Storage {
        Storage::new(self.root(), MemoryKey::random())
    }

    fn open(&self, limits: Limits) -> Store {
        self.storage().open("clipboard", limits).unwrap()
    }

    fn namespace(&self) -> PathBuf {
        self.root().join("clipboard")
    }

    fn state_path(&self) -> PathBuf {
        self.dir.path().join("storage.state")
    }
}

fn text(value: &str) -> NewRecord {
    NewRecord {
        data: json!({ "text": value }),
        blobs: Vec::new(),
    }
}

fn texts(store: &Store) -> Vec<String> {
    store
        .list(None, 200)
        .unwrap()
        .records
        .into_iter()
        .map(|record| record.data["text"].as_str().unwrap().to_owned())
        .collect()
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files
}

/// Gives a sealed file a vault key tag (kind 1, a vault ID and epoch 1); returns its bytes.
fn retag_as_vault(path: &Path) -> Vec<u8> {
    let mut bytes = fs::read(path).unwrap();
    bytes[KEY_TAG] = 1;
    bytes[KEY_TAG + 1..KEY_TAG + 17].fill(0x42);
    bytes[KEY_TAG + 17..KEY_TAG + 21].copy_from_slice(&1u32.to_le_bytes());
    fs::write(path, &bytes).unwrap();
    bytes
}

/// Marks a sealed file as written in the next format; returns its bytes.
fn bump_format(path: &Path) -> Vec<u8> {
    let mut bytes = fs::read(path).unwrap();
    bytes[4] += 1;
    fs::write(path, &bytes).unwrap();
    bytes
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn records_and_blobs_round_trip_across_reopening() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let image = vec![0xab; 10_000];
    let put = store
        .put(NewRecord {
            data: json!({ "kind": "image", "name": "screenshot.png" }),
            blobs: vec![image.clone(), b"thumbnail".to_vec()],
        })
        .unwrap();
    assert_eq!(put.blobs.len(), 2);
    assert_eq!(put.blobs[0].len, 10_000);
    drop(store);

    let store = fixture.open(Limits::default());
    assert_eq!(store.open_report(), Default::default());
    let got = store.get(&put.id).unwrap().unwrap();
    assert_eq!(got, put);
    assert_eq!(got.created_at(), fixture.clock.now());
    assert_eq!(*store.read_blob(&got.blobs[0].handle).unwrap(), image);
    assert_eq!(&**store.read_blob(&got.blobs[1].handle).unwrap(), b"thumbnail");
    assert_eq!(store.get(&"0".repeat(32).parse().unwrap()).unwrap(), None);
}

#[test]
fn nothing_is_stored_in_the_clear() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    store
        .put(NewRecord {
            data: json!({ "text": "correct horse battery staple" }),
            blobs: vec![b"PLAINTEXT-BLOB-CONTENT".to_vec()],
        })
        .unwrap();
    for dir in ["records", "blobs"] {
        for file in files_in(&fixture.namespace().join(dir)) {
            let bytes = fs::read(file).unwrap();
            assert!(!contains(&bytes, b"correct horse"));
            assert!(!contains(&bytes, b"PLAINTEXT-BLOB"));
        }
    }
}

#[cfg(unix)]
#[test]
fn files_and_directories_are_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    store
        .put(NewRecord {
            data: json!(1),
            blobs: vec![b"blob".to_vec()],
        })
        .unwrap();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&fixture.namespace()), 0o700);
    for dir in ["records", "blobs"] {
        for file in files_in(&fixture.namespace().join(dir)) {
            assert_eq!(mode(&file), 0o600, "{}", file.display());
        }
    }
    assert_eq!(mode(&fixture.dir.path().join("storage.state")), 0o600);
}

#[test]
fn equal_blobs_are_stored_once_and_kept_while_referenced() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let first = store
        .put(NewRecord {
            data: json!("a"),
            blobs: vec![b"shared".to_vec()],
        })
        .unwrap();
    let second = store
        .put(NewRecord {
            data: json!("b"),
            blobs: vec![b"shared".to_vec()],
        })
        .unwrap();
    assert_eq!(first.blobs[0].handle, second.blobs[0].handle);
    assert_eq!(files_in(&fixture.namespace().join("blobs")).len(), 1);
    assert_eq!(store.usage().bytes, 3 + 3 + 6);

    assert!(store.delete(&first.id).unwrap());
    assert!(!store.delete(&first.id).unwrap());
    assert_eq!(&**store.read_blob(&second.blobs[0].handle).unwrap(), b"shared");
    store.delete(&second.id).unwrap();
    assert!(matches!(
        store.read_blob(&second.blobs[0].handle),
        Err(Error::NotFound)
    ));
    assert!(files_in(&fixture.namespace().join("blobs")).is_empty());
    assert_eq!(store.usage().bytes, 0);
}

#[test]
fn lists_newest_first_in_pages() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    for index in 0..5 {
        store.put(text(&index.to_string())).unwrap();
    }
    let first = store.list(None, 2).unwrap();
    let names = |page: &dupin_storage::Page| {
        page.records
            .iter()
            .map(|record| record.data["text"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&first), ["4", "3"]);
    let second = store.list(first.next.as_ref(), 2).unwrap();
    assert_eq!(names(&second), ["2", "1"]);
    let third = store.list(second.next.as_ref(), 2).unwrap();
    assert_eq!(names(&third), ["0"]);
    assert_eq!(third.next, None);
    assert_eq!(store.list(None, 0).unwrap().records.len(), 1);
    assert_eq!(store.list(None, usize::MAX).unwrap().records.len(), 5);
}

#[test]
fn clear_forgets_everything_but_the_key_check() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    store
        .put(NewRecord {
            data: json!("x"),
            blobs: vec![b"y".to_vec()],
        })
        .unwrap();
    store.clear().unwrap();
    assert_eq!(store.usage(), Default::default());
    assert!(files_in(&fixture.namespace().join("records")).is_empty());
    assert!(files_in(&fixture.namespace().join("blobs")).is_empty());
    drop(store);
    assert!(
        fixture
            .open(Limits::default())
            .list(None, 10)
            .unwrap()
            .records
            .is_empty()
    );
}

#[test]
fn a_tampered_record_is_dropped_and_the_rest_survive() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let ids: Vec<_> = ["one", "two", "three"]
        .into_iter()
        .map(|value| store.put(text(value)).unwrap().id)
        .collect();
    let path = fixture
        .namespace()
        .join("records")
        .join(format!("{}.rec", ids[1]));
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&path, bytes).unwrap();

    assert!(matches!(store.get(&ids[1]), Err(Error::Corrupt(_))));
    assert_eq!(store.get(&ids[1]).unwrap(), None);
    assert_eq!(texts(&store), ["three", "one"]);
}

#[test]
fn opening_drops_records_with_tampered_headers_or_swapped_names() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let ids: Vec<_> = ["one", "two", "three", "four"]
        .into_iter()
        .map(|value| store.put(text(value)).unwrap().id)
        .collect();
    drop(store);
    let records = fixture.namespace().join("records");
    let file = |id: &dupin_storage::RecordId| records.join(format!("{id}.rec"));

    let mut bytes = fs::read(file(&ids[0])).unwrap();
    bytes[HEADER_CIPHERTEXT + 4] ^= 0x80;
    fs::write(file(&ids[0]), bytes).unwrap();
    // A sealed record moved under another record's name fails: the name is authenticated.
    fs::rename(file(&ids[1]), file(&ids[2])).unwrap();
    fs::write(records.join("not-a-record.txt"), b"junk").unwrap();

    let store = fixture.open(Limits::default());
    assert_eq!(store.open_report().corrupt_records, 3);
    assert_eq!(texts(&store), ["four"]);
    assert_eq!(files_in(&records).len(), 1);
}

#[test]
fn a_tampered_or_missing_blob_drops_only_its_record() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let damaged = store
        .put(NewRecord {
            data: json!({ "text": "damaged" }),
            blobs: vec![b"image bytes".to_vec()],
        })
        .unwrap();
    let missing = store
        .put(NewRecord {
            data: json!({ "text": "missing" }),
            blobs: vec![b"other bytes".to_vec()],
        })
        .unwrap();
    store.put(text("fine")).unwrap();
    let blobs = fixture.namespace().join("blobs");
    let damaged_path = blobs.join(format!("{}.blob", damaged.blobs[0].handle));
    let mut bytes = fs::read(&damaged_path).unwrap();
    bytes[40] ^= 1;
    fs::write(&damaged_path, bytes).unwrap();
    assert!(matches!(
        store.read_blob(&damaged.blobs[0].handle),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(store.get(&damaged.id).unwrap(), None);
    assert!(!damaged_path.exists());
    drop(store);

    fs::remove_file(blobs.join(format!("{}.blob", missing.blobs[0].handle))).unwrap();
    let store = fixture.open(Limits::default());
    assert_eq!(store.open_report().corrupt_records, 1);
    assert_eq!(texts(&store), ["fine"]);
}

#[test]
fn content_whose_blob_was_found_damaged_is_stored_afresh() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let image = b"image bytes".to_vec();
    let first = store
        .put(NewRecord {
            data: json!("first copy"),
            blobs: vec![image.clone()],
        })
        .unwrap();
    let path = fixture
        .namespace()
        .join("blobs")
        .join(format!("{}.blob", first.blobs[0].handle));
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&path, bytes).unwrap();
    assert!(matches!(
        store.read_blob(&first.blobs[0].handle),
        Err(Error::Corrupt(_))
    ));

    let again = store
        .put(NewRecord {
            data: json!("second copy"),
            blobs: vec![image.clone()],
        })
        .unwrap();
    assert_eq!(again.blobs[0].handle, first.blobs[0].handle);
    assert_eq!(*store.read_blob(&again.blobs[0].handle).unwrap(), image);
}

#[cfg(unix)]
#[test]
fn an_unreadable_record_fails_the_open_and_deletes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let put = store
        .put(NewRecord {
            data: json!("kept"),
            blobs: vec![b"blob".to_vec()],
        })
        .unwrap();
    drop(store);
    let path = fixture
        .namespace()
        .join("records")
        .join(format!("{}.rec", put.id));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&path).is_ok() {
        // Running as root, permissions can't make the file unreadable.
        return;
    }
    let records = files_in(&fixture.namespace().join("records"));
    let blobs = files_in(&fixture.namespace().join("blobs"));

    assert!(matches!(
        fixture.storage().open("clipboard", Limits::default()),
        Err(Error::Io(_))
    ));
    assert_eq!(files_in(&fixture.namespace().join("records")), records);
    assert_eq!(files_in(&fixture.namespace().join("blobs")), blobs);

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let store = fixture.open(Limits::default());
    assert_eq!(store.open_report(), Default::default());
    assert_eq!(store.get(&put.id).unwrap(), Some(put));
}

#[test]
fn a_damaged_key_check_is_rewritten_when_the_records_open() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    store.put(text("secret")).unwrap();
    drop(store);
    let check = fixture.namespace().join("key.check");
    let mut bytes = fs::read(&check).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&check, &bytes).unwrap();

    let other = fixture.other_key();
    assert!(matches!(
        other.open("clipboard", Limits::default()),
        Err(Error::WrongKey)
    ));
    assert_eq!(fs::read(&check).unwrap(), bytes);

    let store = fixture.open(Limits::default());
    assert_eq!(store.open_report(), Default::default());
    assert_eq!(texts(&store), ["secret"]);
    drop(store);
    assert_ne!(fs::read(&check).unwrap(), bytes);
    assert!(matches!(
        other.open("clipboard", Limits::default()),
        Err(Error::WrongKey)
    ));
}

#[test]
fn an_intact_key_check_refuses_another_key_even_with_no_records_left() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let record = store.put(text("secret")).unwrap();
    store.delete(&record.id).unwrap();
    drop(store);
    let check = fixture.namespace().join("key.check");
    let bytes = fs::read(&check).unwrap();

    let other = fixture.other_key();
    assert!(matches!(
        other.open("clipboard", Limits::default()),
        Err(Error::WrongKey)
    ));
    assert_eq!(fs::read(&check).unwrap(), bytes);
    assert!(texts(&fixture.open(Limits::default())).is_empty());
}

#[test]
fn records_keep_their_real_time_after_the_clock_ran_ahead() {
    let fixture = Fixture::new();
    let limits = Limits {
        max_age: Some(DAY),
        ..Limits::default()
    };
    let store = fixture.open(Limits::default());
    let now = fixture.clock.now();
    let before = store.put(text("before")).unwrap();
    fixture.clock.set(now + DAY * 3650);
    store.put(text("future")).unwrap();
    fixture.clock.set(now);
    let secret = store.put(text("secret")).unwrap();
    assert_eq!(before.created_at(), now);
    assert_eq!(secret.created_at(), now);
    assert_eq!(
        texts(&store),
        ["secret", "future", "before"],
        "the clock never goes back"
    );
    drop(store);

    let store = fixture.open(limits);
    let reopened = store.put(text("reopened")).unwrap();
    assert_eq!(reopened.created_at(), now);
    assert_eq!(texts(&store)[0], "reopened");

    fixture.clock.advance(DAY * 30);
    assert_eq!(store.sweep().unwrap(), 3);
    assert_eq!(texts(&store), ["future"]);
    assert_eq!(store.open_report().expired, 0);
}

#[test]
fn a_different_key_is_refused_without_touching_the_data() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    store.put(text("secret")).unwrap();
    drop(store);
    let before: Vec<_> = files_in(&fixture.namespace().join("records"));

    let other = fixture.other_key();
    assert!(matches!(
        other.open("clipboard", Limits::default()),
        Err(Error::WrongKey)
    ));
    assert_eq!(files_in(&fixture.namespace().join("records")), before);

    // Without the check file the key is judged by whether any record opens.
    fs::remove_file(fixture.namespace().join("key.check")).unwrap();
    assert!(matches!(
        other.open("clipboard", Limits::default()),
        Err(Error::WrongKey)
    ));
    assert_eq!(files_in(&fixture.namespace().join("records")), before);
    assert_eq!(texts(&fixture.open(Limits::default())), ["secret"]);
}

#[test]
fn namespaces_are_isolated() {
    let fixture = Fixture::new();
    let storage = fixture.storage();
    let a = storage.open("ext.a", Limits::default()).unwrap();
    let b = storage.open("ext.b", Limits::default()).unwrap();
    let put = a
        .put(NewRecord {
            data: json!("a's"),
            blobs: vec![b"a's blob".to_vec()],
        })
        .unwrap();
    assert_eq!(b.get(&put.id).unwrap(), None);
    assert!(matches!(b.read_blob(&put.blobs[0].handle), Err(Error::NotFound)));
    drop(b);

    // a's sealed files copied into b's directory don't open there.
    let from = fixture.root().join("ext.a");
    let to = fixture.root().join("ext.b");
    for dir in ["records", "blobs"] {
        for file in files_in(&from.join(dir)) {
            fs::copy(&file, to.join(dir).join(file.file_name().unwrap())).unwrap();
        }
    }
    let b = storage.open("ext.b", Limits::default()).unwrap();
    assert_eq!(b.open_report().corrupt_records, 1);
    assert_eq!(b.usage(), Default::default());
}

#[test]
fn one_store_per_namespace() {
    let fixture = Fixture::new();
    let storage = fixture.storage();
    let store = storage.open("clipboard", Limits::default()).unwrap();
    assert!(matches!(
        storage.open("clipboard", Limits::default()),
        Err(Error::Busy)
    ));
    assert!(matches!(storage.discard("clipboard"), Err(Error::Busy)));
    drop(store);
    storage.open("clipboard", Limits::default()).unwrap();
}

#[test]
fn discard_removes_a_namespace_sealed_with_a_lost_key() {
    let fixture = Fixture::new();
    fixture.open(Limits::default()).put(text("x")).unwrap();
    let other = fixture.other_key();
    assert!(matches!(
        other.open("clipboard", Limits::default()),
        Err(Error::WrongKey)
    ));
    other.discard("clipboard").unwrap();
    assert!(!fixture.namespace().exists());
    assert!(
        other
            .open("clipboard", Limits::default())
            .unwrap()
            .list(None, 1)
            .unwrap()
            .records
            .is_empty()
    );
}

#[test]
fn retention_by_age() {
    let fixture = Fixture::new();
    let limits = Limits {
        max_age: Some(DAY),
        ..Limits::default()
    };
    let store = fixture.open(limits);
    store.put(text("old")).unwrap();
    fixture.clock.advance(DAY / 2);
    store.put(text("new")).unwrap();
    fixture.clock.advance(DAY / 2);
    assert_eq!(store.sweep().unwrap(), 0, "exactly a day old is kept");
    fixture.clock.advance(Duration::from_millis(1));
    assert_eq!(store.sweep().unwrap(), 1);
    assert_eq!(texts(&store), ["new"]);
    drop(store);

    fixture.clock.advance(DAY);
    let store = fixture.open(limits);
    assert_eq!(store.open_report().expired, 1);
    assert!(texts(&store).is_empty());
}

#[test]
fn forever_keeps_old_records() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    store.put(text("ancient")).unwrap();
    fixture.clock.advance(DAY * 3650);
    assert_eq!(store.sweep().unwrap(), 0);
    assert_eq!(texts(&store), ["ancient"]);
}

#[test]
fn retention_by_count() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits {
        max_entries: 3,
        ..Limits::default()
    });
    for index in 0..5 {
        store.put(text(&index.to_string())).unwrap();
    }
    assert_eq!(texts(&store), ["4", "3", "2"]);
    assert_eq!(
        store
            .set_limits(Limits {
                max_entries: 1,
                ..Limits::default()
            })
            .unwrap(),
        2
    );
    assert_eq!(texts(&store), ["4"]);
}

#[test]
fn retention_by_bytes_deletes_oldest_first() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits {
        max_total_bytes: 2_500,
        max_item_bytes: 1_100,
        ..Limits::default()
    });
    let mut ids = Vec::new();
    for index in 0..4u8 {
        ids.push(
            store
                .put(NewRecord {
                    data: json!(index),
                    blobs: vec![vec![index; 1_000]],
                })
                .unwrap()
                .id,
        );
    }
    assert_eq!(store.usage().entries, 2);
    assert!(store.usage().bytes <= 2_500);
    assert_eq!(store.get(&ids[0]).unwrap(), None);
    assert_eq!(store.get(&ids[1]).unwrap(), None);
    assert!(store.get(&ids[3]).unwrap().is_some());
    assert_eq!(files_in(&fixture.namespace().join("blobs")).len(), 2);
}

#[test]
fn oversized_items_are_refused_whole() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits {
        max_item_bytes: 4 * 1024 * 1024,
        max_total_bytes: 8 * 1024 * 1024,
        ..Limits::default()
    });
    let just_fits = vec![7u8; 4 * 1024 * 1024 - 4];
    let put = store
        .put(NewRecord {
            data: json!(null),
            blobs: vec![just_fits.clone()],
        })
        .unwrap();
    assert_eq!(*store.read_blob(&put.blobs[0].handle).unwrap(), just_fits);

    let too_big = store.put(NewRecord {
        data: json!(null),
        blobs: vec![vec![7u8; 4 * 1024 * 1024]],
    });
    assert!(matches!(too_big, Err(Error::TooLarge { .. })));
    let too_many = store.put(NewRecord {
        data: json!(null),
        blobs: vec![Vec::new(); dupin_storage::MAX_BLOBS_PER_RECORD + 1],
    });
    assert!(matches!(too_many, Err(Error::TooManyBlobs { .. })));
    assert_eq!(store.usage().entries, 1);
    assert!(files_in(&fixture.namespace().join("tmp")).is_empty());
}

#[test]
fn limits_are_bounded() {
    let fixture = Fixture::new();
    let storage = fixture.storage();
    for limits in [
        Limits {
            max_entries: 0,
            ..Limits::default()
        },
        Limits {
            max_item_bytes: Limits::MAX_ITEM_BYTES + 1,
            max_total_bytes: Limits::MAX_TOTAL_BYTES,
            ..Limits::default()
        },
        Limits {
            max_item_bytes: 10,
            max_total_bytes: 5,
            ..Limits::default()
        },
        Limits {
            max_age: Some(Duration::ZERO),
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            storage.open("clipboard", limits),
            Err(Error::InvalidLimits(_))
        ));
    }
    assert!(matches!(
        storage.open("../x", Limits::default()),
        Err(Error::InvalidNamespace(_))
    ));
}

#[test]
fn concurrent_writers_lose_nothing() {
    let fixture = Fixture::new();
    let store = Arc::new(fixture.open(Limits::default()));
    let threads = 8;
    let per_thread = 25;
    let barrier = Arc::new(Barrier::new(threads));
    let handles: Vec<_> = (0..threads)
        .map(|thread| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for index in 0..per_thread {
                    store
                        .put(NewRecord {
                            data: json!({ "thread": thread, "index": index }),
                            blobs: vec![format!("{thread}-{index}").into_bytes(), b"common".to_vec()],
                        })
                        .unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(store.usage().entries, threads * per_thread);
    let all = store.list(None, 200).unwrap();
    assert_eq!(all.records.len(), threads * per_thread);
    assert_eq!(all.next, None);
    for thread in 0..threads {
        let indices: Vec<_> = all
            .records
            .iter()
            .filter(|record| record.data["thread"] == thread)
            .map(|record| record.data["index"].as_u64().unwrap())
            .collect();
        assert_eq!(
            indices,
            (0..per_thread as u64).rev().collect::<Vec<_>>(),
            "newest first"
        );
    }
    let ids: std::collections::HashSet<_> = all.records.iter().map(|record| record.id).collect();
    assert_eq!(ids.len(), threads * per_thread);
    assert_eq!(
        files_in(&fixture.namespace().join("blobs")).len(),
        threads * per_thread + 1
    );
    drop(store);
    assert_eq!(fixture.open(Limits::default()).open_report(), Default::default());
}

#[test]
fn the_sweeper_expires_records_in_the_background() {
    let fixture = Fixture::new();
    let store = Arc::new(fixture.open(Limits {
        max_age: Some(Duration::from_secs(60)),
        ..Limits::default()
    }));
    store.put(text("soon gone")).unwrap();
    let sweeper = Sweeper::spawn(&store, Duration::from_millis(10), |error| panic!("{error}")).unwrap();
    fixture.clock.advance(Duration::from_secs(61));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while store.usage().entries > 0 {
        assert!(std::time::Instant::now() < deadline, "the sweeper never ran");
        std::thread::yield_now();
    }
    drop(sweeper);
}

#[test]
fn the_sweeper_stops_with_its_store() {
    let fixture = Fixture::new();
    let store = Arc::new(fixture.open(Limits::default()));
    let sweeper = Sweeper::spawn(&store, Duration::from_millis(1), |_| {}).unwrap();
    drop(store);
    std::thread::sleep(Duration::from_millis(20));
    drop(sweeper);
    fixture.open(Limits::default());
}

#[cfg(unix)]
#[test]
fn a_key_file_is_created_once_and_kept_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("storage.key");
    let provider = FileKey::new(&path);
    let first = provider.data_key().unwrap();
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(provider.data_key().unwrap().as_bytes(), first.as_bytes());

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(provider.data_key(), Err(Error::Key(_))));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, b"short").unwrap();
    assert!(matches!(provider.data_key(), Err(Error::Key(_))));
}

#[cfg(unix)]
#[test]
fn a_key_file_beside_the_storage_root_leaves_every_namespace_name_free() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(
        dir.path().join("storage"),
        FileKey::new(dir.path().join("storage.key")),
    );
    storage
        .open("key", Limits::default())
        .unwrap()
        .put(text("x"))
        .unwrap();
    assert_eq!(texts(&storage.open("key", Limits::default()).unwrap()), ["x"]);
}

#[test]
fn a_key_provider_failure_is_not_cached() {
    struct Flaky(std::sync::atomic::AtomicBool, DataKey);
    impl KeyProvider for Flaky {
        fn data_key(&self) -> Result<DataKey, Error> {
            if self.0.swap(true, Ordering::SeqCst) {
                Ok(self.1.clone())
            } else {
                Err(Error::Key("locked".into()))
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(
        dir.path(),
        Flaky(Default::default(), DataKey::generate().unwrap()),
    );
    assert!(matches!(
        storage.open("clipboard", Limits::default()),
        Err(Error::Key(_))
    ));
    storage.open("clipboard", Limits::default()).unwrap();
}

#[test]
fn upsert_replaces_the_record_for_a_key_and_keeps_its_creation_time() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let first = store
        .upsert(
            "theme",
            NewRecord {
                data: json!({ "text": "light" }),
                blobs: vec![b"old icon".to_vec()],
            },
        )
        .unwrap();
    assert_eq!(first.id, store.key_id("theme"));
    store.put(text("other")).unwrap();
    fixture.clock.advance(DAY);
    let second = store
        .upsert(
            "theme",
            NewRecord {
                data: json!({ "text": "dark" }),
                blobs: vec![b"new icon".to_vec()],
            },
        )
        .unwrap();
    assert_eq!(second.id, first.id);
    assert_eq!(second.created_at(), first.created_at());
    assert_eq!(
        texts(&store),
        ["dark", "other"],
        "a replaced record is the newest version"
    );
    assert_eq!(files_in(&fixture.namespace().join("records")).len(), 2);
    assert_eq!(files_in(&fixture.namespace().join("blobs")).len(), 1);
    assert_eq!(store.usage().entries, 2);
    drop(store);

    let store = fixture.open(Limits::default());
    assert_eq!(store.open_report(), Default::default());
    assert_eq!(store.get(&first.id).unwrap(), Some(second));
    let other = fixture.storage().open("settings", Limits::default()).unwrap();
    assert_ne!(other.key_id("theme"), store.key_id("theme"));
}

#[test]
fn logical_keys_never_reach_the_disk() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    store.upsert("PRIVATE-LOGICAL-KEY", text("x")).unwrap();
    for file in files_in(&fixture.namespace().join("records")) {
        assert!(!contains(&fs::read(&file).unwrap(), b"PRIVATE-LOGICAL-KEY"));
        assert!(!file.to_string_lossy().contains("PRIVATE"));
    }
}

#[test]
fn the_same_blob_is_sealed_to_the_same_bytes() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let blob_file = |record: &dupin_storage::Record| {
        fixture
            .namespace()
            .join("blobs")
            .join(format!("{}.blob", record.blobs[0].handle))
    };
    let image = || NewRecord {
        data: json!(null),
        blobs: vec![b"image bytes".to_vec()],
    };
    let first = store.put(image()).unwrap();
    let sealed = fs::read(blob_file(&first)).unwrap();
    store.delete(&first.id).unwrap();
    assert!(!blob_file(&first).exists());
    let again = store.put(image()).unwrap();
    assert_eq!(fs::read(blob_file(&again)).unwrap(), sealed);
}

#[test]
fn delete_and_clear_leave_no_tombstones_in_a_local_namespace() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let gone = store.put(text("gone")).unwrap();
    store.put(text("kept")).unwrap();
    assert!(store.delete(&gone.id).unwrap());
    assert_eq!(files_in(&fixture.namespace().join("records")).len(), 1);
    store.clear().unwrap();
    assert!(files_in(&fixture.namespace().join("records")).is_empty());
    let after = store.upsert("k", text("after")).unwrap();
    assert_eq!(texts(&store), ["after"]);
    drop(store);
    assert_eq!(
        fixture.open(Limits::default()).get(&after.id).unwrap(),
        Some(after)
    );
}

#[test]
fn a_page_cursor_survives_its_record_being_deleted() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    for index in 0..4 {
        store.put(text(&index.to_string())).unwrap();
    }
    let first = store.list(None, 2).unwrap();
    store.delete(&first.records[1].id).unwrap();
    let cursor: dupin_storage::Cursor = first.next.unwrap().to_string().parse().unwrap();
    let rest = store.list(Some(&cursor), 10).unwrap();
    let names: Vec<_> = rest
        .records
        .iter()
        .map(|record| record.data["text"].clone())
        .collect();
    assert_eq!(names, [json!("1"), json!("0")]);
}

#[test]
fn files_under_a_key_this_device_lacks_are_skipped_not_deleted() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let foreign = store
        .put(NewRecord {
            data: json!({ "text": "foreign" }),
            blobs: vec![b"foreign blob".to_vec()],
        })
        .unwrap();
    store.put(text("mine")).unwrap();
    drop(store);
    let record = fixture
        .namespace()
        .join("records")
        .join(format!("{}.rec", foreign.id));
    let bytes = retag_as_vault(&record);
    let blobs = files_in(&fixture.namespace().join("blobs"));

    let store = fixture.open(Limits::default());
    let report = store.open_report();
    assert_eq!(report.skipped_records, 1);
    assert_eq!(report.corrupt_records, 0);
    assert_eq!(texts(&store), ["mine"]);
    assert_eq!(fs::read(&record).unwrap(), bytes);
    assert_eq!(files_in(&fixture.namespace().join("blobs")), blobs);
    store.clear().unwrap();
    assert!(
        record.exists(),
        "clearing this key's records leaves another key's alone"
    );
}

#[test]
fn blobs_a_skipped_record_may_use_outlive_this_keys_records_and_count_against_quota() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let shared = b"shared blob".to_vec();
    let only = b"only the skipped record's".to_vec();
    let foreign = store
        .put(NewRecord {
            data: json!({ "text": "foreign" }),
            blobs: vec![shared.clone(), only.clone()],
        })
        .unwrap();
    let mine = store
        .put(NewRecord {
            data: json!({ "text": "mine" }),
            blobs: vec![shared.clone()],
        })
        .unwrap();
    drop(store);
    retag_as_vault(
        &fixture
            .namespace()
            .join("records")
            .join(format!("{}.rec", foreign.id)),
    );
    let blob_dir = fixture.namespace().join("blobs");
    let blobs = files_in(&blob_dir);
    assert_eq!(blobs.len(), 2);

    let store = fixture.open(Limits::default());
    assert_eq!(store.open_report().skipped_records, 1);
    let kept = (shared.len() + only.len()) as u64;
    assert_eq!(store.usage().entries, 1);
    assert!(store.usage().bytes > kept);
    assert!(matches!(
        store.read_blob(&foreign.blobs[1].handle),
        Err(Error::NotFound)
    ));

    store.delete(&mine.id).unwrap();
    let again = store
        .put(NewRecord {
            data: json!({ "text": "again" }),
            blobs: vec![only.clone(), b"written this run".to_vec()],
        })
        .unwrap();
    assert_eq!(again.blobs[0].handle, foreign.blobs[1].handle);
    assert_eq!(&**store.read_blob(&again.blobs[0].handle).unwrap(), &only[..]);
    store.delete(&again.id).unwrap();
    assert_eq!(files_in(&blob_dir), blobs, "only the blob written this run goes");
    assert_eq!(store.usage().bytes, kept);

    let tight = Limits {
        max_total_bytes: kept + 8,
        max_item_bytes: kept + 8,
        ..Limits::default()
    };
    store.set_limits(tight).unwrap();
    store.put(text("x")).unwrap();
    assert!(
        texts(&store).is_empty(),
        "kept blobs take their share of the quota"
    );
    assert_eq!(files_in(&blob_dir), blobs);
}

#[test]
fn files_in_a_newer_format_are_skipped_not_deleted() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let newer = store.put(text("newer")).unwrap();
    drop(store);
    let record = fixture
        .namespace()
        .join("records")
        .join(format!("{}.rec", newer.id));
    let bytes = bump_format(&record);

    let store = fixture.open(Limits::default());
    assert_eq!(store.open_report().skipped_records, 1);
    assert!(texts(&store).is_empty());
    assert_eq!(fs::read(&record).unwrap(), bytes);
}

#[test]
fn a_key_check_in_a_newer_format_is_kept_and_the_records_judge_the_key() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let older = store.put(text("older")).unwrap();
    let newer = store.put(text("newer")).unwrap();
    drop(store);
    let records = fixture.namespace().join("records");
    let check = fixture.namespace().join("key.check");
    let check_bytes = bump_format(&check);
    bump_format(&records.join(format!("{}.rec", newer.id)));
    let record_files = files_in(&records);

    assert!(matches!(
        fixture.other_key().open("clipboard", Limits::default()),
        Err(Error::WrongKey)
    ));
    assert_eq!(files_in(&records), record_files);

    let store = fixture.open(Limits::default());
    assert_eq!(store.open_report().skipped_records, 1);
    assert_eq!(store.open_report().corrupt_records, 0);
    assert_eq!(texts(&store), ["older"]);
    store.delete(&older.id).unwrap();
    drop(store);
    assert_eq!(fs::read(&check).unwrap(), check_bytes);

    // Only files this build can't judge are left: the namespace opens, and nothing changes.
    let store = fixture.open(Limits::default());
    assert_eq!(store.open_report().skipped_records, 1);
    assert!(texts(&store).is_empty());
    drop(store);
    assert_eq!(fs::read(&check).unwrap(), check_bytes);
    assert_eq!(files_in(&records).len(), 1);
}

#[test]
fn a_state_in_a_newer_format_refuses_every_open_and_is_kept() {
    let fixture = Fixture::new();
    let device = fixture.open(Limits::default()).device_id();
    let state = fixture.state_path();
    let bytes = bump_format(&state);

    let storage = fixture.storage();
    assert!(matches!(
        storage.open("clipboard", Limits::default()),
        Err(Error::NewerFormat)
    ));
    assert!(matches!(
        storage.open("fresh", Limits::default()),
        Err(Error::NewerFormat)
    ));
    assert_eq!(fs::read(&state).unwrap(), bytes);
    assert!(!fixture.root().join("fresh/key.check").exists());

    fs::write(&state, {
        let mut restored = bytes;
        restored[4] -= 1;
        restored
    })
    .unwrap();
    assert_eq!(fixture.open(Limits::default()).device_id(), device);
}

#[test]
fn a_new_namespace_under_another_key_leaves_the_device_state_alone() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let device = store.device_id();
    let before = store.put(text("before")).unwrap();
    drop(store);
    let state = fixture.state_path();
    let bytes = fs::read(&state).unwrap();

    let other = fixture.other_key();
    assert!(matches!(
        other.open("clipboard", Limits::default()),
        Err(Error::WrongKey)
    ));
    assert!(matches!(
        other.open("ext.new", Limits::default()),
        Err(Error::WrongKey)
    ));
    assert_eq!(fs::read(&state).unwrap(), bytes);
    assert!(!fixture.root().join("ext.new/key.check").exists());

    let storage = fixture.storage();
    let fresh = storage.open("ext.new", Limits::default()).unwrap();
    assert_eq!(fresh.device_id(), device);
    let after = fresh.put(text("after")).unwrap();
    assert_eq!(after.device(), device);
    drop(fresh);
    let store = storage.open("clipboard", Limits::default()).unwrap();
    assert_eq!(store.get(&before.id).unwrap(), Some(before));
}

#[test]
fn a_damaged_state_is_replaced_when_a_namespace_proves_the_key() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let device = store.device_id();
    store.put(text("kept")).unwrap();
    drop(store);
    let state = fixture.state_path();
    let mut bytes = fs::read(&state).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&state, &bytes).unwrap();

    let store = fixture.storage().open("ext.new", Limits::default()).unwrap();
    assert_ne!(store.device_id(), device);
    assert_ne!(fs::read(&state).unwrap(), bytes);
    drop(store);
    assert_eq!(texts(&fixture.open(Limits::default())), ["kept"]);
}

#[test]
fn records_carry_the_device_that_wrote_them() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits::default());
    let put = store.put(text("x")).unwrap();
    assert_eq!(put.device(), store.device_id());
    let device = store.device_id();
    drop(store);
    let store = fixture.open(Limits::default());
    assert_eq!(store.device_id(), device);
    assert_eq!(store.get(&put.id).unwrap().unwrap().device(), device);
    assert_eq!(
        device.to_string().parse::<dupin_storage::DeviceId>().unwrap(),
        device
    );
}

#[test]
fn a_data_directory_moved_to_other_hardware_writes_as_a_new_device() {
    let fixture = Fixture::new();
    let open = |hardware: &[u8]| {
        Storage::new(fixture.root(), MemoryKey::new(fixture.key.clone()))
            .with_hardware_id(hardware)
            .open("clipboard", Limits::default())
            .unwrap()
    };
    let store = open(b"machine-a");
    let written = store.put(text("x")).unwrap();
    let here = store.device_id();
    drop(store);
    assert_eq!(open(b"machine-a").device_id(), here);

    let store = open(b"machine-b");
    assert_ne!(store.device_id(), here);
    assert_eq!(store.get(&written.id).unwrap().unwrap().device(), here);
    assert_eq!(store.put(text("y")).unwrap().device(), store.device_id());
}

#[test]
fn an_upsert_within_limits_counts_the_record_it_replaces_as_room() {
    let fixture = Fixture::new();
    let store = fixture.open(Limits {
        max_entries: 2,
        max_total_bytes: 64,
        max_item_bytes: 64,
        ..Limits::default()
    });
    let image = |bytes: &[u8]| NewRecord {
        data: json!({ "text": "image" }),
        blobs: vec![bytes.to_vec()],
    };
    store.put_within_limits(text("first")).unwrap();
    store.upsert_within_limits("copy", image(&[1; 30])).unwrap();
    assert!(
        matches!(
            store.put_within_limits(text("third")),
            Err(dupin_storage::Error::QuotaExceeded { .. })
        ),
        "a new record needs a third entry"
    );
    store
        .upsert_within_limits("copy", image(&[2; 30]))
        .expect("the replaced record and its blob make room for the new one");
    assert_eq!(texts(&store), ["image", "first"]);
    assert!(matches!(
        store.upsert_within_limits("copy", image(&[3; 40])),
        Err(dupin_storage::Error::QuotaExceeded { .. })
    ));
    assert_eq!(
        texts(&store),
        ["image", "first"],
        "a refused upsert deletes nothing"
    );
}
