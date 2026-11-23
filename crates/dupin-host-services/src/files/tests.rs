use super::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let root = base.join("root");
    let outside = base.join("outside");
    fs::create_dir_all(root.join("notes/deep")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(root.join("notes/a.txt"), "inside").unwrap();
    fs::write(root.join("notes/deep/b.txt"), "deeper").unwrap();
    fs::write(outside.join("secret.txt"), "SECRET").unwrap();
    Fixture {
        _temp: temp,
        root,
        outside,
    }
}

fn files(fixture: &Fixture) -> Files {
    let mut files = Files::new();
    files.grant(Scope::pick("notes", ScopeKind::Directory, &fixture.root).unwrap());
    files
}

fn path<'a>(path: &'a str) -> Target<'a> {
    Target::Path { scope: "notes", path }
}

#[test]
fn reads_and_lists_inside_the_scope_and_hands_back_validated_handles() {
    let fixture = fixture();
    let mut files = files(&fixture);
    let listing = files.list(path("notes"), DEFAULT_LIST_ENTRIES).unwrap();
    assert!(!listing.truncated);
    let names: Vec<_> = listing
        .entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.kind))
        .collect();
    assert_eq!(
        names,
        [("a.txt", EntryKind::File), ("deep", EntryKind::Directory)]
    );
    assert_eq!(listing.entries[0].size, Some(6));

    let file = listing.entries[0].handle.unwrap();
    let contents = files.read(Target::Handle(file), DEFAULT_READ_BYTES).unwrap();
    assert_eq!(contents.bytes, b"inside");
    assert_eq!(contents.handle, file);
    assert_eq!(Handle::parse(&file.to_string()), Some(file));
    assert_eq!(files.read(path("./notes//a.txt"), 64).unwrap().handle, file);

    let deep = listing.entries[1].handle.unwrap();
    let nested = files.list(Target::Handle(deep), 10).unwrap();
    assert_eq!(nested.handle, deep);
    assert_eq!(nested.entries[0].name, "b.txt");

    let opened = files.open(file).unwrap();
    assert_eq!(opened.path, fixture.root.join("notes/a.txt"));
    assert_eq!(opened.kind, EntryKind::File);

    let root = files.list(path(""), 10).unwrap();
    assert_eq!(root.entries.len(), 1);
}

#[test]
fn paths_that_leave_the_scope_are_refused() {
    let fixture = fixture();
    symlink(&fixture.outside, fixture.root.join("notes/link-dir")).unwrap();
    symlink(
        fixture.outside.join("secret.txt"),
        fixture.root.join("notes/link-file"),
    )
    .unwrap();
    symlink("../../outside/secret.txt", fixture.root.join("relative-link")).unwrap();
    let mut files = files(&fixture);

    for (target, expected) in [
        ("../outside/secret.txt", "cannot contain .."),
        ("notes/../../outside/secret.txt", "cannot contain .."),
        ("notes/..", "cannot contain .."),
        ("/etc/passwd", "must be relative"),
        (
            fixture.outside.join("secret.txt").to_str().unwrap(),
            "must be relative",
        ),
        ("notes/a.txt\0", "contains NUL"),
    ] {
        let error = files.read(path(target), 64).unwrap_err().to_string();
        assert!(error.contains(expected), "{target:?}: {error}");
    }
    for target in ["notes/link-file", "notes/link-dir/secret.txt", "relative-link"] {
        assert!(
            matches!(files.read(path(target), 64), Err(FileError::Symlink)),
            "{target}"
        );
    }
    assert!(matches!(
        files.list(path("notes/link-dir"), 10),
        Err(FileError::Symlink)
    ));
    assert!(matches!(
        files.read(path(&"a/".repeat(65)), 64),
        Err(FileError::InvalidPath(_))
    ));
    assert!(matches!(
        files.read(path(&"x".repeat(256)), 64),
        Err(FileError::InvalidPath(_))
    ));
    assert!(matches!(
        files.read(
            Target::Path {
                scope: "other",
                path: ""
            },
            64
        ),
        Err(FileError::UnknownScope(_))
    ));

    let listing = files.list(path("notes"), 10).unwrap();
    let links: Vec<_> = listing
        .entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Symlink)
        .collect();
    assert_eq!(links.len(), 2);
    assert!(
        links
            .iter()
            .all(|entry| entry.handle.is_none() && entry.size.is_none())
    );
}

