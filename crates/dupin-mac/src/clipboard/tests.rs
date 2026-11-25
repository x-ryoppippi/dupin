use std::cell::RefCell;
use std::collections::HashMap;

use objc2_core_foundation::{CFData, CFRetained};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
};

use super::*;

/// A pasteboard that holds what a test put on it and logs every content read.
#[derive(Default)]
struct Fixture {
    types: Vec<String>,
    strings: HashMap<String, String>,
    data: HashMap<String, Vec<u8>>,
    files: Vec<PathBuf>,
    reads: RefCell<Vec<String>>,
}

impl Fixture {
    fn new(types: &[&str]) -> Self {
        Self {
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

const LIMITS: Limits = Limits {
    text_bytes: 1024,
    image_bytes: 4 * 1024 * 1024,
    files: 64,
};

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
fn text_wins_over_an_image_of_it_and_oversized_text_is_skipped() {
    let board = Fixture::new(&[STRING_TYPE, "public.png"])
        .text(STRING_TYPE, "A1 cell")
        .bytes("public.png", encoded(2, 2, "public.png"));
    assert_eq!(
        read_content(&board, &LIMITS),
        Some(service::Content::Text {
            text: "A1 cell".into()
        })
    );
    assert_eq!(board.reads(), [STRING_TYPE]);

    let board = Fixture::new(&[STRING_TYPE, "public.png"]).text(STRING_TYPE, &"x".repeat(1025));
    assert_eq!(read_content(&board, &LIMITS), None);
    assert_eq!(board.reads(), [STRING_TYPE], "the image is not read in its place");
}

#[test]
fn a_link_is_its_url() {
    let board = Fixture::new(&[URL_TYPE, STRING_TYPE])
        .text(URL_TYPE, "https://dupin.app/")
        .text(STRING_TYPE, "Dupin");
    assert_eq!(
        read_content(&board, &LIMITS),
        Some(service::Content::Url {
            url: "https://dupin.app/".into()
        })
    );
}

#[test]
fn files_are_described_by_reference_never_read_as_their_names() {
    let dir = tempfile::tempdir().unwrap();
    let movie = dir.path().join("Holiday.mov");
    std::fs::write(&movie, vec![0; 2048]).unwrap();
    let folder = dir.path().join("Plain folder");
    std::fs::create_dir(&folder).unwrap();
    let missing = dir.path().join("gone.txt");
    let mut board =
        Fixture::new(&[FILE_URL_TYPE, STRING_TYPE, "public.tiff"]).text(STRING_TYPE, "Holiday.mov");
    board.files = vec![movie.clone(), folder.clone(), missing.clone()];

    let Some(service::Content::Files { files }) = read_content(&board, &LIMITS) else {
        panic!("files are read as files");
    };
    let described: Vec<_> = files
        .iter()
        .map(|file| (file.name.as_str(), file.size, file.kind.as_deref()))
        .collect();
    assert_eq!(
        described,
        [
            ("Holiday.mov", Some(2048), Some("com.apple.quicktime-movie")),
            ("Plain folder", None, Some("public.folder")),
            ("gone.txt", None, Some("public.plain-text")),
        ]
    );
    assert_eq!(
        files[0].folder.as_deref(),
        Some(dupin_core::item::tilde(&dir.path().to_string_lossy()).as_str())
    );
    assert_eq!(
        board.reads(),
        [FILE_URL_TYPE],
        "the names offered as text are not read"
    );
}

#[test]
fn images_are_kept_as_png_from_every_pasteboard_type() {
    for kind in ["public.png", "public.tiff", "public.heic", "public.jpeg"] {
        let board = Fixture::new(&["com.example.private", kind]).bytes(kind, encoded(32, 20, kind));
        let Some(service::Content::Image {
            width, height, data, ..
        }) = read_content(&board, &LIMITS)
        else {
            panic!("{kind}");
        };
        assert_eq!((width, height), (32, 20), "{kind}");
        assert!(data.starts_with(b"\x89PNG\r\n\x1a\n"), "{kind}");
        assert_eq!(board.reads(), [kind]);
    }

    let png = encoded(8, 8, "public.png");
    let board = Fixture::new(&["public.tiff", "public.png"])
        .bytes("public.png", png.clone())
        .bytes("public.tiff", encoded(8, 8, "public.tiff"));
    assert_eq!(
        read_content(&board, &LIMITS),
        Some(service::Content::Image {
            media_type: "image/png".into(),
            width: 8,
            height: 8,
            data: png,
        }),
        "a PNG that fits is kept as copied"
    );
}

#[test]
fn a_copy_offered_as_an_image_type_is_decoded_only_when_it_is_that_kind_of_image() {
    let gif = encoded(8, 8, "com.compuserve.gif");
    let board = Fixture::new(&["public.png"]).bytes("public.png", gif);
    assert_eq!(read_content(&board, &LIMITS), None);
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
