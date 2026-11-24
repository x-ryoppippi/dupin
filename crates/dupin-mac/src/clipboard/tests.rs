use std::cell::RefCell;
use std::collections::HashMap;

use dupin_core::clipboard::Record;
use objc2_core_foundation::{CFData, CFRetained};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
};

use super::*;

/// A pasteboard that holds what a test put on it and logs every content read.
#[derive(Default)]
struct Fixture {
    count: isize,
    /// What `change_count` reports after the first read, as when another copy lands meanwhile.
    recount: Option<isize>,
    access: Access,
    types: Vec<String>,
    strings: HashMap<String, String>,
    data: HashMap<String, Vec<u8>>,
    files: Vec<PathBuf>,
    reads: RefCell<Vec<String>>,
}

impl Fixture {
    fn new(types: &[&str]) -> Self {
        Self {
            count: 1,
            types: types.iter().map(|kind| (*kind).to_owned()).collect(),
            ..Self::default()
        }
    }

    fn text(mut self, kind: &str, text: &str) -> Self {
        self.strings.insert(kind.into(), text.into());
        self
    }

    fn bytes(mut self, kind: &str, data: Vec<u8>) -> Self {
        self.data.insert(kind.into(), data);
        self
    }

    fn reads(&self) -> Vec<String> {
        self.reads.borrow().clone()
    }
}

impl Board for Fixture {
    fn change_count(&self) -> isize {
        if self.reads.borrow().is_empty() {
            self.count
        } else {
            self.recount.unwrap_or(self.count)
        }
    }

    fn access(&self) -> Access {
        self.access
    }

    fn types(&self) -> Vec<String> {
        self.types.clone()
    }

    fn string(&self, kind: &str, max_bytes: usize) -> Option<String> {
        self.reads.borrow_mut().push(kind.into());
        self.strings
            .get(kind)
            .filter(|text| text.len() <= max_bytes)
            .cloned()
    }

    fn data(&self, kind: &str, max_bytes: usize) -> Option<Vec<u8>> {
        self.reads.borrow_mut().push(kind.into());
        self.data
            .get(kind)
            .filter(|data| data.len() <= max_bytes)
            .cloned()
    }

    fn file_urls(&self) -> Vec<PathBuf> {
        self.reads.borrow_mut().push(FILE_URL_TYPE.into());
        self.files.clone()
    }
}

fn app(id: &str) -> Option<App> {
    Some(App {
        id: id.into(),
        name: Some(format!("{id} app")),
    })
}

const EDITOR: &str = "com.example.editor";
const VAULT: &str = "com.apple.keychainaccess";

fn history(config: &str) -> Mutex<History> {
    let config = dupin_core::Config::parse(&format!("[clipboard]\nenabled = true\n{config}")).unwrap();
    Mutex::new(History::new((&config.clipboard).into()))
}

/// Run one capture of `board` with the editor frontmost, and return what was recorded.
fn capture_with(board: &Fixture, history: &Mutex<History>, apps: &[Option<App>]) -> Option<Arc<Record>> {
    capture(board, board.count, apps, history, &AtomicBool::new(true));
    history.lock().unwrap().record(1)
}

fn record(board: &Fixture, config: &str) -> Option<Arc<Record>> {
    capture_with(board, &history(config), &[app(EDITOR), app(EDITOR)])
}

