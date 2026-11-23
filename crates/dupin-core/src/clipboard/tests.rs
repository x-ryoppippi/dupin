use super::*;
use crate::config::Retention;

fn policy(max_entries: usize, max_entry_bytes: usize) -> Policy {
    Policy {
        max_entries,
        max_entry_bytes,
        max_image_bytes: 1024,
        include_images: true,
        include_files: true,
        retention: None,
        ignored_apps: vec!["com.example.vault".into()],
    }
}

fn text(value: &str) -> Content {
    Content::Text(value.into())
}

fn image(width: u32, height: u32, bytes: usize) -> Content {
    Content::Image(Image {
        png: vec![7; bytes],
        width,
        height,
    })
}

fn files(paths: &[&str]) -> Content {
    Content::Files(paths.iter().map(|path| FileRef::new(*path)).collect())
}

fn titles(history: &History) -> Vec<String> {
    history
        .entries()
        .map(|entry| entry.record.content.title())
        .collect()
}

fn push(history: &mut History, content: Content, now: u64) -> bool {
    history.push(content, None, now)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    Put(u64),
    Delete(u64),
    Clear,
    Expire(u64),
}

/// A store that keeps records in memory and logs what the history asked of it.
#[derive(Clone, Default)]
struct Recorder {
    records: Arc<Mutex<Vec<Arc<Record>>>>,
    ops: Arc<Mutex<Vec<Op>>>,
}

impl Recorder {
    fn ops(&self) -> Vec<Op> {
        std::mem::take(&mut self.ops.lock().unwrap())
    }

    fn ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .records
            .lock()
            .unwrap()
            .iter()
            .map(|record| record.id)
            .collect();
        ids.sort_unstable();
        ids
    }
}

impl Store for Recorder {
    fn list(&mut self) -> Result<Vec<Arc<Record>>, StoreError> {
        Ok(self.records.lock().unwrap().clone())
    }

    fn put(&mut self, record: &Arc<Record>) -> Result<(), StoreError> {
        self.ops.lock().unwrap().push(Op::Put(record.id));
        let mut records = self.records.lock().unwrap();
        records.retain(|kept| kept.id != record.id);
        records.push(record.clone());
        Ok(())
    }

    fn delete(&mut self, id: u64) -> Result<(), StoreError> {
        self.ops.lock().unwrap().push(Op::Delete(id));
        self.records.lock().unwrap().retain(|record| record.id != id);
        Ok(())
    }

    fn clear(&mut self) -> Result<(), StoreError> {
        self.ops.lock().unwrap().push(Op::Clear);
        self.records.lock().unwrap().clear();
        Ok(())
    }

    fn expire(&mut self, before: u64) -> Result<(), StoreError> {
        self.ops.lock().unwrap().push(Op::Expire(before));
        self.records
            .lock()
            .unwrap()
            .retain(|record| record.copied_at >= before);
        Ok(())
    }
}

struct Failing;

impl Store for Failing {
    fn list(&mut self) -> Result<Vec<Arc<Record>>, StoreError> {
        Err(StoreError("locked".into()))
    }
    fn put(&mut self, _: &Arc<Record>) -> Result<(), StoreError> {
        Err(StoreError("full".into()))
    }
    fn delete(&mut self, _: u64) -> Result<(), StoreError> {
        Err(StoreError("gone".into()))
    }
    fn clear(&mut self) -> Result<(), StoreError> {
        Err(StoreError("gone".into()))
    }
    fn expire(&mut self, _: u64) -> Result<(), StoreError> {
        Err(StoreError("gone".into()))
    }
}

