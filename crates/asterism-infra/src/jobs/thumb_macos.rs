//! ImageIO fast path for `thumb_gen` on macOS.
//!
//! `CGImageSourceCreateThumbnailAtIndex` reads a JPEG (or HEIC / PNG),
//! resizes to a target longer edge, and returns a `CGImage` in one
//! call. On Apple Silicon this route flips to the hardware JPEG
//! decoder, keeping CPU load close to zero during a large import
//! wave — the pure-Rust `image` crate path (see `handlers::make_thumb`
//! fallback) burns a full core per decode even at `Lanczos3`.
//!
//! The returned bytes are JPEG-encoded via `CGImageDestination` so
//! the DB blob shape stays identical across platforms.

use std::ffi::c_void;

use asterism_core::error::DomainError;
use objc2_core_foundation::{
    CFDictionary, CFIndex, CFMutableData, CFNumber, CFNumberType, CFString, CFURL,
    kCFAllocatorDefault, kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks,
};
use objc2_image_io::{
    CGImageDestination, CGImageSource, kCGImageSourceCreateThumbnailFromImageAlways,
    kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceThumbnailMaxPixelSize,
};

pub(crate) fn err<S: Into<String>>(msg: S) -> DomainError {
    DomainError::Infra(anyhow::anyhow!(msg.into()))
}

/// Encodes a `CGImage` to JPEG bytes via `CGImageDestination`.
///
/// Shared with the video path (`thumb_video`), which produces its
/// frame through AVFoundation but has to land in the same blob shape:
/// `thumb_cache` holds one column for every thumbnail regardless of
/// what the source was, and the UI decodes it without asking.
pub(crate) fn encode_jpeg(cg_image: &objc2_core_graphics::CGImage) -> Result<Vec<u8>, DomainError> {
    let out =
        CFMutableData::new(None, 0).ok_or_else(|| err("thumb ImageIO: CFMutableData failed"))?;
    let jpeg_uti = CFString::from_str("public.jpeg");
    let dest = unsafe { CGImageDestination::with_data(&out, &jpeg_uti, 1, None) }
        .ok_or_else(|| err("thumb ImageIO: CGImageDestination failed"))?;
    unsafe { dest.add_image(cg_image, None) };
    if !unsafe { dest.finalize() } {
        return Err(err("thumb ImageIO: CGImageDestinationFinalize failed"));
    }
    Ok(out.to_vec())
}

