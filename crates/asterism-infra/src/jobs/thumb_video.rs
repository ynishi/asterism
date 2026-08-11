//! Video frame extraction for `thumb_gen` on macOS.
//!
//! A video has no thumbnail without this: ImageIO (`thumb_macos`) and
//! the pure-Rust `image` crate both decode stills only, so before this
//! path existed a video asset simply showed an empty tile in the grid.
//! `AVAssetImageGenerator` pulls one frame as a `CGImage`, which
//! `thumb_macos::encode_jpeg` then turns into the same JPEG blob every
//! other thumbnail is stored as — the cache, the UI and the palette
//! extractor cannot tell the two sources apart, which is the point.
//!
//! Format coverage is AVFoundation's, not the mime map's. `video/mp4`
//! and `video/quicktime` open natively; `video/webm` does not (macOS
//! ships no WebM demuxer), so those assets fail here and keep the
//! empty tile they had before. The failure is soft — one logged job
//! failure per enqueue, no retry storm (`jobs::mod` has no retry
//! policy, and the UI stops re-kicking after its own budget) — but it
//! means the extension→mime table in `domain::material` and this
//! extractor's real capabilities are two lists that can drift apart.
//!
//! The generator's synchronous `copyCGImageAtTime` is deprecated in
//! favour of the async form. It is the right call here anyway: the
//! whole `thumb_gen` handler already runs inside `spawn_blocking`
//! under a decode-slot semaphore, so blocking is the contract this
//! path is called under, and an async completion handler would only
//! add a channel round-trip to get back to it.

use asterism_core::error::DomainError;
use objc2_av_foundation::{AVAssetImageGenerator, AVURLAsset};
use objc2_core_foundation::CGSize;
use objc2_core_media::CMTime;
use objc2_foundation::{NSString, NSURL};

use super::thumb_macos::{encode_jpeg, err};

/// Where in the video to grab the frame, as a fraction of its length.
///
/// Not zero: the first frame of a recording is very often a black or
/// half-faded one, and a grid of black tiles is no better than a grid
/// of empty ones.
const FRAME_POSITION_RATIO: f64 = 0.1;

/// Upper bound on that offset, in seconds. For anything longer than
/// ~10 s the 10 % rule would walk far into the video for no benefit —
/// one second in is already past the fade.
const FRAME_POSITION_MAX_SECS: f64 = 1.0;

/// Timescale for the seek request. 600 is the conventional CoreMedia
/// value: it divides evenly by the common frame rates (24 / 25 / 30 /
/// 60), so the requested instant lands on a frame boundary.
const TIMESCALE: i32 = 600;

/// Picks the instant to grab, given the video's length in seconds.
///
/// Duration is unknown (`CMTIME_IS_INDEFINITE`) for a stream and comes
/// back as NaN for a file AVFoundation cannot parse; both are
/// non-finite. Seeking to NaN would be rejected by the generator, and
/// guessing "the middle" of an unknown length is meaningless, so an
/// unreadable duration falls back to the fixed cap — a video long
/// enough to have a duration problem is long enough to have a frame
/// one second in.
fn frame_position_secs(duration_secs: f64) -> f64 {
    if duration_secs.is_finite() && duration_secs > 0.0 {
        (duration_secs * FRAME_POSITION_RATIO).min(FRAME_POSITION_MAX_SECS)
    } else {
        FRAME_POSITION_MAX_SECS
    }
}

/// Extracts one frame from the video at `path_str`, scaled to fit a
/// `target_px` box, and returns JPEG-encoded bytes.
///
/// Runs synchronously — call inside `spawn_blocking`, like the image
/// path it sits beside.
pub fn make_thumb(path_str: &str, target_px: u32) -> Result<Vec<u8>, DomainError> {
    // Every Objective-C call may autorelease internally, and a tokio
    // blocking-pool thread has no pool of its own — without this the
    // objects AVFoundation drops while parsing the container (tracks,
    // readers, option dictionaries, the `NSError` on the failure path)
    // accumulate until the thread exits, which during an import wave
    // means "for the whole wave". The sibling ImageIO path needs no
    // such wrapper: CoreFoundation is create-rule throughout, this is
    // the first Objective-C in the crate.
    objc2::rc::autoreleasepool(|_pool| make_thumb_inner(path_str, target_px))
}