fn pixels(width: usize, height: usize) -> CFRetained<CGImage> {
    let bytes: Vec<u8> = (0..width * height)
        .flat_map(|i| [(i % 251) as u8, (i % 13) as u8 * 19, 200, 255])
        .collect();
    let provider = CGDataProvider::with_cf_data(Some(&CFData::from_bytes(&bytes))).unwrap();
    // SAFETY: The buffer holds `height` rows of `width` RGBX pixels; no decode array.
    unsafe {
        CGImage::new(
            width,
            height,
            8,
            32,
            width * 4,
            CGColorSpace::new_device_rgb().as_deref(),
            CGBitmapInfo(CGImageAlphaInfo::NoneSkipLast.0),
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .unwrap()
}

fn encoded(width: usize, height: usize, kind: &str) -> Vec<u8> {
    image::encode(&pixels(width, height), kind).unwrap()
}

#[test]
fn concealed_transient_and_generated_copies_are_never_read() {
    let plain = STRING_TYPE.to_owned();
    assert!(!service::withheld(std::slice::from_ref(&plain)));
    for marker in service::WITHHELD_TYPES {
        for kinds in [
            [STRING_TYPE, marker],
            [FILE_URL_TYPE, marker],
            ["public.png", marker],
        ] {
            let board = Fixture::new(&kinds)
                .text(STRING_TYPE, "hunter2")
                .bytes("public.png", encoded(2, 2, "public.png"));
            assert!(record(&board, "").is_none(), "{marker}");
            assert!(board.reads().is_empty(), "{marker} {kinds:?}");
        }
    }
}

#[test]
fn copies_while_an_ignored_app_is_frontmost_are_never_read() {
    for apps in [[app(VAULT), app(EDITOR)], [app(EDITOR), app(VAULT)]] {
        let board = Fixture::new(&[STRING_TYPE, "public.png"]).text(STRING_TYPE, "secret");
        assert!(capture_with(&board, &history(""), &apps).is_none());
        assert!(board.reads().is_empty());
    }
}

#[test]
fn copies_naming_an_ignored_source_read_only_the_name() {
    let board = Fixture::new(&[STRING_TYPE, SOURCE_TYPE])
        .text(STRING_TYPE, "secret")
        .text(SOURCE_TYPE, VAULT);
    assert!(record(&board, "").is_none());
    assert_eq!(board.reads(), [SOURCE_TYPE]);
}

#[test]
fn ignored_apps_in_config_replace_the_defaults() {
    let board = Fixture::new(&[STRING_TYPE]).text(STRING_TYPE, "draft");
    let history = history("ignored_apps = [\"com.example.editor\"]");
    assert!(capture_with(&board, &history, &[app(EDITOR), app(EDITOR)]).is_none());
    assert!(board.reads().is_empty());
}

#[test]
fn nothing_is_read_without_background_access_or_once_stopped() {
    for access in [Access::Unasked, Access::Restricted] {
        let mut board = Fixture::new(&[STRING_TYPE]).text(STRING_TYPE, "note");
        board.access = access;
        assert!(record(&board, "").is_none());
        assert!(board.reads().is_empty());
    }

    let board = Fixture::new(&[STRING_TYPE]).text(STRING_TYPE, "note");
    let history = history("");
    capture(
        &board,
        board.count,
        &[app(EDITOR), app(EDITOR)],
        &history,
        &AtomicBool::new(false),
    );
    assert!(history.lock().unwrap().record(1).is_none());
    assert!(board.reads().is_empty());
}

#[test]
fn text_is_recorded_with_the_app_it_came_from() {
    let board = Fixture::new(&[STRING_TYPE, "public.rtf"]).text(STRING_TYPE, "hello");
    let record = record(&board, "").unwrap();
    assert_eq!(record.content, Content::Text("hello".into()));
    assert_eq!(record.source.as_deref(), Some("com.example.editor app"));
    assert_eq!(board.reads(), [STRING_TYPE]);

    let board = Fixture::new(&[STRING_TYPE, SOURCE_TYPE])
        .text(STRING_TYPE, "hello")
        .text(SOURCE_TYPE, "com.example.unknown");
    let record = self::record(&board, "").unwrap();
    assert_eq!(record.source.as_deref(), Some("com.example.unknown"));
}

#[test]
fn text_wins_over_an_image_of_it_and_oversized_text_is_skipped() {
    let board = Fixture::new(&[STRING_TYPE, "public.png"])
        .text(STRING_TYPE, "A1 cell")
        .bytes("public.png", encoded(2, 2, "public.png"));
    assert_eq!(
        record(&board, "").unwrap().content,
        Content::Text("A1 cell".into())
    );
    assert_eq!(board.reads(), [STRING_TYPE]);

    let board = Fixture::new(&[STRING_TYPE, "public.png"]).text(STRING_TYPE, &"x".repeat(1025));
    assert!(record(&board, "max_entry_bytes = 1024").is_none());
    assert_eq!(board.reads(), [STRING_TYPE], "the image is not read in its place");
}

#[test]
fn a_copy_replaced_while_it_was_read_is_left_for_the_next_poll() {
    let mut board = Fixture::new(&[STRING_TYPE]).text(STRING_TYPE, "stale");
    board.recount = Some(2);
    assert!(record(&board, "").is_none());
}

#[test]
fn files_are_kept_by_reference_with_their_size_and_type() {
    let dir = tempfile::tempdir().unwrap();
    let movie = dir.path().join("Holiday.mov");
    std::fs::write(&movie, vec![0; 2048]).unwrap();
    let folder = dir.path().join("Plain folder");
    std::fs::create_dir(&folder).unwrap();
    let missing = dir.path().join("gone.txt");
    let mut board =
        Fixture::new(&[FILE_URL_TYPE, STRING_TYPE, "public.tiff"]).text(STRING_TYPE, "Holiday.mov");
    board.files = vec![movie.clone(), folder.clone(), missing.clone()];

    let record = record(&board, "").unwrap();
    let Content::Files(files) = &record.content else {
        panic!("{:?}", record.content);
    };
    assert_eq!(
        files,
        &[
            FileRef {
                path: movie,
                size: Some(2048),
                kind: Some("com.apple.quicktime-movie".into()),
            },
            FileRef {
                path: folder,
                size: None,
                kind: Some("public.folder".into()),
            },
            FileRef {
                path: missing,
                size: None,
                kind: Some("public.plain-text".into()),
            },
        ]
    );
    assert_eq!(
        board.reads(),
        [FILE_URL_TYPE],
        "the names offered as text are not read"
    );
}

#[test]
fn only_files_still_there_are_put_back() {
    let files = [
        FileRef::new("/fixture/gone/a.txt"),
        FileRef::new("/fixture/b.txt"),
    ];
    let exists = |path: &Path| !path.starts_with("/fixture/gone");
    assert_eq!(
        restorable(&files, exists),
        Ok(vec![PathBuf::from("/fixture/b.txt")])
    );
    assert!(restorable(&files[..1], exists).is_err());
}

#[test]
fn excluded_files_are_neither_recorded_nor_read_as_their_names() {
    let mut board = Fixture::new(&[FILE_URL_TYPE, STRING_TYPE]).text(STRING_TYPE, "report.pdf");
    board.files = vec!["/fixture/report.pdf".into()];
    assert!(record(&board, "include_files = false").is_none());
    assert!(board.reads().is_empty());
}

#[test]
fn images_are_kept_as_png_from_every_pasteboard_type() {
    for kind in ["public.png", "public.tiff", "public.heic", "public.jpeg"] {
        let board = Fixture::new(&["com.example.private", kind]).bytes(kind, encoded(32, 20, kind));
        let record = record(&board, "").unwrap_or_else(|| panic!("{kind}"));
        let Content::Image(image) = &record.content else {
            panic!("{kind}: {:?}", record.content);
        };
        assert_eq!((image.width, image.height), (32, 20), "{kind}");
        assert!(image.png.starts_with(b"\x89PNG\r\n\x1a\n"), "{kind}");
        assert_eq!(board.reads(), [kind]);
    }

    let png = encoded(8, 8, "public.png");
    let board = Fixture::new(&["public.tiff", "public.png"])
        .bytes("public.png", png.clone())
        .bytes("public.tiff", encoded(8, 8, "public.tiff"));
    let record = record(&board, "").unwrap();
    assert_eq!(
        record.content,
        Content::Image(dupin_core::clipboard::Image {
            png,
            width: 8,
            height: 8
        }),
        "a PNG that fits is kept as copied"
    );
}

#[test]
fn excluded_images_are_never_read() {
    let board = Fixture::new(&["public.png"]).bytes("public.png", encoded(2, 2, "public.png"));
    assert!(record(&board, "include_images = false").is_none());
    assert!(board.reads().is_empty());
}

#[test]
fn large_images_are_scaled_down_and_ones_still_too_large_are_skipped() {
    let tiff = encoded(64, 16, "public.tiff");
    let image = image::to_png(&tiff, 16, usize::MAX).unwrap();
    assert_eq!((image.width, image.height), (16, 4));

    let png = encoded(64, 16, "public.png");
    let image = image::to_png(&png, 32, usize::MAX).unwrap();
    assert_eq!((image.width, image.height), (32, 8));
    assert_ne!(image.png, png);

    let fitting = image::to_png(&tiff, 64, usize::MAX).unwrap();
    assert!(image::to_png(&tiff, 64, fitting.png.len() - 1).is_none());

    assert!(image::to_png(b"not an image", 64, usize::MAX).is_none());
    assert!(image::to_png(&[], 64, usize::MAX).is_none());
}

#[test]
fn access_rows_offer_the_matching_macos_action() {
    assert!(access_item(Access::Allowed).is_none());
    assert_eq!(
        access_item(Access::Unasked).unwrap().action("open"),
        Some(&Action::RequestClipboardAccess),
    );
    assert_eq!(
        access_item(Access::Restricted).unwrap().action("open"),
        Some(&Action::OpenUrl(
            "x-apple.systempreferences:com.apple.preference.security".into(),
        )),
    );
}

#[test]
fn background_reads_require_always_allow_when_macos_reports_a_policy() {
    for (policy, expected, status, os) in [
        (None, Access::Allowed, privacy::Access::Allowed, OsAccess::Allowed),
        (
            Some(NSPasteboardAccessBehavior::AlwaysAllow),
            Access::Allowed,
            privacy::Access::Allowed,
            OsAccess::Allowed,
        ),
        (
            Some(NSPasteboardAccessBehavior::Default),
            Access::Unasked,
            privacy::Access::NotDetermined,
            OsAccess::NotDetermined,
        ),
        (
            Some(NSPasteboardAccessBehavior::Ask),
            Access::Restricted,
            privacy::Access::NotAllowed,
            OsAccess::Denied,
        ),
        (
            Some(NSPasteboardAccessBehavior::AlwaysDeny),
            Access::Restricted,
            privacy::Access::Denied,
            OsAccess::Denied,
        ),
    ] {
        assert_eq!(background_access(policy), expected);
        assert_eq!(policy_status(policy), status);
        assert_eq!(status.os_access(), os);
    }
}
