//! Several devices, each with its own data directory and store, syncing through one folder on
//! this machine.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dupin_storage::{Clock, DeviceId, Limits, MemoryKey, NewRecord, Storage, Store};
use serde_json::json;

use crate::keys::MemoryKeys;
use crate::{
    DeviceInfo, Engine, Error, Folder, FolderOptions, Joined, KdfParams, KeyMode, Mode, Namespace,
    Passphrase, Report,
};

const DAY: Duration = Duration::from_secs(24 * 3600);
const NAMESPACE: &str = "notes";

#[derive(Clone)]
struct FakeClock(Arc<AtomicU64>);

impl FakeClock {
    fn new() -> Self {
        Self(Arc::new(AtomicU64::new(1_800_000_000_000)))
    }

    fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

fn passphrase() -> Passphrase {
    Passphrase::chosen("correct horse battery staple").unwrap()
}

struct World {
    _dir: tempfile::TempDir,
    folder: PathBuf,
    clock: FakeClock,
}

struct Device {
    data: PathBuf,
    storage: Arc<Storage>,
    engine: Engine,
    store: Option<Store>,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("iCloud/Dupin");
        Self {
            folder,
            _dir: dir,
            clock: FakeClock::new(),
        }
    }

    fn data_dir(&self, name: &str) -> PathBuf {
        self._dir.path().join(name)
    }

    fn storage(&self, name: &str, key: MemoryKey) -> Arc<Storage> {
        Arc::new(Storage::new(self.data_dir(name).join("storage"), key).with_clock(self.clock.clone()))
    }

    fn open_folder(&self, name: &str) -> Folder {
        Folder::open(&self.folder, &self.data_dir(name), FolderOptions::default()).unwrap()
    }

    fn device(
        &self,
        name: &'static str,
        joined: impl FnOnce(&Folder, &Arc<Storage>, &MemoryKeys) -> Joined,
    ) -> Device {
        self.device_with(name, MemoryKey::random(), joined)
    }

    fn device_with(
        &self,
        name: &'static str,
        key: MemoryKey,
        joined: impl FnOnce(&Folder, &Arc<Storage>, &MemoryKeys) -> Joined,
    ) -> Device {
        let storage = self.storage(name, key);
        let folder = self.open_folder(name);
        let keys = MemoryKeys::default();
        let joined = joined(&folder, &storage, &keys);
        let store = storage
            .open_synced(NAMESPACE, Limits::default(), &joined.key)
            .unwrap();
        let engine = Engine::new(
            folder,
            storage.clone(),
            joined.key,
            joined.header,
            DeviceInfo {
                name: name.into(),
                platform: "macos".into(),
                version: "test".into(),
            },
            Arc::new(self.clock.clone()),
        )
        .unwrap();
        Device {
            data: self.data_dir(name),
            storage,
            engine,
            store: Some(store),
        }
    }

    fn first(&self, name: &'static str) -> Device {
        self.device(name, |folder, storage, keys| {
            crate::create(
                folder,
                storage.device_id().unwrap(),
                &passphrase(),
                KdfParams::FLOOR,
                keys,
                None,
            )
            .unwrap()
        })
    }

    fn joiner(&self, name: &'static str) -> Device {
        self.device(name, |folder, _, keys| {
            crate::join(folder, Some(&passphrase()), keys, None).unwrap()
        })
    }
}

impl Device {
    fn store(&self) -> &Store {
        self.store.as_ref().unwrap()
    }

    fn id(&self) -> DeviceId {
        self.storage.device_id().unwrap()
    }

    fn sync(&mut self) -> Report {
        let store = self.store.as_ref().unwrap();
        let namespace = Namespace {
            name: NAMESPACE,
            store,
            mode: Mode::Lww,
            fold: None,
        };
        self.engine.run(&[namespace], &[]).unwrap()
    }

    fn leave(&mut self) -> Report {
        self.engine.run(&[], &[NAMESPACE]).unwrap()
    }

    fn set(&self, key: &str, value: &str) {
        self.store()
            .upsert(
                key,
                NewRecord {
                    data: json!(value),
                    blobs: Vec::new(),
                },
            )
            .unwrap();
    }

