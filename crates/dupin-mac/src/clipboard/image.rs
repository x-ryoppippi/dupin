//! Converting a copied image to the PNG clipboard history keeps, with ImageIO.

use objc2_core_foundation::{
    CFBoolean, CFData, CFDictionary, CFMutableData, CFNumber, CFRetained, CFString, CFType,
};
use objc2_core_graphics::CGImage;
use objc2_image_io::{
    CGImageDestination, CGImageSource, kCGImagePropertyPixelHeight, kCGImagePropertyPixelWidth,
    kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
    kCGImageSourceShouldCache, kCGImageSourceThumbnailMaxPixelSize,
};

/// The longest side kept; larger images are scaled down to it.
pub(crate) const MAX_PIXELS: u32 = 4096;

const PNG: &str = "public.png";

/// The only image types ImageIO is let decode, in the order they are preferred when a copy
/// offers several. A copy's bytes are whatever the app that copied them put there, and an
/// extension's are whatever it stored, so no other codec ever parses them in this process.
pub(crate) const TYPES: [&str; 5] = [PNG, "public.tiff", "public.heic", "public.heif", "public.jpeg"];
/// The brands an ISO media file names to say it holds a HEIC or HEIF image.
const HEIF_BRANDS: [&[u8; 4]; 10] = [
    b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"hevm", b"hevs", b"mif1", b"msf1",
];

/// A copied image as PNG.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Image {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The image in `data` as PNG, at most `max_pixels` on its longest side, or `None` when it
/// can't be decoded or its PNG is larger than `max_bytes`. A PNG that already fits is kept as
/// copied, so copying it back gives the same bytes.
pub(crate) fn to_png(data: &[u8], max_pixels: u32, max_bytes: usize) -> Option<Image> {
    let source = source(data)?;
    // SAFETY: The options dictionaries map ImageIO's keys to CF values of the documented types;
    // `source` has an image at index 0.
    unsafe {
        let properties = source.properties_at_index(0, None)?;
        let properties: &CFDictionary<CFString, CFType> = properties.cast_unchecked();
        let width = dimension(properties, kCGImagePropertyPixelWidth)?;
        let height = dimension(properties, kCGImagePropertyPixelHeight)?;
        let is_png = source.r#type().is_some_and(|kind| kind.to_string() == PNG);
        if is_png && width.max(height) <= max_pixels && data.len() <= max_bytes {
            return Some(Image {
                png: data.to_vec(),
                width,
                height,
            });
        }
        let longest = CFNumber::new_i64(i64::from(width.max(height).min(max_pixels)));
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
        let png = encode(&image, PNG)?;
        (png.len() <= max_bytes).then(|| Image {
            png,
            width: u32::try_from(CGImage::width(Some(&image))).unwrap_or(u32::MAX),
            height: u32::try_from(CGImage::height(Some(&image))).unwrap_or(u32::MAX),
        })
    }
}

/// An ImageIO source for the image in `data`, or `None` unless it holds an image of one of
/// [`TYPES`]. Its leading bytes are checked before ImageIO reads any of it, and the type ImageIO
/// then finds after, which tells a TIFF from a camera RAW built on TIFF.
pub(crate) fn source(data: &[u8]) -> Option<CFRetained<CGImageSource>> {
    let allowed: &[&str] = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        &[PNG]
    } else if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        &["public.tiff"]
    } else if data.starts_with(b"\xff\xd8\xff") {
        &["public.jpeg"]
    } else if data.get(4..8) == Some(&b"ftyp"[..])
        && data
            .get(8..12)
            .is_some_and(|brand| HEIF_BRANDS.iter().any(|known| known[..] == *brand))
    {
        &["public.heic", "public.heif"]
    } else {
        return None;
    };
    let no_cache = CFDictionary::<CFString, CFType>::from_slices(
        &[unsafe { kCGImageSourceShouldCache }],
        &[CFBoolean::new(false).as_ref()],
    );
    // SAFETY: The options dictionary maps ImageIO's keys to CF values of the documented types.
    let source = unsafe { CGImageSource::with_data(&CFData::from_bytes(data), Some(no_cache.as_opaque())) }?;
    // SAFETY: Reading a source's type and count decodes no image.
    let found = unsafe { source.r#type() }?.to_string();
    (allowed.contains(&found.as_str()) && unsafe { source.count() } > 0).then_some(source)
}

pub(crate) fn dimension(properties: &CFDictionary<CFString, CFType>, key: &CFString) -> Option<u32> {
    let value = properties.get(key)?;
    let number = value.downcast_ref::<CFNumber>()?;
    u32::try_from(number.as_i64()?).ok().filter(|&pixels| pixels > 0)
}

/// `image` encoded as the uniform type `kind`.
pub(crate) fn encode(image: &CGImage, kind: &str) -> Option<Vec<u8>> {
    let out = CFMutableData::new(None, 0)?;
    // SAFETY: One image is added before finalizing, as the count promises; no options.
    unsafe {
        let destination: CFRetained<CGImageDestination> =
            CGImageDestination::with_data(&out, &CFString::from_str(kind), 1, None)?;
        destination.add_image(image, None);
        destination.finalize().then(|| out.to_vec())
    }
}