/// Decodes the JPEG at `path_str`, resizes so the longer edge is
/// `target_px`, and returns JPEG-encoded bytes. Runs synchronously —
/// call inside `spawn_blocking` from an async handler.
pub fn make_thumb(path_str: &str, target_px: u32) -> Result<Vec<u8>, DomainError> {
    // 1. Build a CFURL for the on-disk file. ImageIO can also read
    //    from CFData, but the URL form avoids the extra copy into
    //    memory before decode.
    let path_cf = CFString::from_str(path_str);
    let url = unsafe {
        CFURL::from_file_system_representation(
            kCFAllocatorDefault,
            path_str.as_ptr(),
            path_str.len() as CFIndex,
            false,
        )
    }
    .ok_or_else(|| err(format!("thumb ImageIO: CFURL failed for {path_str}")))?;
    let _ = path_cf; // silence unused warning if we later drop the CFString path.

    // 2. Wrap the URL in a CGImageSource. `None` options are fine —
    //    the max-pixel-size hint travels with the thumbnail call.
    let source = unsafe { CGImageSource::with_url(&url, None) }.ok_or_else(|| {
        err(format!(
            "thumb ImageIO: CGImageSource failed for {path_str}"
        ))
    })?;

    // 3. Compose the thumbnail options dictionary. Keys:
    //    - `kCGImageSourceThumbnailMaxPixelSize` = target_px
    //    - `kCGImageSourceCreateThumbnailFromImageAlways` = true
    //      (skip any embedded EXIF thumbnail, which is usually 160 px
    //      max — we care about the caller's target size)
    //    - `kCGImageSourceCreateThumbnailWithTransform` = true
    //      (respect EXIF orientation so portrait photos land upright)
    let target_i32: i32 = target_px as i32;
    let target_num = unsafe {
        CFNumber::new(
            None,
            CFNumberType::IntType,
            &target_i32 as *const i32 as *const c_void,
        )
    }
    .ok_or_else(|| err("thumb ImageIO: CFNumber failed"))?;
    let true_bool = unsafe {
        // `kCFBooleanTrue` is `Option<&'static CFBoolean>`; unwrap is
        // safe because Core Foundation always provides the singleton.
        objc2_core_foundation::kCFBooleanTrue.expect("kCFBooleanTrue missing")
    };

    let keys: [*const c_void; 3] = unsafe {
        [
            (kCGImageSourceThumbnailMaxPixelSize as *const CFString).cast(),
            (kCGImageSourceCreateThumbnailFromImageAlways as *const CFString).cast(),
            (kCGImageSourceCreateThumbnailWithTransform as *const CFString).cast(),
        ]
    };
    let values: [*const c_void; 3] = [
        (&*target_num as *const CFNumber).cast(),
        (true_bool as *const _ as *const c_void),
        (true_bool as *const _ as *const c_void),
    ];
    let options = unsafe {
        CFDictionary::new(
            None,
            keys.as_ptr() as *mut *const c_void,
            values.as_ptr() as *mut *const c_void,
            keys.len() as CFIndex,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
    }
    .ok_or_else(|| err("thumb ImageIO: CFDictionary failed"))?;

    // 4. Ask ImageIO for the resized image. On Apple Silicon this
    //    call short-circuits through the hardware JPEG decoder for
    //    supported inputs.
    let cg_image = unsafe { source.thumbnail_at_index(0, Some(&options)) }.ok_or_else(|| {
        err(format!(
            "thumb ImageIO: thumbnail_at_index returned None for {path_str}"
        ))
    })?;

    // 5. Encode the resulting CGImage back to JPEG bytes so the DB
    //    blob shape matches the fallback path (`public.jpeg` UTI).
    encode_jpeg(&cg_image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::material::KNOWN_IMAGE_MIMES;

    /// The image half of the format tripwire (sibling of
    /// `thumb_ffmpeg`'s video one). Every mime the extension table can
    /// produce must be one this extractor is *known* to decode — when
    /// a tenth format lands in `KNOWN_IMAGE_MIMES`, this test names
    /// the missing capability decision instead of the format falling
    /// through as an imported-but-invisible tile.
    #[test]
    fn every_known_image_mime_is_a_deliberate_imageio_capability() {
        for mime in KNOWN_IMAGE_MIMES {
            match *mime {
                // ImageIO decodes all of these natively on the macOS
                // versions this app targets; the fixture test below
                // proves the exotic half against real bytes.
                "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/heic"
                | "image/heif" | "image/avif" | "image/tiff" | "image/bmp" => {}
                other => panic!(
                    "{other} is in KNOWN_IMAGE_MIMES but has no acknowledged thumbnail \
                     capability — verify ImageIO (or add a route) and extend this test"
                ),
            }
        }
    }

    /// Real bytes through the real decoder: the importer's format
    /// fixtures (the exotic half of the list — HEIC / AVIF / TIFF /
    /// BMP / interlaced GIF) each yield a JPEG thumbnail. This is what
    /// "the mime map entry is honest" means — before the map knew
    /// these formats, the files imported fine and then sat invisible.
    ///
    /// Panics when a fixture is missing (they are written by
    /// `scripts/gen-test-fixtures.py` and owned by
    /// `asterism-importer-image`'s tests) — this repo's fixture tests
    /// fail loudly rather than skip silently.
    #[test]
    fn the_importer_format_fixtures_all_yield_a_jpeg_thumbnail() {
        let fixtures = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../asterism-importer-image/tests/fixtures");
        for name in [
            "testcard.heic",
            "testcard.avif",
            "testcard.tiff",
            "testcard.bmp",
            "testcard.gif",
        ] {
            let path = fixtures.join(name);
            assert!(
                path.is_file(),
                "fixture {name} missing at {} — regenerate with \
                 `python3 scripts/gen-test-fixtures.py`",
                path.display()
            );
            let bytes = make_thumb(path.to_str().unwrap(), 64)
                .unwrap_or_else(|e| panic!("{name} did not thumbnail: {e}"));
            assert!(
                bytes.starts_with(&[0xFF, 0xD8]),
                "{name}: the blob is a JPEG, same shape as every other thumbnail"
            );
        }
    }
}