    fn get(&self, key: &str) -> Option<String> {
        let store = self.store();
        store
            .get(&store.key_id(key))
            .unwrap()
            .map(|record| record.data.as_str().unwrap().to_owned())
    }

    fn values(&self) -> Vec<String> {
        let mut values: Vec<String> = self
            .store()
            .list(None, 200)
            .unwrap()
            .records
            .into_iter()
            .map(|record| record.data.as_str().unwrap().to_owned())
            .collect();
        values.sort();
        values
    }

    fn ns_dir(&self, world: &World) -> PathBuf {
        world.folder.join("ns").join(self.store().namespace_id())
    }

    fn own_dir(&self, world: &World) -> PathBuf {
        self.ns_dir(world).join(self.id().to_string())
    }
}

fn files_under(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(path) else {
        return out;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let meta = std::fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

fn sync_all(devices: &mut [&mut Device]) {
    for _ in 0..2 {
        for device in devices.iter_mut() {
            device.sync();
        }
    }
}

#[test]
fn two_devices_converge_and_nothing_lands_in_the_clear() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    a.set("greeting", "hello from a");
    a.store()
        .put(NewRecord {
            data: json!("a picture"),
            blobs: vec![b"PIXELDATA-SECRET".to_vec()],
        })
        .unwrap();
    sync_all(&mut [&mut a, &mut b]);
    assert_eq!(b.get("greeting").as_deref(), Some("hello from a"));
    assert_eq!(b.values(), ["a picture", "hello from a"]);

    world.clock.advance(Duration::from_secs(1));
    b.set("greeting", "edited on b");
    sync_all(&mut [&mut a, &mut b]);
    assert_eq!(a.get("greeting").as_deref(), Some("edited on b"));
    assert_eq!(a.values(), b.values());

    let header = std::fs::read(world.folder.join("vault.dupin")).unwrap();
    for file in files_under(&world.folder) {
        let bytes = std::fs::read(&file).unwrap();
        for secret in [
            &b"hello from a"[..],
            b"edited on b",
            b"PIXELDATA-SECRET",
            b"greeting",
            NAMESPACE.as_bytes(),
            b"correct horse",
        ] {
            assert!(!contains(&bytes, secret), "{} holds {:?}", file.display(), secret);
        }
        if bytes != header {
            assert!(
                !contains(&bytes, b"\"vault\""),
                "only the vault header is plaintext JSON"
            );
        }
    }
    let ns = world.folder.join("ns");
    let namespaces: Vec<_> = std::fs::read_dir(&ns).unwrap().collect();
    assert_eq!(namespaces.len(), 1, "one folder per namespace, named by its ID");
}

#[test]
fn each_device_writes_only_its_own_directory() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    a.set("k1", "a");
    b.set("k2", "b");
    sync_all(&mut [&mut a, &mut b]);
    let mut dirs: Vec<String> = std::fs::read_dir(a.ns_dir(&world))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    dirs.sort();
    let mut expected = vec![a.id().to_string(), b.id().to_string(), "blobs".to_owned()];
    expected.sort();
    assert_eq!(dirs, expected);
    assert_eq!(files_under(&a.own_dir(&world)).len(), 1);
    assert_eq!(files_under(&b.own_dir(&world)).len(), 1);
    assert_eq!(a.values(), ["a", "b"]);

    // B's later edit of A's record supersedes A's file, which A then removes.
    world.clock.advance(Duration::from_secs(1));
    b.set("k1", "b took over");
    sync_all(&mut [&mut a, &mut b]);
    assert_eq!(files_under(&a.own_dir(&world)).len(), 0);
    assert_eq!(files_under(&b.own_dir(&world)).len(), 2);
    let devices: Vec<_> = std::fs::read_dir(world.folder.join("devices")).unwrap().collect();
    assert_eq!(devices.len(), 2);
}

