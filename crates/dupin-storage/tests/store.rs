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

    fn storage(&self) -> Storage {
        Storage::new(self.dir.path(), MemoryKey::new(self.key.clone())).with_clock(self.clock.clone())
    }

    fn open(&self, limits: Limits) -> Store {
        self.storage().open("clipboard", limits).unwrap()
    }

    fn namespace(&self) -> PathBuf {
        self.dir.path().join("clipboard")
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
    assert_eq!(store.get(&"0".repeat(24).parse().unwrap()).unwrap(), None);
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
    bytes[20] ^= 0x80;
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

    let other = Storage::new(fixture.dir.path(), MemoryKey::random());
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

    let other = Storage::new(fixture.dir.path(), MemoryKey::random());
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
    assert_eq!(secret.created_at(), now);
    assert!(secret.id > before.id);
    drop(store);

    let store = fixture.open(limits);
    let reopened = store.put(text("reopened")).unwrap();
    assert_eq!(reopened.created_at(), now);
    assert!(reopened.id > secret.id);

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

    let other = Storage::new(fixture.dir.path(), MemoryKey::random());
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
    let from = fixture.dir.path().join("ext.a");
    let to = fixture.dir.path().join("ext.b");
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
    let other = Storage::new(fixture.dir.path(), MemoryKey::random());
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
    let mut ids: Vec<_> = all.records.iter().map(|record| record.id).collect();
    let mut sorted = ids.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(ids, sorted, "newest first");
    ids.dedup();
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
