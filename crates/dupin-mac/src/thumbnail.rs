//! Thumbnails of the images extensions keep, decoded and scaled down with ImageIO, which reads
//! PNG, TIFF, HEIC, HEIF and JPEG as clipboard history does. Call it off the panel's thread.

use std::io::Read;
use std::path::Path;

use objc2_core_foundation::{CFBoolean, CFData, CFDictionary, CFNumber, CFString, CFType};
use objc2_core_graphics::CGImage;
use objc2_image_io::{
    CGImageSource, kCGImagePropertyPixelHeight, kCGImagePropertyPixelWidth,
    kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
    kCGImageSourceShouldCache, kCGImageSourceThumbnailMaxPixelSize,
};

use crate::clipboard::image::{dimension, encode};

/// The largest encoded image read, as large as a copied image clipboard history reads.
pub const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
/// The most pixels an image may have: ImageIO may decode a PNG whole before scaling it down, so
/// this bounds what one thumbnail holds in memory while it is made.
pub const MAX_SOURCE_PIXELS: u64 = 64 * 1024 * 1024;

/// A scaled-down image, as PNG, and its size in pixels.
#[derive(Debug, PartialEq, Eq)]
pub struct Thumbnail {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The image in `data`, scaled down to fit `fit` pixels (width, height), keeping its aspect
/// ratio and its orientation, never scaled up. `None` when it isn't an image ImageIO decodes, or
/// is larger than [`MAX_SOURCE_BYTES`] or [`MAX_SOURCE_PIXELS`].
pub fn from_bytes(data: &[u8], fit: (u32, u32)) -> Option<Thumbnail> {
    if data.is_empty() || data.len() > MAX_SOURCE_BYTES || fit.0 == 0 || fit.1 == 0 {
        return None;
    }
    let no_cache = CFDictionary::<CFString, CFType>::from_slices(
        &[unsafe { kCGImageSourceShouldCache }],
        &[CFBoolean::new(false).as_ref()],
    );
    // SAFETY: The options dictionary maps ImageIO's keys to CF values of the documented types.
    let source = unsafe { CGImageSource::with_data(&CFData::from_bytes(data), Some(no_cache.as_opaque())) }?;
    // SAFETY: As above; index 0 is checked to exist first.
    unsafe {
        if source.count() == 0 {
            return None;
        }
        let properties = source.properties_at_index(0, None)?;
        let properties: &CFDictionary<CFString, CFType> = properties.cast_unchecked();
        let width = dimension(properties, kCGImagePropertyPixelWidth)?;
        let height = dimension(properties, kCGImagePropertyPixelHeight)?;
        if u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
            return None;
        }
        let scale = (f64::from(fit.0) / f64::from(width))
            .min(f64::from(fit.1) / f64::from(height))
            .min(1.0);
        let longest = (f64::from(width.max(height)) * scale).ceil().max(1.0) as i64;
        let longest = CFNumber::new_i64(longest);
        let options = CFDictionary::<CFString, CFType>::from_slices(
            &[
                kCGImageSourceCreateThumbnailFromImageAlways,
                kCGImageSourceCreateThumbnailWithTransform,
                kCGImageSourceThumbnailMaxPixelSize,
                kCGImageSourceShouldCache,
            ],
            &[
                CFBoolean::new(true).as_ref(),
                CFBoolean::new(true).as_ref(),
                longest.as_ref(),
                CFBoolean::new(false).as_ref(),
            ],
        );
        let image = source.thumbnail_at_index(0, Some(options.as_opaque()))?;
        Some(Thumbnail {
            png: encode(&image, "public.png")?,
            width: u32::try_from(CGImage::width(Some(&image))).ok()?,
            height: u32::try_from(CGImage::height(Some(&image))).ok()?,
        })
    }
}

/// As [`from_bytes`], for the image in the regular file at `path`, which is read only when it
/// is no larger than [`MAX_SOURCE_BYTES`].
pub fn from_file(path: &Path, fit: (u32, u32)) -> Option<Thumbnail> {
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES as u64 {
        return None;
    }
    let mut data = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_SOURCE_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .ok()?;
    from_bytes(&data, fit)
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CFData, CFRetained};
    use objc2_core_graphics::{
        CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    };

    use super::*;

    fn encoded(width: usize, height: usize, kind: &str) -> Vec<u8> {
        let bytes: Vec<u8> = (0..width * height)
            .flat_map(|i| [(i % 251) as u8, (i % 13) as u8 * 19, 200, 255])
            .collect();
        let provider = CGDataProvider::with_cf_data(Some(&CFData::from_bytes(&bytes))).unwrap();
        // SAFETY: The buffer holds `height` rows of `width` RGBX pixels; no decode array.
        let image: CFRetained<CGImage> = unsafe {
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
        .unwrap();
        encode(&image, kind).unwrap()
    }

    #[test]
    fn images_shrink_to_fit_keeping_their_shape_and_never_grow() {
        let wide = encoded(400, 100, "public.png");
        let thumbnail = from_bytes(&wide, (64, 64)).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (64, 16));
        assert!(thumbnail.png.starts_with(b"\x89PNG"));

        let tall = encoded(100, 400, "public.tiff");
        let thumbnail = from_bytes(&tall, (600, 200)).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (50, 200));

        let small = encoded(20, 10, "public.jpeg");
        let thumbnail = from_bytes(&small, (64, 64)).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (20, 10));
    }

    #[test]
    fn what_isnt_an_image_or_is_too_large_has_no_thumbnail() {
        assert!(from_bytes(b"not an image", (64, 64)).is_none());
        assert!(from_bytes(&[], (64, 64)).is_none());
        assert!(from_bytes(&encoded(4, 4, "public.png"), (0, 64)).is_none());

        let folder = tempfile::tempdir().unwrap();
        let png = folder.path().join("a.png");
        std::fs::write(&png, encoded(8, 8, "public.png")).unwrap();
        assert_eq!(from_file(&png, (4, 4)).map(|t| (t.width, t.height)), Some((4, 4)));
        assert!(from_file(folder.path(), (4, 4)).is_none(), "a folder");
        assert!(from_file(&folder.path().join("gone.png"), (4, 4)).is_none());
        let huge = folder.path().join("huge.png");
        std::fs::File::create(&huge)
            .unwrap()
            .set_len(MAX_SOURCE_BYTES as u64 + 1)
            .unwrap();
        assert!(from_file(&huge, (4, 4)).is_none());
    }
}
