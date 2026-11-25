//! SF Symbols, and the icons drawn outside the result list: a command's chip and Settings'
//! list of extensions. A symbol is drawn once on a worker thread in the colour of the text
//! beside it, and kept by name, size and colour, so a frame only looks it up; a theme or a
//! selected row draws it once more in its new colour. Its pixels wake the windows showing it
//! when they land.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError, mpsc};

use dupin_core::Icon;
use rune::{Color, Image, ImageCache, Pixels};

/// Device pixels a point, as the panel's other icons are drawn at.
const SCALE: f32 = 2.0;
/// Drawings kept before the oldest are let go; a frame asking again draws them anew.
const MAX_KEPT: usize = 256;

/// How large a symbol is drawn and the square it is centred on, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub square: f32,
    pub glyph: f32,
}

/// A result row's: its symbol sits as large as an app icon's artwork does in its square.
pub const ROW: Size = Size {
    square: crate::ui::ICON_SIZE,
    glyph: 20.0,
};
/// A command's chip and an extension's heading in Settings.
pub const INLINE: Size = Size {
    square: 16.0,
    glyph: 14.0,
};

struct Job {
    name: String,
    canvas: u32,
    glyph: u32,
    ink: [u8; 4],
    pixels: Pixels,
}

type Drawing = (u32, u32, [u8; 4]);

#[derive(Default)]
struct Kept {
    symbols: HashMap<String, Vec<(Drawing, Pixels)>>,
    count: usize,
    images: HashMap<(PathBuf, u32), Image>,
}

static KEPT: LazyLock<Mutex<Kept>> = LazyLock::new(Mutex::default);

static WORKER: LazyLock<mpsc::Sender<Job>> = LazyLock::new(|| {
    let (send, receive) = mpsc::channel::<Job>();
    std::thread::Builder::new()
        .name("dupin-symbols".into())
        .spawn(move || {
            for job in receive {
                if let Some(rgba) = dupin_mac::symbol::rgba(&job.name, job.canvas, job.glyph, job.ink) {
                    job.pixels.set(job.canvas, job.canvas, rgba);
                }
            }
        })
        .expect("spawn the symbol worker");
    send
});

/// Decodes the image files icons name. Its pixels wake the windows showing them, so it needs
/// no waker of its own.
static IMAGES: LazyLock<ImageCache> = LazyLock::new(|| ImageCache::new(std::sync::Arc::new(|| {})));

fn pixels(points: f32) -> u32 {
    (points * SCALE).round().max(1.0) as u32
}

/// Straight-alpha sRGB, as the drawing is.
fn srgb(color: Color) -> [u8; 4] {
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let encode = rune::color::srgb_encode;
    [
        byte(encode(color.r)),
        byte(encode(color.g)),
        byte(encode(color.b)),
        byte(color.a),
    ]
}

/// The symbol `name` in `ink`, empty until it is drawn and for good when the system has no
/// symbol of that name.
pub fn symbol(name: &str, size: Size, ink: Color) -> Pixels {
    let drawing = (
        pixels(size.square),
        pixels(size.glyph).min(pixels(size.square)),
        srgb(ink),
    );
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, pixels)) = kept
        .symbols
        .get(name)
        .and_then(|drawings| drawings.iter().find(|(kept, _)| *kept == drawing))
    {
        return pixels.clone();
    }
    if kept.count >= MAX_KEPT {
        kept.symbols.clear();
        kept.count = 0;
    }
    let pixels = Pixels::new();
    kept.symbols
        .entry(name.to_owned())
        .or_default()
        .push((drawing, pixels.clone()));
    kept.count += 1;
    drop(kept);
    let (canvas, glyph, ink) = drawing;
    let _ = WORKER.send(Job {
        name: name.to_owned(),
        canvas,
        glyph,
        ink,
        pixels: pixels.clone(),
    });
    pixels
}

/// The image file at `path`, fitting `square` points.
fn image(path: &Path, square: f32) -> Pixels {
    let key = (path.to_path_buf(), pixels(square));
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    if kept.images.len() >= MAX_KEPT && !kept.images.contains_key(&key) {
        kept.images.clear();
    }
    kept.images
        .entry(key)
        .or_insert_with(|| IMAGES.file(path, square, SCALE))
        .pixels()
        .clone()
}

/// What `icon` draws at `size` beside text in `ink`, where the result list's own icon fetch
/// doesn't reach: a symbol, or an image file. A Finder icon or a stored image has no picture
/// here.
pub fn icon(icon: &Icon, size: Size, ink: Color) -> Option<Pixels> {
    match icon {
        Icon::Symbol(name) => Some(symbol(name, size, ink)),
        Icon::Image(path) => Some(image(path, size.square)),
        Icon::File(_) | Icon::Host(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drawing_is_kept_by_name_size_and_colour() {
        let ink = Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let id = |pixels: Pixels| pixels.signature().0;
        let first = id(symbol("moon.fill", ROW, ink));
        assert_eq!(id(symbol("moon.fill", ROW, ink)), first);
        assert_ne!(id(symbol("moon.fill", ROW, ink.alpha(0.5))), first);
        assert_ne!(id(symbol("moon.fill", INLINE, ink)), first);
        assert_ne!(id(symbol("sun.max", ROW, ink)), first);
        assert_eq!(srgb(ink), [255, 0, 0, 255]);
    }
}