fn make_thumb_inner(path_str: &str, target_px: u32) -> Result<Vec<u8>, DomainError> {
    let path = NSString::from_str(path_str);
    let url = NSURL::fileURLWithPath(&path);
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };

    let duration_secs = unsafe { asset.duration().seconds() };
    let position_secs = frame_position_secs(duration_secs);
    let at = unsafe { CMTime::with_seconds(position_secs, TIMESCALE) };

    let generator = unsafe { AVAssetImageGenerator::assetImageGeneratorWithAsset(&asset) };
    unsafe {
        // Portrait video carries its rotation in the track transform;
        // without this the frame comes out sideways.
        generator.setAppliesPreferredTrackTransform(true);
        // Bound the output to the tile the caller asked for.
        // `maximumSize` fits inside the box and preserves the aspect
        // ratio, so this is the same "longer edge = target" contract
        // the ImageIO path applies. Whether AVFoundation also avoids
        // decoding the frame at native resolution first is not
        // something the API promises — the guarantee here is on the
        // size of what comes back, not on what it cost to make.
        generator.setMaximumSize(CGSize {
            width: f64::from(target_px),
            height: f64::from(target_px),
        });
        // Half a second of slack either way. Exact seeking forces the
        // decoder back to the previous keyframe and forward again
        // frame by frame; for a thumbnail, the nearest keyframe is
        // indistinguishable to the viewer and much cheaper.
        let tolerance: CMTime = CMTime::new(i64::from(TIMESCALE) / 2, TIMESCALE);
        generator.setRequestedTimeToleranceBefore(tolerance);
        generator.setRequestedTimeToleranceAfter(tolerance);
    }

    #[allow(deprecated)] // see the module doc: sync is the contract here
    let frame = unsafe { generator.copyCGImageAtTime_actualTime_error(at, std::ptr::null_mut()) }
        .map_err(|e| {
        err(format!(
            "thumb AVFoundation: no frame at {position_secs:.2}s for {path_str}: {e}"
        ))
    })?;

    encode_jpeg(&frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_position_skips_the_opening_and_caps_out() {
        // Short clip: 10 % in, past the fade, still near the start.
        assert!((frame_position_secs(4.0) - 0.4).abs() < f64::EPSILON);
        // Long clip: the cap keeps the seek cheap.
        assert!((frame_position_secs(600.0) - 1.0).abs() < f64::EPSILON);
        // Never the very first frame, which is often black.
        assert!(frame_position_secs(0.5) > 0.0);
    }

    #[test]
    fn unreadable_duration_falls_back_instead_of_seeking_to_nan() {
        for bad in [f64::NAN, f64::INFINITY, 0.0, -1.0] {
            let at = frame_position_secs(bad);
            assert!(at.is_finite() && at > 0.0, "duration {bad} → {at}");
        }
    }

    /// Real extraction against a real container. Ignored by default:
    /// it needs a video file, and the repository does not carry binary
    /// fixtures. Point it at one and run explicitly:
    ///
    /// ```text
    /// ffmpeg -f lavfi -i testsrc=size=640x360:rate=30 -t 3 /tmp/sample.mp4
    /// ASTERISM_TEST_VIDEO=/tmp/sample.mp4 \
    ///   cargo test -p asterism-infra thumb_video -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs ASTERISM_TEST_VIDEO pointing at a real video file"]
    fn extracts_a_jpeg_frame_from_a_real_video() {
        let path = std::env::var("ASTERISM_TEST_VIDEO")
            .expect("set ASTERISM_TEST_VIDEO to a video file path");
        let bytes = make_thumb(&path, 256).expect("frame extraction failed");
        assert!(
            bytes.len() > 1024,
            "suspiciously small: {} bytes",
            bytes.len()
        );
        // JPEG SOI marker — proof the blob is the shape `thumb_cache`
        // and the UI expect, not some other image container.
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "not a JPEG");
        // Written out so the frame can be looked at: "is there a
        // picture in the tile" is the actual requirement, and no
        // assertion on bytes can answer it.
        let out = std::env::temp_dir().join("asterism-thumb-video-smoke.jpg");
        std::fs::write(&out, &bytes).expect("write the frame for inspection");
        eprintln!(
            "extracted {} bytes from {path} → {}",
            bytes.len(),
            out.display()
        );
    }
}