#[test]
fn a_root_replaced_by_a_symlink_or_another_folder_is_no_longer_the_scope() {
    let fixture = fixture();
    let mut files = files(&fixture);
    let moved = fixture.root.with_file_name("moved");
    fs::rename(&fixture.root, &moved).unwrap();
    symlink(&fixture.outside, &fixture.root).unwrap();
    assert!(matches!(files.list(path(""), 10), Err(FileError::ScopeChanged)));
    fs::remove_file(&fixture.root).unwrap();
    fs::create_dir(&fixture.root).unwrap();
    assert!(matches!(files.list(path(""), 10), Err(FileError::ScopeChanged)));
    fs::remove_dir(&fixture.root).unwrap();
    fs::rename(&moved, &fixture.root).unwrap();
    assert!(files.list(path(""), 10).is_ok());
}

#[test]
fn reads_and_listings_are_bounded() {
    let fixture = fixture();
    fs::write(fixture.root.join("large"), vec![b'x'; MAX_READ_BYTES + 10]).unwrap();
    for index in 0..MAX_LIST_ENTRIES + 5 {
        fs::write(fixture.root.join(format!("notes/deep/{index:05}")), "").unwrap();
    }
    let mut files = files(&fixture);

    let small = files.read(path("large"), 10).unwrap();
    assert_eq!(small.bytes.len(), 10);
    assert!(small.truncated);
    assert_eq!(small.size, (MAX_READ_BYTES + 10) as u64);
    let capped = files.read(path("large"), usize::MAX).unwrap();
    assert_eq!(capped.bytes.len(), MAX_READ_BYTES);
    assert!(capped.truncated);
    let exact = files.read(path("notes/a.txt"), 6).unwrap();
    assert!(!exact.truncated);
    assert_eq!(files.read(path("notes/a.txt"), 0).unwrap().bytes, b"");

    let listing = files.list(path("notes/deep"), 3).unwrap();
    assert_eq!(listing.entries.len(), 3);
    assert!(listing.truncated);
    let capped = files.list(path("notes/deep"), usize::MAX).unwrap();
    assert_eq!(capped.entries.len(), MAX_LIST_ENTRIES);
    assert!(capped.truncated);
}

#[test]
fn special_files_are_refused_without_blocking() {
    let fixture = fixture();
    let fifo = CString::new(fixture.root.join("pipe").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let mut files = files(&fixture);
    assert!(matches!(files.read(path("pipe"), 64), Err(FileError::NotAFile)));
    assert!(matches!(files.read(path("notes"), 64), Err(FileError::NotAFile)));
    assert!(matches!(
        files.list(path("notes/a.txt"), 64),
        Err(FileError::NotADirectory)
    ));
    let listing = files.list(path(""), 10).unwrap();
    let pipe = listing.entries.iter().find(|entry| entry.name == "pipe").unwrap();
    assert_eq!(pipe.kind, EntryKind::Other);
    assert!(pipe.handle.is_none());
}

#[test]
fn a_directory_moved_out_after_it_was_opened_is_caught_before_reading() {
    let fixture = fixture();
    let files = files(&fixture);
    let scope = files.scopes().next().unwrap();
    let names = components("notes/deep/b.txt").unwrap();
    let walk = scope.walk(&names, false).unwrap();
    assert!(walk.verify(&names).is_ok());
    fs::rename(fixture.root.join("notes/deep"), fixture.outside.join("deep")).unwrap();
    assert!(matches!(walk.verify(&names), Err(FileError::Escaped)));
}

#[test]
fn racing_symlink_swaps_never_expose_files_outside_the_scope() {
    let fixture = fixture();
    let notes = fixture.root.join("notes");
    let parked = fixture.root.join("parked");
    fs::write(fixture.outside.join("a.txt"), "SECRET").unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let swapper = {
        let stop = stop.clone();
        let outside = fixture.outside.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                fs::rename(&notes, &parked).unwrap();
                symlink(&outside, &notes).unwrap();
                fs::remove_file(&notes).unwrap();
                fs::rename(&parked, &notes).unwrap();
            }
        })
    };
    let mut files = files(&fixture);
    let mut inside = 0;
    for _ in 0..2000 {
        match files.read(path("notes/a.txt"), 64) {
            Ok(contents) => {
                assert_eq!(contents.bytes, b"inside");
                inside += 1;
            }
            Err(FileError::Symlink | FileError::NotADirectory | FileError::NotFound | FileError::Escaped) => {
            }
            Err(error) => panic!("unexpected error: {error}"),
        }
    }
    stop.store(true, Ordering::Relaxed);
    swapper.join().unwrap();
    assert!(inside > 0);
}