#[test]
fn newest_first_with_duplicates_moved_to_the_front() {
    let mut history = History::new(policy(10, 1024));
    assert!(push(&mut history, text("alpha"), 1));
    assert!(push(&mut history, text("beta"), 2));
    assert!(push(&mut history, text("alpha"), 3));
    assert_eq!(titles(&history), ["alpha", "beta"]);
    let first = history.entries().next().unwrap();
    assert_eq!(first.record.copied_at, 3);
    let ids: Vec<_> = history.entries().map(|entry| entry.record.id).collect();
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn files_are_the_same_copy_by_path_even_when_their_size_changed() {
    let mut history = History::new(policy(10, 1024));
    let mut grown = FileRef::new("/fixture/movie.mov");
    grown.size = Some(2);
    push(&mut history, files(&["/fixture/movie.mov"]), 1);
    push(&mut history, Content::Files(vec![grown]), 2);
    assert_eq!(titles(&history), ["movie.mov"]);
}

#[test]
fn blank_oversized_and_excluded_kinds_are_skipped_not_truncated() {
    let mut history = History::new(policy(10, 8));
    assert!(!push(&mut history, text(" \n\t"), 1));
    assert!(!push(&mut history, text("123456789"), 1));
    assert!(!push(&mut history, Content::Files(Vec::new()), 1));
    assert!(!push(&mut history, image(1, 1, 0), 1));
    assert!(!push(&mut history, image(1, 1, 1025), 1));
    assert!(push(&mut history, text("12345678"), 1));
    assert!(push(&mut history, image(1, 1, 1024), 1));

    let mut no_extras = policy(10, 1024);
    no_extras.include_images = false;
    no_extras.include_files = false;
    let mut history = History::new(no_extras);
    assert!(!push(&mut history, image(1, 1, 8), 1));
    assert!(!push(&mut history, files(&["/fixture/a.txt"]), 1));
    assert!(push(&mut history, text("kept"), 1));
}

#[test]
fn count_limit_drops_the_oldest_and_new_limits_apply_at_once() {
    let mut history = History::new(policy(3, 1024));
    for (now, value) in ["one", "two", "three", "four"].into_iter().enumerate() {
        push(&mut history, text(value), now as u64);
    }
    assert_eq!(titles(&history), ["four", "three", "two"]);
    history.set_policy(policy(2, 4), 10);
    assert_eq!(titles(&history), ["four", "two"]);
    history.set_policy(policy(1, 1024), 10);
    assert_eq!(titles(&history), ["four"]);
}

#[test]
fn turning_off_images_or_files_forgets_those_already_kept() {
    let mut history = History::new(policy(10, 1024));
    push(&mut history, files(&["/fixture/a.txt"]), 1);
    push(&mut history, image(4, 3, 8), 2);
    push(&mut history, text("note"), 3);
    let mut next = policy(10, 1024);
    next.include_images = false;
    history.set_policy(next.clone(), 3);
    assert_eq!(titles(&history), ["note", "a.txt"]);
    next.include_files = false;
    history.set_policy(next, 3);
    assert_eq!(titles(&history), ["note"]);
}

#[test]
fn images_beyond_the_memory_budget_go_oldest_first() {
    let mut roomy = policy(1000, 1024);
    roomy.max_image_bytes = IMAGE_MEMORY / 3;
    let mut history = History::new(roomy);
    for now in 0..4 {
        push(
            &mut history,
            image(now as u32 + 1, 1, IMAGE_MEMORY / 3 - now),
            now as u64,
        );
    }
    push(&mut history, text("small"), 9);
    assert_eq!(
        titles(&history),
        ["small", "Image · 4×1 PNG", "Image · 3×1 PNG", "Image · 2×1 PNG"]
    );
}

#[test]
fn retention_expires_entries_in_memory_and_in_the_store() {
    let store = Recorder::default();
    let mut weekly = policy(10, 1024);
    weekly.retention = Retention::Week.seconds();
    let week = 7 * 86_400;
    let mut history = History::with_store(weekly, Box::new(store.clone()), |_| {}, 0);
    push(&mut history, text("old"), 10);
    push(&mut history, text("new"), 20);
    assert_eq!(store.ops(), [Op::Put(1), Op::Put(2)]);

    history.expire(week + 10);
    assert_eq!(titles(&history), ["new", "old"]);
    assert!(
        store.ops().is_empty(),
        "nothing expired, so the store is left alone"
    );

    history.expire(week + 11);
    assert_eq!(titles(&history), ["new"]);
    assert_eq!(store.ops(), [Op::Expire(11)]);
    assert_eq!(store.ids(), [2]);

    let mut forever = policy(10, 1024);
    forever.retention = Retention::Forever.seconds();
    history.set_policy(forever, u64::MAX);
    assert_eq!(titles(&history), ["new"]);
}

#[test]
fn a_store_is_loaded_newest_first_and_trimmed_to_the_policy() {
    let store = Recorder::default();
    {
        let mut history = History::with_store(policy(10, 1024), Box::new(store.clone()), |_| {}, 0);
        push(&mut history, text("first"), 100);
        push(&mut history, image(2, 2, 8), 200);
        push(&mut history, files(&["/fixture/a.txt"]), 300);
        push(&mut history, text("stale"), 1);
    }
    store.ops();

    let mut daily = policy(2, 1024);
    daily.retention = Retention::Day.seconds();
    let mut history = History::with_store(daily, Box::new(store.clone()), |_| {}, 86_400 + 50);
    assert_eq!(titles(&history), ["a.txt", "Image · 2×2 PNG"]);
    assert_eq!(store.ops(), [Op::Expire(50), Op::Delete(1)]);
    assert_eq!(store.ids(), [2, 3]);

    assert!(push(&mut history, text("next"), 86_400 + 60));
    assert_eq!(
        history.entries().next().unwrap().record.id,
        5,
        "IDs continue past the store's"
    );
}

#[test]
fn deleting_and_clearing_reach_the_store() {
    let store = Recorder::default();
    let mut history = History::with_store(policy(10, 1024), Box::new(store.clone()), |_| {}, 0);
    push(&mut history, text("keep"), 1);
    push(&mut history, text("drop"), 2);
    push(&mut history, text("keep"), 3);
    assert_eq!(store.ops(), [Op::Put(1), Op::Put(2), Op::Delete(1), Op::Put(3)]);

    assert_eq!(history.remove(2), Ok(true));
    assert_eq!(history.remove(2), Ok(false));
    assert_eq!(titles(&history), ["keep"]);
    assert_eq!(store.ids(), [3]);

    history.clear().unwrap();
    assert!(history.entries.is_empty());
    assert!(store.ids().is_empty());
    assert_eq!(store.ops(), [Op::Delete(2), Op::Delete(2), Op::Clear]);
}

#[test]
fn store_failures_keep_the_history_working_and_are_reported() {
    static REPORTED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut history = History::with_store(
        policy(1, 1024),
        Box::new(Failing),
        |error| REPORTED.lock().unwrap().push(error.0.clone()),
        0,
    );
    assert!(push(&mut history, text("one"), 1));
    assert!(push(&mut history, text("two"), 2));
    assert_eq!(titles(&history), ["two"]);
    assert_eq!(*REPORTED.lock().unwrap(), ["locked", "full", "full", "gone"]);
    assert!(history.remove(2).is_err());
    assert!(history.clear().is_err());
    assert!(
        history.entries.is_empty(),
        "memory forgets even when the store can't"
    );
}

#[test]
fn ignored_sources_are_refused() {
    let history = History::new(policy(10, 1024));
    assert!(history.accepts_source(Some("com.example.editor")));
    assert!(history.accepts_source(None));
    assert!(!history.accepts_source(Some("com.example.vault")));
    assert!(history.accepts_source(Some("com.example.vault.helper")));
}

#[test]
fn search_requires_every_word_ignoring_case_and_highlights_the_title() {
    let mut history = History::new(policy(10, 1024));
    push(&mut history, text("Invoice number 42\nDue Friday"), 1);
    push(&mut history, text("meeting notes"), 2);
    push(&mut history, files(&["/fixture/Reports/Invoice.pdf"]), 3);

    let hits = history.search("INVOICE due", 10);
    assert_eq!(hits.len(), 1);
    let (_, title, matched) = &hits[0];
    assert_eq!(title, "Invoice number 42 Due Friday");
    assert_eq!(matched, &[0, 1, 2, 3, 4, 5, 6, 18, 19, 20]);

    let hits = history.search("invoice", 10);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].1, "Invoice.pdf");

    let hits = history.search("reports", 10);
    assert_eq!(hits.len(), 1, "file entries match their whole path");
    assert!(hits[0].2.is_empty());

    assert_eq!(history.search("", 2).len(), 2);
    assert!(history.search("absent", 10).is_empty());
}