#[test]
fn concurrent_edits_settle_on_the_later_one_everywhere() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    a.set("k", "a first");
    world.clock.advance(Duration::from_millis(10));
    b.set("k", "b later");
    sync_all(&mut [&mut a, &mut b]);
    assert_eq!(a.get("k").as_deref(), Some("b later"));
    assert_eq!(b.get("k").as_deref(), Some("b later"));
}

#[test]
fn deletions_propagate_and_tombstones_are_collected_once_acknowledged() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    a.set("doomed", "x");
    sync_all(&mut [&mut a, &mut b]);
    world.clock.advance(Duration::from_secs(1));
    let id = a.store().key_id("doomed");
    assert!(a.store().delete(&id).unwrap());
    sync_all(&mut [&mut a, &mut b]);
    assert_eq!(b.get("doomed"), None);
    assert_eq!(files_under(&a.own_dir(&world)).len(), 1, "the tombstone stays");

    world.clock.advance(29 * DAY);
    sync_all(&mut [&mut a, &mut b]);
    assert_eq!(files_under(&a.own_dir(&world)).len(), 1, "not before 30 days");
    world.clock.advance(2 * DAY);
    sync_all(&mut [&mut a, &mut b]);
    assert!(files_under(&a.own_dir(&world)).is_empty());
    assert!(a.store().versions().is_empty());
    assert_eq!(b.get("doomed"), None);
}

#[test]
fn a_tombstone_waits_for_a_device_that_has_not_acknowledged_it() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    a.set("doomed", "x");
    sync_all(&mut [&mut a, &mut b]);
    let id = a.store().key_id("doomed");
    a.store().delete(&id).unwrap();
    // B is away: A syncs alone for longer than 30 days.
    world.clock.advance(40 * DAY);
    for _ in 0..3 {
        a.sync();
    }
    assert_eq!(files_under(&a.own_dir(&world)).len(), 1);
    // B comes back with its old copy, which must not resurrect.
    sync_all(&mut [&mut b, &mut a]);
    assert_eq!(b.get("doomed"), None);
    assert_eq!(a.get("doomed"), None);
    assert!(files_under(&a.own_dir(&world)).is_empty());
}

#[test]
fn a_clear_on_one_device_clears_older_versions_everywhere() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    a.set("one", "1");
    b.set("two", "2");
    sync_all(&mut [&mut a, &mut b]);
    world.clock.advance(Duration::from_secs(1));
    b.store().clear().unwrap();
    world.clock.advance(Duration::from_secs(1));
    a.set("three", "3");
    sync_all(&mut [&mut a, &mut b]);
    assert_eq!(a.values(), ["3"]);
    assert_eq!(b.values(), ["3"]);
}

#[test]
fn blobs_arrive_after_their_records_and_equal_blobs_are_one_file() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    let image = || NewRecord {
        data: json!("screenshot"),
        blobs: vec![vec![7; 4096]],
    };
    a.store().put(image()).unwrap();
    b.store().put(image()).unwrap();
    sync_all(&mut [&mut a, &mut b]);
    let blobs = files_under(&a.ns_dir(&world).join("blobs"));
    assert_eq!(blobs.len(), 1);
    assert_eq!(a.values(), ["screenshot", "screenshot"]);

    // A blob missing from the folder holds its record back until a holder uploads it again.
    let c = NewRecord {
        data: json!("late"),
        blobs: vec![vec![9; 100]],
    };
    a.store().put(c).unwrap();
    a.sync();
    for blob in files_under(&a.ns_dir(&world).join("blobs")) {
        std::fs::remove_file(blob).unwrap();
    }
    b.sync();
    assert!(!b.values().contains(&"late".to_owned()));
    a.sync();
    b.sync();
    assert!(b.values().contains(&"late".to_owned()));
}

