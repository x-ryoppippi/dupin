//! The system UI font (San Francisco) as bytes, for Rune to draw text in.

use std::fs::File;
use std::path::PathBuf;

use memmap2::Mmap;
use objc2_core_foundation::{CFRetained, CFURL, CFURLPathStyle};
use objc2_core_text::{CTFont, CTFontUIFontType, kCTFontURLAttribute};

/// The system UI font's file, mapped for the life of the process, or `None` when CoreText
/// names no file or it can't be mapped.
///
/// Rune treats only the fonts an app hands it as covering text: a character outside them is
/// looked up through CoreText's fallback, which takes about a second cold, even when the
/// system font has it. Handing Rune the system font's bytes keeps that lookup for the
/// characters San Francisco lacks.
pub fn system_ui_font() -> Option<&'static [u8]> {
    let file = File::open(system_ui_font_path()?).ok()?;
    // SAFETY: A system font file under /System, which is read-only and never truncated while
    // mapped. The map is leaked, so the slice lives as long as the process.
    let map = unsafe { Mmap::map(&file) }.ok()?;
    Some(Box::leak(Box::new(map)))
}

fn system_ui_font_path() -> Option<PathBuf> {
    // SAFETY: A CoreText font made for its URL attribute only; `kCTFontURLAttribute` is
    // CoreText's constant key.
    let url = unsafe {
        let font = CTFont::new_ui_font_for_language(CTFontUIFontType::System, 0.0, None)?;
        font.attribute(kCTFontURLAttribute)?
    };
    let url: CFRetained<CFURL> = url.downcast().ok()?;
    let path = url.file_system_path(CFURLPathStyle::CFURLPOSIXPathStyle)?;
    Some(PathBuf::from(path.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_system_ui_font() {
        let font = system_ui_font().expect("CoreText names the system UI font's file");
        // An sfnt: TrueType outlines (0x00010000) or a collection ('ttcf').
        assert!(font.starts_with(&[0, 1, 0, 0]) || font.starts_with(b"ttcf"));
    }
}