#[test]
fn handles_belong_to_one_table_and_one_identity() {
    let fixture = fixture();
    let mut files = files(&fixture);
    let handle = files.read(path("notes/a.txt"), 64).unwrap().handle;

    let mut other = self::files(&fixture);
    assert!(matches!(
        other.read(Target::Handle(handle), 64),
        Err(FileError::UnknownHandle)
    ));
    assert!(Handle::parse("nope").is_none());
    assert!(Handle::parse("1.").is_none());
    assert!(Handle::parse("+1.2").is_none());

    fs::write(fixture.root.join("notes/replacement"), "replaced").unwrap();
    fs::rename(
        fixture.root.join("notes/replacement"),
        fixture.root.join("notes/a.txt"),
    )
    .unwrap();
    assert!(matches!(
        files.read(Target::Handle(handle), 64),
        Err(FileError::StaleHandle)
    ));
    assert!(matches!(files.open(handle), Err(FileError::StaleHandle)));
    let fresh = files.read(path("notes/a.txt"), 64).unwrap();
    assert_eq!(fresh.bytes, b"replaced");
    assert_ne!(fresh.handle, handle);

    files.revoke("notes");
    assert!(matches!(
        files.read(Target::Handle(fresh.handle), 64),
        Err(FileError::UnknownHandle)
    ));
    files.grant(Scope::pick("notes", ScopeKind::Directory, &fixture.root).unwrap());
    assert!(matches!(
        files.read(Target::Handle(fresh.handle), 64),
        Err(FileError::UnknownHandle)
    ));
}

#[test]
fn the_handle_table_is_bounded() {
    let fixture = fixture();
    for index in 0..MAX_HANDLES {
        fs::write(fixture.root.join(format!("notes/deep/{index:05}")), "").unwrap();
    }
    let mut files = files(&fixture);
    let first = files.read(path("notes/a.txt"), 0).unwrap().handle;
    for start in (0..MAX_HANDLES).step_by(MAX_LIST_ENTRIES) {
        for index in start..(start + MAX_LIST_ENTRIES).min(MAX_HANDLES) {
            files.read(path(&format!("notes/deep/{index:05}")), 0).unwrap();
        }
    }
    assert!(files.records.len() <= MAX_HANDLES);
    assert_eq!(files.records.len(), files.index.len());
    assert!(matches!(
        files.read(Target::Handle(first), 0),
        Err(FileError::UnknownHandle)
    ));
}

#[test]
fn a_file_scope_reads_only_its_file_and_survives_atomic_saves() {
    let fixture = fixture();
    let file = fixture.root.join("notes/a.txt");
    let mut files = Files::new();
    let scope = Scope::pick("todo", ScopeKind::File, &file).unwrap();
    assert!(scope.describe().starts_with("Read the file "));
    files.grant(scope);
    let target = || Target::Path {
        scope: "todo",
        path: "",
    };
    assert_eq!(files.read(target(), 64).unwrap().bytes, b"inside");
    assert!(matches!(
        files.read(
            Target::Path {
                scope: "todo",
                path: "deep/b.txt"
            },
            64
        ),
        Err(FileError::NotADirectory)
    ));
    assert!(matches!(files.list(target(), 10), Err(FileError::NotADirectory)));

    fs::write(fixture.root.join("notes/a.tmp"), "saved").unwrap();
    fs::rename(fixture.root.join("notes/a.tmp"), &file).unwrap();
    assert_eq!(files.read(target(), 64).unwrap().bytes, b"saved");

    fs::remove_file(&file).unwrap();
    symlink(fixture.outside.join("secret.txt"), &file).unwrap();
    assert!(matches!(files.read(target(), 64), Err(FileError::Symlink)));

    assert!(matches!(
        Scope::pick("todo", ScopeKind::File, &fixture.root),
        Err(FileError::NotAFile)
    ));
    assert!(matches!(
        Scope::pick("notes", ScopeKind::Directory, &fixture.outside.join("secret.txt")),
        Err(FileError::NotADirectory)
    ));
    for name in ["", "Upper", "a/b", "dot.name", &"x".repeat(65)] {
        assert!(matches!(
            Scope::pick(name, ScopeKind::Directory, &fixture.root),
            Err(FileError::InvalidScopeName)
        ));
    }
}