#[test]
fn tampered_truncated_and_garbage_files_are_skipped_and_kept() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    b.set("first", "1");
    b.set("second", "2");
    b.sync();
    let b_dir = b.own_dir(&world);
    let mut files = files_under(&b_dir);
    files.sort();
    let mut tampered = std::fs::read(&files[0]).unwrap();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    std::fs::write(&files[0], &tampered).unwrap();
    let truncated = std::fs::read(&files[1]).unwrap();
    std::fs::write(&files[1], &truncated[..truncated.len() / 2]).unwrap();
    std::fs::write(b_dir.join(format!("{}.rec", "ab".repeat(16))), b"garbage").unwrap();
    std::fs::write(b_dir.join(format!("{} 2.rec", "cd".repeat(16))), b"copy").unwrap();
    std::fs::write(b_dir.join(".DS_Store"), b"finder").unwrap();
    std::fs::write(a.ns_dir(&world).join("blobs").join("junk.blob"), b"x").unwrap_or(());

    let report = a.sync();
    assert!(a.values().is_empty());
    assert_eq!(report.unreadable.len(), 3, "{:?}", report.unreadable);
    assert!(
        std::fs::read(&files[0]).unwrap() == tampered,
        "never deleted by a reader"
    );
    assert!(b_dir.join(format!("{} 2.rec", "cd".repeat(16))).exists());
    assert!(b_dir.join(".DS_Store").exists());
}

#[test]
fn replayed_older_files_never_win() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    b.set("k", "old");
    b.sync();
    let file = files_under(&b.own_dir(&world)).pop().unwrap();
    let old = std::fs::read(&file).unwrap();
    a.sync();
    world.clock.advance(Duration::from_secs(1));
    b.set("k", "new");
    b.sync();
    a.sync();
    assert_eq!(a.get("k").as_deref(), Some("new"));
    std::fs::write(&file, &old).unwrap();
    a.sync();
    assert_eq!(a.get("k").as_deref(), Some("new"));
    // A newly joined device that sees only the replayed file takes it, as the design allows,
    // until B's next export puts the newer one back.
    b.sync();
    a.sync();
    assert_eq!(a.get("k").as_deref(), Some("new"));
}

