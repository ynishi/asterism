//! Preview-rendition transcode: an unplayable video in, an H.264 MP4
//! out.
//!
//! The embedded webview cannot display VP9 at all and rejects the
//! Matroska container (measured 2026-07-31), and the
//! corpus this app is for — generation-tool output — emits VP9 WebM
//! by default. No delivery trick fixes a codec the engine will not
//! decode in the DOM, so the fix is the one video sites use: keep the
//! original untouched in the ledger and play a transcoded rendition.
//! H.264 + AAC in MP4 is the one combination every measured route
//! plays.
//!
//! The rendition is a **cache**, not a copy of record: capped
//! resolution, disposable, regenerable from the original at any time
//! — the thumbnail relationship at video scale. It lives beside the
//! profile database (`<profile>/previews/<asset_id>.mp4`) so tests
//! that sandbox the database sandbox the renditions with it.
//!
//! Like `thumb_ffmpeg`, this shells out to an installed ffmpeg
//! binary; when none is present the job fails naming the fix instead
//! of leaving a silent crossed-out player.

use std::path::Path;

use asterism_core::error::DomainError;

use super::thumb_ffmpeg::{ffmpeg_binary, ffmpeg_command};

/// Box the rendition fits in (longer edge, pixels). Preview quality
/// is deliberately below original quality — the original is one
/// click away and the rendition's job is "does this clip look right",
/// not archival fidelity.
pub const PREVIEW_MAX_EDGE: u32 = 1280;

/// H.264 encoders to try, in order. `libx264` is what a host-installed
/// ffmpeg (Homebrew's, for one) usually carries; the second is the
/// operating system's encoder, which an LGPL-clean ffmpeg without
/// libx264 carries instead — trying both keeps this working across
/// either shape of the dependency.
///
/// The second entry differs per platform: `h264_mf` (Media Foundation)
/// on Windows, `h264_videotoolbox` (Apple's VideoToolbox) off it. Media
/// Foundation does not exist off Windows and VideoToolbox does not
/// exist on it, so listing both on either side would add an attempt
/// that cannot succeed there, and its "unknown encoder" would replace
/// the real reason as the last error.
#[cfg(not(windows))]
const ENCODERS: &[&str] = &["libx264", "h264_videotoolbox"];
#[cfg(windows)]
const ENCODERS: &[&str] = &["libx264", "h264_mf"];

/// What ffmpeg's `h264_mf` encoder logs when `mfplat.dll` cannot be
/// loaded — Windows N and KN editions, which ship without Media
/// Foundation until the Media Feature Pack is installed. Logged at
/// error level, so the `-v error` every run passes keeps it.
const MEDIA_FOUNDATION_ABSENT: &str = "DLL mfplat.dll failed to open";

/// The quality arguments for `encoder`. Each encoder has its own
/// vocabulary: x264 speaks preset/CRF, VideoToolbox a 1-100 quality
/// scale through `-q:v` (and rejects the x264 words).
///
/// `h264_mf` needs its own arm rather than VideoToolbox's `-q:v`: the
/// ffmpeg CLI turns `-q:v 55` into a `global_quality` of 55 × 118 =
/// 6490, and `h264_mf` hands `global_quality` to Media Foundation as a
/// QP, whose range is 16-51. Its 1-100 scale is `-quality`, which takes
/// effect under `-rate_control quality`.
fn quality_args(encoder: &str) -> &'static [&'static str] {
    match encoder {
        "libx264" => &["-preset", "veryfast", "-crf", "27"],
        "h264_mf" => &["-rate_control", "quality", "-quality", "55"],
        _ => &["-q:v", "55"],
    }
}

/// Whether one attempt's failure was `encoder` being `h264_mf` and
/// Media Foundation not loading.
fn media_foundation_absent(encoder: &str, stderr: &str) -> bool {
    encoder == "h264_mf" && stderr.contains(MEDIA_FOUNDATION_ABSENT)
}