#[test]
fn images_match_by_kind_dimensions_size_and_source_app() {
    let mut history = History::new(policy(10, 1024));
    history.push(image(1280, 720, 1000), Some("Preview".into()), 1);
    history.push(text("an image of a cat"), None, 2);

    for query in [
        "image png",
        "1280×720",
        "1280x720",
        "1.0 kb",
        "preview",
        "IMAGE 720 PREVIEW",
    ] {
        let hits = history.search(query, 10);
        assert!(
            hits.iter().any(|(_, title, _)| title == "Image · 1280×720 PNG"),
            "{query}"
        );
    }
    let hits = history.search("image", 10);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[1].2, [0, 1, 2, 3, 4]);
    assert!(history.search("cat preview", 10).is_empty());
}

#[test]
fn search_matches_full_unicode_text_beyond_the_title() {
    let mut history = History::default();
    let original = format!("{} TÜRKİYE Café", "x".repeat(TITLE_CHARS + 50));
    assert!(push(&mut history, text(&original), 1));
    let hits = history.search("CAFÉ türki\u{307}ye", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0.record.content, text(&original));
    assert!(hits[0].2.is_empty());
}

#[test]
fn titles_collapse_whitespace_and_stay_short() {
    assert_eq!(text("  a\n\n b\tc  ").title(), "a b c");
    let long = "x".repeat(TITLE_CHARS + 50);
    let title = text(&long).title();
    assert_eq!(title.chars().count(), TITLE_CHARS + 1);
    assert!(title.ends_with('…'));
    assert_eq!(files(&["/a/one.txt", "/b/two.txt"]).title(), "one.txt, two.txt");
    assert_eq!(image(1280, 720, 1).title(), "Image · 1280×720 PNG");
}