#[test]
fn links_in_the_folder_are_never_followed() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    b.set("k", "v");
    sync_all(&mut [&mut a, &mut b]);
    let outside = world.data_dir("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), b"private").unwrap();

    // A link named like a record in B's directory, pointing outside the folder.
    let b_dir = b.own_dir(&world);
    std::os::unix::fs::symlink(
        outside.join("secret.txt"),
        b_dir.join(format!("{}.rec", "ef".repeat(16))),
    )
    .unwrap();
    // A's own directory replaced by a link to somewhere else: A must not write through it.
    let a_dir = a.own_dir(&world);
    std::fs::remove_dir_all(&a_dir).unwrap();
    std::os::unix::fs::symlink(&outside, &a_dir).unwrap();
    a.set("mine", "must not leak");
    let result = {
        let store = a.store.as_ref().unwrap();
        a.engine.run(
            &[Namespace {
                name: NAMESPACE,
                store,
                mode: Mode::Lww,
                fold: None,
            }],
            &[],
        )
    };
    assert!(result.is_err(), "writing through a planted link fails");
    assert_eq!(
        std::fs::read_dir(&outside).unwrap().count(),
        1,
        "nothing was written outside the folder"
    );
    assert_eq!(std::fs::read(outside.join("secret.txt")).unwrap(), b"private");

    // B's whole directory replaced with a link: A no longer reads it, and keeps what it has.
    std::fs::remove_file(&a_dir).unwrap();
    let moved = world.data_dir("moved");
    std::fs::rename(&b_dir, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &b_dir).unwrap();
    a.sync();
    assert_eq!(a.get("k").as_deref(), Some("v"));
    assert!(
        std::fs::symlink_metadata(&b_dir)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn a_wrong_passphrase_or_a_downgraded_kdf_is_refused_without_changes() {
    let world = World::new();
    let _a = world.first("a");
    let header = std::fs::read(world.folder.join("vault.dupin")).unwrap();
    let folder = world.open_folder("b");
    let keys = MemoryKeys::default();
    assert!(matches!(
        crate::join(
            &folder,
            Some(&Passphrase::entered("not the passphrase")),
            &keys,
            None
        ),
        Err(Error::WrongPassphrase)
    ));
    assert!(matches!(
        crate::join(&folder, None, &keys, None),
        Err(Error::KeyUnavailable)
    ));
    assert_eq!(std::fs::read(world.folder.join("vault.dupin")).unwrap(), header);

    let text = String::from_utf8(header[5..].to_vec()).unwrap();
    let mut downgraded = header[..5].to_vec();
    downgraded.extend_from_slice(
        text.replace("\"memory_kib\": 65536", "\"memory_kib\": 1024")
            .as_bytes(),
    );
    std::fs::write(world.folder.join("vault.dupin"), &downgraded).unwrap();
    assert!(matches!(
        crate::join(&folder, Some(&passphrase()), &keys, None),
        Err(Error::UnacceptableKdf(_))
    ));
    assert!(crate::join(&folder, Some(&passphrase()), &keys, None).is_err());
}

#[test]
fn a_tampered_header_is_put_back_by_a_member() {
    let world = World::new();
    let mut a = world.first("a");
    let header = std::fs::read(world.folder.join("vault.dupin")).unwrap();
    let text = String::from_utf8(header[5..].to_vec()).unwrap();
    let mut tampered = header[..5].to_vec();
    tampered.extend_from_slice(
        text.replace("\"mode\": \"passphrase\"", "\"mode\": \"keychain\"")
            .as_bytes(),
    );
    std::fs::write(world.folder.join("vault.dupin"), &tampered).unwrap();
    a.sync();
    assert_eq!(std::fs::read(world.folder.join("vault.dupin")).unwrap(), header);
}

#[test]
fn creating_a_second_vault_in_a_claimed_folder_is_refused() {
    let world = World::new();
    let _a = world.first("a");
    let storage = world.storage("b", MemoryKey::random());
    let folder = world.open_folder("b");
    assert!(matches!(
        crate::create(
            &folder,
            storage.device_id().unwrap(),
            &passphrase(),
            KdfParams::FLOOR,
            &MemoryKeys::default(),
            None
        ),
        Err(Error::Refused(crate::Refusal::HoldsAVault))
    ));
    assert!(matches!(
        crate::create(
            &folder,
            storage.device_id().unwrap(),
            &Passphrase::entered("unchecked passphrase"),
            KdfParams::FLOOR,
            &MemoryKeys::default(),
            None
        ),
        Err(Error::KeyUnavailable)
    ));
}

#[test]
fn competing_vaults_settle_on_the_lowest_id() {
    let world = World::new();
    let mut a = world.first("a");
    let other = World::new();
    let c = other.first("c");
    let c_header = std::fs::read(other.folder.join("vault.dupin")).unwrap();
    std::fs::write(world.folder.join("vault 2.dupin"), &c_header).unwrap();
    let report = a.sync();
    let a_id = a.engine.vault().id();
    let c_id = c.engine.vault().id();
    if c_id < a_id {
        assert!(report.losing_vault);
        assert_eq!(
            files_under(&world.folder.join("ns")).len(),
            0,
            "a losing Mac exports nothing"
        );
    } else {
        assert!(!report.losing_vault);
        assert!(
            !world.folder.join("vault 2.dupin").exists(),
            "the losing copy is removed"
        );
    }
    let joiner = world.open_folder("d");
    let joined = crate::join(&joiner, Some(&passphrase()), &MemoryKeys::default(), None);
    if c_id < a_id {
        assert_eq!(joined.unwrap().key.id(), c_id, "a new Mac joins the winner");
    } else {
        assert_eq!(joined.unwrap().key.id(), a_id);
    }
}

#[test]
fn a_tampered_descriptor_is_reported_and_ignored() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    sync_all(&mut [&mut a, &mut b]);
    let path = world.folder.join("devices").join(format!("{}.dev", b.id()));
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[10] ^= 1;
    std::fs::write(&path, &bytes).unwrap();
    let report = a.sync();
    assert!(report.unreadable.iter().any(|path| path.starts_with("devices/")));
    assert_eq!(report.devices.len(), 0);
}

#[test]
fn a_clock_far_ahead_is_named_and_held() {
    let world = World::new();
    let mut a = world.first("a");
    // B runs an hour fast.
    let fast = FakeClock::new();
    fast.advance(Duration::from_secs(3600));
    let b_storage = Arc::new(
        Storage::new(world.data_dir("b").join("storage"), MemoryKey::random()).with_clock(fast.clone()),
    );
    let b_folder = world.open_folder("b");
    let keys = MemoryKeys::default();
    let joined = crate::join(&b_folder, Some(&passphrase()), &keys, None).unwrap();
    let b_store = b_storage
        .open_synced(NAMESPACE, Limits::default(), &joined.key)
        .unwrap();
    let mut b_engine = Engine::new(
        b_folder,
        b_storage.clone(),
        joined.key,
        joined.header,
        DeviceInfo::default(),
        Arc::new(fast),
    )
    .unwrap();
    b_store
        .upsert(
            "k",
            NewRecord {
                data: json!("from the future"),
                blobs: Vec::new(),
            },
        )
        .unwrap();
    b_engine
        .run(
            &[Namespace {
                name: NAMESPACE,
                store: &b_store,
                mode: Mode::Lww,
                fold: None,
            }],
            &[],
        )
        .unwrap();
    let report = a.sync();
    assert!(report.clock_ahead.contains(&b_storage.device_id().unwrap()));
    assert_eq!(a.get("k"), None);
    world.clock.advance(Duration::from_secs(3600));
    a.sync();
    assert_eq!(a.get("k").as_deref(), Some("from the future"));
}

#[test]
fn a_leaving_device_hands_its_files_over_before_removing_them() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    a.set("only-a", "a wrote this");
    sync_all(&mut [&mut a, &mut b]);
    assert_eq!(b.get("only-a").as_deref(), Some("a wrote this"));

    a.leave();
    assert!(a.own_dir(&world).exists(), "kept until B takes it over");
    b.sync();
    a.leave();
    assert!(!a.own_dir(&world).exists());
    assert_eq!(files_under(&b.own_dir(&world)).len(), 1, "B carries A's version");

    // A new device still gets the record.
    let mut c = world.joiner("c");
    c.sync();
    assert_eq!(c.get("only-a").as_deref(), Some("a wrote this"));
}

