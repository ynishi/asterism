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
use std::process::Command;

use asterism_core::error::DomainError;

use super::thumb_ffmpeg::ffmpeg_binary;

/// Box the rendition fits in (longer edge, pixels). Preview quality
/// is deliberately below original quality — the original is one
/// click away and the rendition's job is "does this clip look right",
/// not archival fidelity.
pub const PREVIEW_MAX_EDGE: u32 = 1280;

/// H.264 encoders to try, in order. `libx264` is what Homebrew's
/// ffmpeg carries; `h264_videotoolbox` (Apple's hardware encoder) is
/// what an LGPL-clean bundled ffmpeg would carry instead — trying
/// both keeps this working across either shape of the dependency.
const ENCODERS: &[&str] = &["libx264", "h264_videotoolbox"];

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
    for encoder in ENCODERS {
        let mut cmd = Command::new(&bin);
        cmd.args(["-v", "error", "-y", "-i", src])
            // First video stream, first audio stream if any; drop
            // subtitles / data / attachments a Matroska may carry.
            .args(["-map", "0:v:0", "-map", "0:a:0?", "-sn", "-dn"])
            .args(["-vf", &scale, "-c:v", encoder, "-pix_fmt", "yuv420p"]);
        // Quality knobs are per-encoder vocabularies: x264 speaks
        // preset/CRF, VideoToolbox speaks a 1-100 quality scale and
        // rejects the x264 words.
        match *encoder {
            "libx264" => {
                cmd.args(["-preset", "veryfast", "-crf", "27"]);
            }
            _ => {
                cmd.args(["-q:v", "55"]);
            }
        }
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
        let _ = std::fs::remove_file(&part);
    }
    Err(DomainError::Infra(anyhow::anyhow!(
        "{src}: ffmpeg produced no rendition: {last_err}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let status = Command::new(&bin)
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
