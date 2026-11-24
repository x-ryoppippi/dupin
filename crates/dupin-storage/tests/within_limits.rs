use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dupin_storage::{Clock, Error, Limits, MemoryKey, NewRecord, Storage, Usage};
use serde_json::json;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone)]
struct FakeClock(Arc<AtomicU64>);

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

fn record(text: &str, blob: &[u8]) -> NewRecord {
    NewRecord {
        data: json!(text),
        blobs: vec![blob.to_vec()],
    }
}

fn limits(max_entries: usize, max_total_bytes: u64) -> Limits {
    Limits {
        max_age: None,
        max_entries,
        max_total_bytes,
        max_item_bytes: max_total_bytes,
    }
}

#[test]
fn a_record_with_no_room_is_refused_and_nothing_older_goes() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(dir.path(), MemoryKey::random());
    // `"a"` is 3 bytes of JSON, so each record below is 3 + 10 bytes.
    let store = storage.open("ext-u-a", limits(10, 30)).unwrap();
    store.put_within_limits(record("a", &[1; 10])).unwrap();
    store.put_within_limits(record("b", &[2; 10])).unwrap();
    assert!(matches!(
        store.put_within_limits(record("c", &[3; 10])),
        Err(Error::QuotaExceeded {
            entries: 3,
            bytes: 39
        })
    ));
    // The same blob again costs only the record's JSON.
    store.put_within_limits(record("d", &[1; 10])).unwrap();
    assert_eq!(
        store.usage(),
        Usage {
            entries: 3,
            bytes: 29
        }
    );

    let store = {
        drop(store);
        storage.open("ext-u-a", limits(3, 1024)).unwrap()
    };
    assert!(matches!(
        store.put_within_limits(record("e", &[])),
        Err(Error::QuotaExceeded { entries: 4, .. })
    ));
    assert_eq!(store.usage().entries, 3);
}

#[test]
fn records_past_their_age_make_room_first() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock(Arc::new(AtomicU64::new(1_700_000_000_000)));
    let storage = Storage::new(dir.path(), MemoryKey::random()).with_clock(clock.clone());
    let store = storage
        .open(
            "ext-u-a",
            Limits {
                max_age: Some(DAY),
                ..limits(1, 1024)
            },
        )
        .unwrap();
    store.put_within_limits(record("old", &[])).unwrap();
    assert!(store.put_within_limits(record("new", &[])).is_err());
    clock.0.fetch_add(2 * DAY.as_millis() as u64, Ordering::SeqCst);
    store.put_within_limits(record("new", &[])).unwrap();
    assert_eq!(store.list(None, 10).unwrap().records[0].data, json!("new"));
}

#[test]
fn concurrent_writers_never_pass_the_limits_together() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(dir.path(), MemoryKey::random());
    // Ten records of 3 + 97 bytes fit; the other writers must be refused.
    let store = Arc::new(storage.open("ext-u-a", limits(1000, 1000)).unwrap());
    let barrier = Arc::new(Barrier::new(32));
    let writers: Vec<_> = (0..32u8)
        .map(|index| {
            let (store, barrier) = (store.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                store.put_within_limits(record("x", &[index; 97])).is_ok()
            })
        })
        .collect();
    let stored = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .filter(|stored| *stored)
        .count();
    assert_eq!(stored, 10);
    assert_eq!(
        store.usage(),
        Usage {
            entries: 10,
            bytes: 1000
        }
    );
}