#[test]
fn a_device_that_rejoins_drops_what_was_deleted_while_it_was_away() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    a.set("doomed", "x");
    a.set("kept", "y");
    sync_all(&mut [&mut a, &mut b]);
    b.leave();
    a.sync();
    b.leave();
    let id = a.store().key_id("doomed");
    world.clock.advance(Duration::from_secs(1));
    a.store().delete(&id).unwrap();
    world.clock.advance(31 * DAY);
    sync_all(&mut [&mut a]);
    assert!(
        a.store().versions().iter().all(|version| version.id != id),
        "collected without B"
    );

    world.clock.advance(Duration::from_secs(1));
    b.set("written-after", "z");
    b.sync();
    b.sync();
    a.sync();
    assert_eq!(b.get("doomed"), None, "nothing deleted comes back");
    assert_eq!(b.get("kept").as_deref(), Some("y"));
    assert_eq!(a.get("written-after").as_deref(), Some("z"));
    assert_eq!(a.get("doomed"), None);
}

#[test]
fn a_removed_mac_is_taken_over_and_rejoins_under_a_new_id() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    let mut c = world.joiner("c");
    c.set("from-c", "c's record");
    sync_all(&mut [&mut a, &mut b, &mut c]);
    let old_c = c.id();
    a.engine.retire(old_c).unwrap();
    sync_all(&mut [&mut a, &mut b]);
    sync_all(&mut [&mut a, &mut b]);
    assert!(!a.ns_dir(&world).join(old_c.to_string()).exists());
    assert!(!world.folder.join("devices").join(format!("{old_c}.dev")).exists());
    assert_eq!(a.get("from-c").as_deref(), Some("c's record"));

    let report = c.sync();
    assert!(report.reminted);
    assert_ne!(c.id(), old_c);
    sync_all(&mut [&mut c, &mut a, &mut b]);
    assert_eq!(c.get("from-c").as_deref(), Some("c's record"));
    assert!(!a.ns_dir(&world).join(old_c.to_string()).exists());
}

