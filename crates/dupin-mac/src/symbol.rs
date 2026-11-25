//! SF Symbols drawn to pixels for icons, in one colour. Call it off the panel's thread: AppKit
//! loads the symbol's font the first time one is drawn.

use std::ffi::c_void;

use objc2::rc::autoreleasepool;

use objc2_app_kit::{NSFontWeightRegular, NSImage, NSImageSymbolConfiguration};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGInterpolationQuality,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

/// The largest square drawn, as large as Rune's own icons.
const MAX_PIXELS: u32 = 1024;

/// The symbol `name` drawn in `ink` (straight-alpha sRGB), as large as fits `glyph` pixels square
/// keeping its shape, centred on a transparent square of `canvas` pixels: `canvas * canvas`
/// straight-alpha sRGB RGBA pixels, rows top first. `None` when the system has no symbol of
/// that name or the sizes are out of range.
pub fn rgba(name: &str, canvas: u32, glyph: u32, ink: [u8; 4]) -> Option<Vec<u8>> {
    // The worker that calls this lives as long as Dupin, and no pool of AppKit's drains on it.
    autoreleasepool(|_| draw(name, canvas, glyph, ink))
}

fn draw(name: &str, canvas: u32, glyph: u32, ink: [u8; 4]) -> Option<Vec<u8>> {
    if !dupin_core::item::is_symbol_name(name)
        || canvas == 0
        || canvas > MAX_PIXELS
        || glyph == 0
        || glyph > canvas
    {
        return None;
    }
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?;
    // SAFETY: NSFontWeightRegular is an AppKit constant, read only.
    let weight = unsafe { NSFontWeightRegular };
    let configuration =
        NSImageSymbolConfiguration::configurationWithPointSize_weight(f64::from(glyph), weight);
    let image = image.imageWithSymbolConfiguration(&configuration)?;
    let NSSize { width, height } = image.size();
    if !(width > 0.0 && height > 0.0) {
        return None;
    }
    let scale = (f64::from(glyph) / width).min(f64::from(glyph) / height);
    let (width, height) = (
        (width * scale).round().max(1.0),
        (height * scale).round().max(1.0),
    );
    let mut proposed = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));
    // SAFETY: `proposed` outlives the call; no context or hints.
    let drawn = unsafe { image.CGImageForProposedRect_context_hints(&mut proposed, None, None) }?;

    let side = canvas as usize;
    let mut pixels = vec![0u8; side * side * 4];
    let space = CGColorSpace::new_device_rgb()?;
    // SAFETY: `pixels` holds `side` rows of `side * 4` bytes and outlives the context, which is
    // dropped below before the buffer is read.
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast::<c_void>(),
            side,
            side,
            8,
            side * 4,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }?;
    CGContext::set_interpolation_quality(Some(&context), CGInterpolationQuality::High);
    let origin = |length: f64| ((f64::from(canvas) - length) / 2.0).round();
    let rect = NSRect::new(
        NSPoint::new(origin(width), origin(height)),
        NSSize::new(width, height),
    );
    CGContext::draw_image(Some(&context), rect, Some(&drawn));
    drop(context);
    Some(tint(&pixels, ink))
}

/// `drawn`'s coverage, its alpha, in `ink`: a symbol is drawn as a template, so its colour says
/// nothing and its alpha says where it is.
fn tint(drawn: &[u8], [r, g, b, a]: [u8; 4]) -> Vec<u8> {
    drawn
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| {
            let alpha = (u16::from(pixel[3]) * u16::from(a) + 127) / 255;
            [r, g, b, alpha as u8]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage(pixels: &[u8]) -> usize {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] > 0)
            .count()
    }

    #[test]
    fn a_symbol_draws_in_its_ink_inside_the_glyph_box() {
        let ink = [10, 20, 30, 255];
        let pixels = rgba("moon.fill", 64, 40, ink).expect("macOS has moon.fill");
        assert_eq!(pixels.len(), 64 * 64 * 4);
        assert!(coverage(&pixels) > 100);
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[..3] == ink[..3] || pixel[3] == 0)
        );
        let row = |y: usize| &pixels[y * 64 * 4..(y + 1) * 64 * 4];
        assert_eq!(coverage(row(0)), 0, "the margin above the glyph box is clear");
        assert_eq!(coverage(row(63)), 0, "the margin below the glyph box is clear");
    }

    #[test]
    fn half_transparent_ink_halves_the_coverage() {
        let solid = rgba("lock.fill", 32, 32, [0, 0, 0, 255]).unwrap();
        let faint = rgba("lock.fill", 32, 32, [0, 0, 0, 128]).unwrap();
        let most = |pixels: &[u8]| {
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .map(|pixel| pixel[3])
                .max()
                .unwrap()
        };
        assert_eq!(most(&solid), 255);
        assert_eq!(most(&faint), 128);
    }

    #[test]
    fn unknown_or_malformed_names_and_bad_sizes_draw_nothing() {
        assert!(rgba("no.such.symbol.anywhere", 32, 32, [0; 4]).is_none());
        assert!(rgba("../moon", 32, 32, [0; 4]).is_none());
        assert!(rgba("moon", 0, 0, [0; 4]).is_none());
        assert!(rgba("moon", 16, 32, [0; 4]).is_none());
        assert!(rgba("moon", MAX_PIXELS + 1, 32, [0; 4]).is_none());
    }
}
