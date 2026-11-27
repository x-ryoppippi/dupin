//! SF Symbols, and the icons drawn outside the result list: a command's chip and Settings'
//! list of extensions. A symbol is drawn once on a worker thread in the colour of the text
//! beside it, and kept by name, size and colour, so a frame only looks it up; a theme or a
//! selected row draws it once more in its new colour. Its pixels wake the windows showing it
//! when they land, and so does the worker's finding that the system has no symbol of a name.

use std::collections::{HashMap, HashSet};
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
/// The query field's search icon: as large as the glyph it stands in for.
pub const FIELD: Size = Size {
    square: 24.0,
    glyph: 20.0,
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
    /// Names the system has no symbol for. Kept apart from the drawings, so letting those go
    /// doesn't show an empty square again while the worker finds the name missing anew.
    missing: HashSet<String>,
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
                } else if !dupin_mac::symbol::exists(&job.name) {
                    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
                    if kept.missing.len() >= MAX_KEPT {
                        // Their kept drawings are clear squares: let them go too, so a frame
                        // asking again finds the name missing anew rather than drawing a gap.
                        let Kept {
                            symbols,
                            count,
                            missing,
                            ..
                        } = &mut *kept;
                        for name in missing.drain() {
                            *count -= symbols.remove(&name).map_or(0, |drawings| drawings.len());
                        }
                    }
                    kept.missing.insert(job.name);
                    drop(kept);
                    // A clear square wakes the windows showing it, which then ask again and
                    // learn the name is missing.
                    let side = job.canvas as usize;
                    job.pixels.set(job.canvas, job.canvas, vec![0; side * side * 4]);
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

/// The symbol `name` in `ink`, empty until it is drawn, or `None` once the drawing found the
/// system has no symbol of that name.
pub fn symbol(name: &str, size: Size, ink: Color) -> Option<Pixels> {
    let drawing = (
        pixels(size.square),
        pixels(size.glyph).min(pixels(size.square)),
        srgb(ink),
    );
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    if kept.missing.contains(name) {
        return None;
    }
    if let Some((_, pixels)) = kept
        .symbols
        .get(name)
        .and_then(|drawings| drawings.iter().find(|(kept, _)| *kept == drawing))
    {
        return Some(pixels.clone());
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
    Some(pixels)
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
/// here, and neither has a symbol the system lacks.
pub fn icon(icon: &Icon, size: Size, ink: Color) -> Option<Pixels> {
    match icon {
        Icon::Symbol(name) => symbol(name, size, ink),
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
        let id = |pixels: Option<Pixels>| pixels.unwrap().signature().0;
        let first = id(symbol("moon.fill", ROW, ink));
        assert_eq!(id(symbol("moon.fill", ROW, ink)), first);
        assert_ne!(id(symbol("moon.fill", ROW, ink.alpha(0.5))), first);
        assert_ne!(id(symbol("moon.fill", INLINE, ink)), first);
        assert_ne!(id(symbol("sun.max", ROW, ink)), first);
        assert_eq!(srgb(ink), [255, 0, 0, 255]);
    }

    #[test]
    fn a_name_the_system_lacks_is_missing_once_drawn() {
        let pixels = symbol("no.such.symbol.anywhere", INLINE, Color::WHITE).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while pixels.size() == (0, 0) {
            assert!(std::time::Instant::now() < deadline, "the worker answers");
            std::thread::yield_now();
        }
        assert!(symbol("no.such.symbol.anywhere", INLINE, Color::WHITE).is_none());
        assert!(symbol("no.such.symbol.anywhere", ROW, Color::BLACK).is_none());
    }
}