#[test]
fn a_cloned_data_directory_takes_a_new_device_id() {
    let world = World::new();
    let key = MemoryKey::random();
    let mut a = world.device_with("a", key.clone(), |folder, storage, keys| {
        crate::create(
            folder,
            storage.device_id().unwrap(),
            &passphrase(),
            KdfParams::FLOOR,
            keys,
            None,
        )
        .unwrap()
    });
    a.set("k", "original");
    a.sync();
    a.store.take();
    copy_tree(&a.data, &world.data_dir("clone"));
    a.store = Some(
        a.storage
            .open_synced(NAMESPACE, Limits::default(), a.engine.vault())
            .unwrap(),
    );
    let mut clone = world.device_with("clone", key, |folder, _, keys| {
        crate::join(folder, Some(&passphrase()), keys, None).unwrap()
    });
    assert_eq!(clone.id(), a.id());
    world.clock.advance(Duration::from_secs(1));
    clone.set("k", "clone edit");
    clone.sync();
    let report = a.sync();
    assert!(
        report.reminted,
        "A finds a version in its directory it never wrote"
    );
    assert_ne!(a.id(), clone.id());
    sync_all(&mut [&mut a, &mut clone]);
    assert_eq!(a.get("k").as_deref(), Some("clone edit"));
}

#[test]
fn icloud_keychain_mode_joins_without_the_passphrase_and_keeps_a_backup() {
    let world = World::new();
    let shared = MemoryKeys::default();
    let storage = world.storage("a", MemoryKey::random());
    let folder = world.open_folder("a");
    let created = crate::create(
        &folder,
        storage.device_id().unwrap(),
        &passphrase(),
        KdfParams::FLOOR,
        &MemoryKeys::default(),
        Some(&shared),
    )
    .unwrap();
    assert_eq!(created.header.mode, KeyMode::Keychain);
    assert!(created.header.has_passphrase());
    let joiner = world.open_folder("b");
    let local = MemoryKeys::default();
    let joined = crate::join(&joiner, None, &local, Some(&shared)).unwrap();
    assert_eq!(joined.key.key().as_bytes(), created.key.key().as_bytes());
    let resumed = crate::resume(joined.key.id(), 1, Some(&joiner), &local).unwrap();
    assert_eq!(resumed.key.key().as_bytes(), created.key.key().as_bytes());
    let backup = crate::join(&joiner, Some(&passphrase()), &MemoryKeys::default(), None).unwrap();
    assert_eq!(backup.key.key().as_bytes(), created.key.key().as_bytes());
}

#[test]
fn the_folder_cannot_be_the_data_directory() {
    let world = World::new();
    let data = world.data_dir("a");
    std::fs::create_dir_all(data.join("storage")).unwrap();
    assert!(matches!(
        Folder::open(&data.join("storage/sync"), &data, FolderOptions::default()),
        Err(Error::Refused(crate::Refusal::InsideDataDirectory))
    ));
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else if entry.file_name() != "lock" {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A small deterministic generator, so a failing simulation can be replayed from its seed.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }
}