/// The job's error once every encoder has failed. `last_err` is the
/// last attempt's stderr; `mf_absent` says whether the `h264_mf`
/// attempt failed because Media Foundation did not load, in which case
/// the message names the fix, since ffmpeg's line does not.
///
/// It says only that the Media Foundation encoder could not start, not
/// that the rendition needs it: an earlier encoder may have failed for
/// a reason of its own, which installing the pack would not change.
fn failure_message(src: &str, last_err: &str, mf_absent: bool) -> String {
    if mf_absent {
        format!(
            "{src}: ffmpeg produced no rendition, and its Media Foundation encoder could not \
             start because mfplat.dll did not load — on Windows N and KN editions, install \
             the Media Feature Pack: {last_err}"
        )
    } else {
        format!("{src}: ffmpeg produced no rendition: {last_err}")
    }
}

// Path naming lives in the domain (`render::video_preview_path` and
// siblings) — the status endpoint in core reads the same files this
// module writes, and one owner keeps the two sides from drifting.
pub use asterism_core::domain::render::{
    video_preview_failed_path as failed_marker_path, video_preview_part_path as part_marker_path,
    video_preview_path as preview_path,
};

/// Transcodes `src` into `dest` (H.264 + AAC MP4, capped to
/// [`PREVIEW_MAX_EDGE`]). Synchronous — call inside `spawn_blocking`.
///
/// Writes to the `.part` path and renames on success, so a `dest`
/// that exists is always a complete rendition.
pub fn make_preview(src: &str, previews_dir: &Path, asset_id: &str) -> Result<(), DomainError> {
    let Some(bin) = ffmpeg_binary() else {
        return Err(DomainError::Infra(anyhow::anyhow!(
            "{src}: preview rendition needs ffmpeg, which was not found — \
             install it (e.g. `brew install ffmpeg`) or point $ASTERISM_FFMPEG at a binary"
        )));
    };
    let dest = preview_path(previews_dir, asset_id);
    let part = part_marker_path(previews_dir, asset_id);
    // Downscale-only fit into the box; H.264 needs even dimensions.
    let scale = format!(
        "scale=w={PREVIEW_MAX_EDGE}:h={PREVIEW_MAX_EDGE}:force_original_aspect_ratio=decrease:force_divisible_by=2"
    );
    let mut last_err = String::new();
    let mut mf_absent = false;
    for encoder in ENCODERS {
        let mut cmd = ffmpeg_command(&bin);
        cmd.args(["-v", "error", "-y", "-i", src])
            // First video stream, first audio stream if any; drop
            // subtitles / data / attachments a Matroska may carry.
            .args(["-map", "0:v:0", "-map", "0:a:0?", "-sn", "-dn"])
            .args(["-vf", &scale, "-c:v", encoder, "-pix_fmt", "yuv420p"])
            .args(quality_args(encoder));
        let output = cmd
            .args(["-c:a", "aac", "-b:a", "128k"])
            // moov up front so progressive playback starts immediately.
            .args(["-movflags", "+faststart", "-f", "mp4"])
            .arg(&part)
            .output()
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("{src}: ffmpeg spawn failed: {e}")))?;
        if output.status.success() && part.is_file() {
            std::fs::rename(&part, &dest).map_err(|e| {
                DomainError::Infra(anyhow::anyhow!("{}: rename failed: {e}", dest.display()))
            })?;
            return Ok(());
        }
        last_err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        mf_absent |= media_foundation_absent(encoder, &last_err);
        let _ = std::fs::remove_file(&part);
    }
    Err(DomainError::Infra(anyhow::anyhow!(failure_message(
        src, &last_err, mf_absent
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Off Windows the list is the one it was before Windows joined it:
    /// x264 first when the host ffmpeg has it, VideoToolbox after.
    #[cfg(not(windows))]
    #[test]
    fn off_windows_the_encoders_are_x264_then_videotoolbox() {
        assert_eq!(ENCODERS, ["libx264", "h264_videotoolbox"]);
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_the_encoders_are_x264_then_media_foundation() {
        assert_eq!(ENCODERS, ["libx264", "h264_mf"]);
    }

    #[test]
    fn each_encoder_gets_its_own_quality_arguments() {
        assert_eq!(
            quality_args("libx264"),
            ["-preset", "veryfast", "-crf", "27"]
        );
        assert_eq!(quality_args("h264_videotoolbox"), ["-q:v", "55"]);
        assert_eq!(
            quality_args("h264_mf"),
            ["-rate_control", "quality", "-quality", "55"]
        );
    }

    /// `-q:v` reaches `h264_mf` as a QP of 6490, so it must never be in
    /// that encoder's arguments.
    #[test]
    fn media_foundation_is_never_handed_q_v() {
        assert!(!quality_args("h264_mf").contains(&"-q:v"));
    }

    /// Only an `h264_mf` attempt that logged the DLL line counts: the
    /// line under another encoder's name, or another `h264_mf` failure,
    /// is not Media Foundation being absent.
    #[test]
    fn media_foundation_is_absent_only_when_h264_mf_says_so() {
        let dll = "[h264_mf @ 0x0] DLL mfplat.dll failed to open";
        assert!(media_foundation_absent("h264_mf", dll));
        assert!(!media_foundation_absent("libx264", dll));
        assert!(!media_foundation_absent(
            "h264_mf",
            "Error while opening encoder"
        ));
    }

    #[test]
    fn a_missing_media_foundation_is_named_with_its_fix() {
        let stderr = "[h264_mf @ 0x0] DLL mfplat.dll failed to open\n\
                      Error while opening encoder";
        let message = failure_message("clip.webm", stderr, true);
        assert!(message.contains("Media Feature Pack"), "{message}");
        assert!(
            message.contains(stderr),
            "ffmpeg's own words stay: {message}"
        );
    }

    /// Any other failure reads exactly as it did before `h264_mf`
    /// existed.
    #[test]
    fn any_other_failure_keeps_the_plain_message() {
        assert_eq!(
            failure_message("clip.webm", "Unknown encoder 'libx264'", false),
            "clip.webm: ffmpeg produced no rendition: Unknown encoder 'libx264'"
        );
    }

    /// End-to-end through a real ffmpeg: synthesise a VP9 WebM (the
    /// format the webview cannot play), transcode it, and get a
    /// faststart MP4 back. Panics with the install instruction when
    /// ffmpeg is absent — fixture tests fail loudly, not silently.
    #[test]
    fn a_vp9_webm_becomes_a_playable_mp4_rendition() {
        let bin = ffmpeg_binary().expect(
            "ffmpeg is required for this test: brew install ffmpeg (or set $ASTERISM_FFMPEG)",
        );
        let tmp = tempfile::tempdir().expect("tempdir");
        let clip = tmp.path().join("in.webm");
        let status = ffmpeg_command(&bin)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1:size=322x240:rate=10", // odd width on purpose
            ])
            .arg(&clip)
            .status()
            .expect("synthesise webm");
        assert!(status.success(), "ffmpeg could not synthesise the fixture");

        let previews = tmp.path().join("previews");
        std::fs::create_dir_all(&previews).expect("previews dir");
        make_preview(clip.to_str().unwrap(), &previews, "test-asset").expect("transcode");

        let dest = preview_path(&previews, "test-asset");
        let bytes = std::fs::read(&dest).expect("rendition exists");
        // An MP4 opens with an ftyp box at offset 4.
        assert_eq!(&bytes[4..8], b"ftyp", "the rendition is an MP4");
        assert!(
            !part_marker_path(&previews, "test-asset").exists(),
            "the .part staging file was renamed away"
        );
    }
}