#[test]
fn sizes_read_like_finder() {
    assert_eq!(size(0), "0 B");
    assert_eq!(size(999), "999 B");
    assert_eq!(size(1000), "1.0 KB");
    assert_eq!(size(12_345), "12 KB");
    assert_eq!(size(999_960), "1.0 MB");
    assert_eq!(size(1_260_000), "1.3 MB");
    assert_eq!(size(4_000_000_000), "4.0 GB");
}

fn provider(history: Arc<Mutex<History>>) -> ClipboardProvider {
    ClipboardProvider {
        history,
        access_item: |access| {
            (access != Access::Allowed).then(|| {
                Item::path(
                    "/fixture/clipboard-access".into(),
                    "Clipboard access".into(),
                    vec![],
                )
            })
        },
        clock: || 220,
        exists: |path| !path.starts_with("/fixture/gone"),
    }
}

fn search(provider: &ClipboardProvider, query: &str, limit: usize) -> Vec<Item> {
    let mut rows = Vec::new();
    provider
        .search(&Request::detached(query, limit), &mut |batch| rows.extend(batch))
        .unwrap();
    rows
}

fn action_ids(item: &Item) -> Vec<&str> {
    item.actions.iter().map(|action| action.id.as_str()).collect()
}

#[test]
fn provider_rows_restore_paste_delete_and_confirm_clearing_without_usage_history() {
    let history = Arc::new(Mutex::new(History::new(policy(10, 1024))));
    {
        let mut history = history.lock().unwrap();
        push(&mut history, text("first line\nsecond line"), 100);
        let mut movie = FileRef::new("/fixture/Movies/clip.mov");
        movie.size = Some(12_000_000);
        movie.kind = Some("com.apple.quicktime-movie".into());
        history.push(Content::Files(vec![movie]), None, 160);
        history.push(image(1280, 720, 900), Some("Preview".into()), 200);
    }
    let provider = provider(history.clone());
    let rows = search(&provider, "", 10);
    assert_eq!(rows.len(), 3);

    let picture = &rows[0];
    assert_eq!(
        picture.view,
        View::Text {
            title: "Image · 1280×720 PNG".into(),
            subtitle: "Just now · 900 B · Preview".into()
        }
    );
    assert_eq!(
        picture.action("copy"),
        Some(&Action::RestoreClipboardEntry {
            entry: 3,
            paste: false
        })
    );
    assert_eq!(
        picture.action("paste"),
        Some(&Action::RestoreClipboardEntry {
            entry: 3,
            paste: true
        })
    );
    assert_eq!(picture.alternate.as_deref(), Some("paste"));
    assert_eq!(action_ids(picture), ["copy", "paste", "delete", "clear"]);

    let file = &rows[1];
    assert_eq!(file.display_title(), "clip.mov");
    assert_eq!(
        file.view,
        View::Text {
            title: "clip.mov".into(),
            subtitle: "1 min ago · /fixture/Movies · 12 MB".into()
        }
    );
    assert_eq!(file.payload.paths, [PathBuf::from("/fixture/Movies/clip.mov")]);
    assert_eq!(
        file.action("reveal"),
        Some(&Action::Reveal("/fixture/Movies/clip.mov".into()))
    );
    assert_eq!(action_ids(file), ["copy", "paste", "reveal", "delete", "clear"]);

    let note = &rows[2];
    assert!(!note.record_usage);
    assert_eq!(note.primary.as_deref(), Some("copy"));
    assert_eq!(
        note.view,
        View::Text {
            title: "first line second line".into(),
            subtitle: "2 min ago · 2 lines".into()
        }
    );
    assert_eq!(
        note.action("copy"),
        Some(&Action::RestoreClipboardEntry {
            entry: 1,
            paste: false
        })
    );
    assert!(note.item_action("delete").unwrap().confirmation().is_none());
    assert!(note.item_action("clear").unwrap().confirmation().is_some());

    for access in [Access::Unasked, Access::Restricted] {
        history.lock().unwrap().set_access(access);
        let rows = search(&provider, "line", 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "/fixture/clipboard-access");
    }
}