/// Three devices edit, delete and sync in a random order while the folder truncates files,
/// grows sync tools' copies and time jumps by up to days. Every device must end with the
/// last write of every key, and nothing deleted may come back.
#[test]
fn random_interleavings_converge_on_the_last_write() {
    for seed in 1..=4u64 {
        let world = World::new();
        let mut devices = vec![world.first("a"), world.joiner("b"), world.joiner("c")];
        let mut rng = Lcg(seed);
        let mut model: std::collections::BTreeMap<String, Option<String>> = Default::default();
        for step in 0..160 {
            world.clock.advance(Duration::from_millis(1 + rng.next(5)));
            let device = rng.next(devices.len() as u64) as usize;
            let key = format!("k{}", rng.next(5));
            match rng.next(10) {
                0..=3 => {
                    let value = format!("{key}@{step}");
                    devices[device].set(&key, &value);
                    model.insert(key, Some(value));
                }
                4 => {
                    let store = devices[device].store();
                    if store.delete(&store.key_id(&key)).unwrap() {
                        model.insert(key, None);
                    }
                }
                5 => {
                    let files: Vec<PathBuf> = files_under(&world.folder.join("ns"))
                        .into_iter()
                        .filter(|path| path.extension().is_some_and(|ext| ext == "rec"))
                        .collect();
                    if !files.is_empty() {
                        let victim = &files[rng.next(files.len() as u64) as usize];
                        let bytes = std::fs::read(victim).unwrap();
                        std::fs::write(victim, &bytes[..bytes.len() / 2]).unwrap();
                    }
                }
                6 => {
                    let dirs = files_under(&world.folder.join("ns"));
                    if let Some(file) = dirs.first() {
                        let copy = file
                            .with_file_name(format!("{} 2.rec", file.file_stem().unwrap().to_string_lossy()));
                        let _ = std::fs::copy(file, copy);
                    }
                }
                7 => world
                    .clock
                    .advance(Duration::from_secs(rng.next(4) * 12 * 24 * 3600)),
                _ => {
                    devices[device].sync();
                }
            }
        }
        for _ in 0..4 {
            for device in devices.iter_mut() {
                device.sync();
            }
        }
        // A sync tool's copy in a device's own directory makes it take a new ID; the rounds
        // above let it rejoin.
        for _ in 0..3 {
            for device in devices.iter_mut() {
                device.sync();
            }
        }
        for device in &devices {
            for (key, expected) in &model {
                assert_eq!(
                    device.get(key).as_ref(),
                    expected.as_ref(),
                    "seed {seed}: {key} on {}",
                    device.id()
                );
            }
        }
    }
}

/// Sums the counts in a leaving device's record into the folding device's own.
struct SumFold;

impl crate::Fold for SumFold {
    fn fold(&self, store: &Store, own: DeviceId, leaving: DeviceId) -> Result<(), dupin_storage::Error> {
        let count = |device: DeviceId| -> Result<u64, dupin_storage::Error> {
            Ok(store
                .get(&store.key_id(&device.to_string()))?
                .and_then(|record| record.data.as_u64())
                .unwrap_or(0))
        };
        let total = count(own)? + count(leaving)?;
        store.upsert(
            &own.to_string(),
            NewRecord {
                data: json!(total),
                blobs: Vec::new(),
            },
        )?;
        Ok(())
    }
}

fn per_device_sync(device: &mut Device) -> Report {
    let store = device.store.as_ref().unwrap();
    device
        .engine
        .run(
            &[Namespace {
                name: NAMESPACE,
                store,
                mode: Mode::PerDevice,
                fold: Some(&SumFold),
            }],
            &[],
        )
        .unwrap()
}

fn per_device_total(device: &Device) -> u64 {
    device
        .store()
        .list(None, 200)
        .unwrap()
        .records
        .iter()
        .filter_map(|record| record.data.as_u64())
        .sum()
}

#[test]
fn a_removed_macs_per_device_record_is_folded_once() {
    let world = World::new();
    let mut a = world.first("a");
    let mut b = world.joiner("b");
    let mut c = world.joiner("c");
    for (device, count) in [(&a, 1u64), (&b, 10), (&c, 100)] {
        device
            .store()
            .upsert(
                &device.id().to_string(),
                NewRecord {
                    data: json!(count),
                    blobs: Vec::new(),
                },
            )
            .unwrap();
    }
    for _ in 0..2 {
        for device in [&mut a, &mut b, &mut c] {
            per_device_sync(device);
        }
    }
    assert_eq!(per_device_total(&a), 111);
    let gone = c.id();
    a.engine.retire(gone).unwrap();
    for _ in 0..3 {
        for device in [&mut a, &mut b] {
            per_device_sync(device);
        }
    }
    assert!(!a.ns_dir(&world).join(gone.to_string()).exists());
    assert_eq!(
        per_device_total(&a),
        111,
        "the removed Mac's count is kept exactly once"
    );
    assert_eq!(per_device_total(&b), 111);
}