#[test]
fn missing_files_show_as_unavailable_and_can_only_be_forgotten() {
    let history = Arc::new(Mutex::new(History::new(policy(10, 1024))));
    {
        let mut history = history.lock().unwrap();
        let mut gone = FileRef::new("/fixture/gone/old.mp4");
        gone.size = Some(5_000);
        history.push(Content::Files(vec![gone]), None, 220);
        push(
            &mut history,
            files(&["/fixture/gone/a.txt", "/fixture/b.txt"]),
            220,
        );
    }
    let rows = search(&provider(history), "", 10);

    let some = &rows[0];
    assert_eq!(
        some.view,
        View::Text {
            title: "a.txt, b.txt".into(),
            subtitle: "Just now · 2 files".into()
        }
    );
    assert_eq!(some.payload.paths, [PathBuf::from("/fixture/b.txt")]);
    assert_eq!(
        some.action("reveal"),
        Some(&Action::Reveal("/fixture/b.txt".into()))
    );

    let gone = &rows[1];
    assert_eq!(
        gone.view,
        View::Text {
            title: "old.mp4".into(),
            subtitle: "Just now · Unavailable · /fixture/gone".into()
        }
    );
    assert_eq!(gone.primary, None);
    assert_eq!(action_ids(gone), ["delete", "clear"]);
    assert!(gone.payload.is_empty());
}

#[test]
fn searching_expires_entries_past_retention() {
    let mut daily = policy(10, 1024);
    daily.retention = Retention::Day.seconds();
    let history = Arc::new(Mutex::new(History::new(daily)));
    push(&mut history.lock().unwrap(), text("ancient"), 0);
    push(&mut history.lock().unwrap(), text("recent"), 200);
    let mut provider = provider(history.clone());
    provider.clock = || 86_400 + 100;
    let rows = search(&provider, "", 10);
    assert_eq!(rows.len(), 1);
    assert_eq!(titles(&history.lock().unwrap()), ["recent"]);
}

#[test]
fn ages_read_naturally() {
    assert_eq!(ago(100, 100), "Just now");
    assert_eq!(ago(100, 200), "Just now");
    assert_eq!(ago(3599, 0), "59 min ago");
    assert_eq!(ago(7200, 0), "2 h ago");
    assert_eq!(ago(172_800, 0), "2 d ago");
}
