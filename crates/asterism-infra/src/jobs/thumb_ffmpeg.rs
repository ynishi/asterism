//! Video frame extraction through an external `ffmpeg`, for the
//! formats AVFoundation cannot open.
//!
//! macOS ships no WebM / Matroska / AVI demuxer, so before this path
//! existed a `.webm` clip kept an empty tile forever — the thumb job
//! failed softly and the grid showed nothing (journal 2026-07-27, the
//! "webm mkv avi" carry). Bundling an ffmpeg *library* was rejected
//! back then for its build cost, and still is; this module instead
//! shells out to an ffmpeg *binary* when one is installed, and fails
//! with an instruction naming the fix when none is.
//!
//! # Routing, and the drift this module closes
//!
//! [`route_for`] is the single answer to "which extractor can turn
//! this video mime into a frame". The extension→mime table
//! (`asterism_core::domain::material`) and the extractors' real
//! capabilities used to be two lists growing independently — a format
//! added to the table without an extractor showed up as an empty
//! tile, not an error. The test at the bottom walks
//! `KNOWN_VIDEO_MIMES` and fails the moment an entry has no
//! deliberate route, so the two lists can no longer drift silently.
//!
//! # Why the binary is probed at fixed paths too
//!
//! A GUI app launched through LaunchServices inherits a minimal
//! `PATH` (`/usr/bin:/bin:…`) that does not contain Homebrew's
//! prefix, so `Command::new("ffmpeg")` alone would report "not
//! installed" on exactly the machines that have it. The probe order
//! is: `$ASTERISM_FFMPEG` (explicit override) → the bundled sidecar
//! beside the executable → `PATH` → the usual install prefixes.
//!
//! The sidecar is an LGPL-clean ffmpeg (no libx264 — H.264 encoding
//! rides `h264_videotoolbox`) that `tauri build` copies to
//! `Asterism.app/Contents/MacOS/ffmpeg` via `bundle.externalBin`;
//! `scripts/build-ffmpeg-sidecar.sh` produces it. It outranks `PATH`
//! so the app prefers the binary it shipped and was tested with over
//! whatever the host happens to carry, and a clean machine (no
//! Homebrew) plays video out of the box.

use std::path::PathBuf;
use std::process::Command;

use asterism_core::domain::value::MimeType;
use asterism_core::error::DomainError;

/// Which extractor owns a video mime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoThumbRoute {
    /// AVFoundation demuxes it natively (`thumb_video`, macOS).
    Native,
    /// Needs the external ffmpeg binary (this module).
    ExternalFfmpeg,
}

/// Picks the extraction route for a video mime.
///
/// `None` (mime unknown) routes native: that is the pre-existing
/// behaviour for unclassified rows, and AVFoundation's own error is
/// more informative than a guessed ffmpeg invocation.
/// The set lives on [`VideoFormat`] rather than here: this question and
/// "does the detail player need a transcoded rendition?"
/// (`render::needs_video_preview`) are the same three containers, and
/// they used to be two copies of the same literals in two crates —
/// adding a format to one and not the other is silent in both
/// directions (an empty tile, or a crossed-out player).
pub fn route_for(mime: Option<&MimeType>) -> VideoThumbRoute {
    match mime {
        Some(MimeType::Video(f)) if f.needs_external_frame_grab() => {
            VideoThumbRoute::ExternalFfmpeg
        }
        _ => VideoThumbRoute::Native,
    }
}

/// Locates an ffmpeg binary, or says how to get one. Shared with the
/// preview-rendition transcoder (`preview_ffmpeg`), which rides the
/// same soft dependency.
///
/// `pub` so tests outside this crate resolve ffmpeg the way the app
/// does. A test that reimplements the probe drifts from it: the
/// server-side preview e2e carried its own `PATH`-then-prefixes copy
/// and so could not see the bundled sidecar — it reported "ffmpeg is
/// required" on exactly the machines the sidecar exists for.
pub fn ffmpeg_binary() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("ASTERISM_FFMPEG") {
        let path = PathBuf::from(explicit);
        return path.is_file().then_some(path);
    }
    if let Some(sidecar) = std::env::current_exe()
        .ok()
        .and_then(|exe| sidecar_beside(&exe))
    {
        return Some(sidecar);
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("ffmpeg");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/usr/bin/ffmpeg",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

/// The bundled-sidecar candidate for a given executable path: an
/// `ffmpeg` in the same directory (`Contents/MacOS/` in the bundle,
/// where Tauri's `externalBin` places it, triple suffix stripped).
/// Split from [`ffmpeg_binary`] so the beside-the-exe rule is
/// testable without planting files next to the real test binary.
fn sidecar_beside(exe: &std::path::Path) -> Option<PathBuf> {
    let candidate = exe.parent()?.join("ffmpeg");
    candidate.is_file().then_some(candidate)
}

/// Extracts one frame from the video at `path_str`, scaled to fit a
/// `target_px` box, and returns JPEG-encoded bytes — the same blob
/// shape the AVFoundation and ImageIO routes produce, so the cache,
/// the UI and the palette extractor cannot tell the sources apart.
///
/// Runs synchronously — call inside `spawn_blocking`, like the two
/// routes it sits beside.
pub fn make_thumb(path_str: &str, target_px: u32) -> Result<Vec<u8>, DomainError> {
    let Some(bin) = ffmpeg_binary() else {
        return Err(DomainError::Infra(anyhow::anyhow!(
            "{path_str}: this video format needs ffmpeg, which was not found — \
             install it (e.g. `brew install ffmpeg`) or point $ASTERISM_FFMPEG at a binary"
        )));
    };
    // Mirror thumb_video's frame choice: ~1 s in, past the fade-from-
    // black most recordings open with. A clip shorter than the seek
    // yields no frame at all, so retry at 0 rather than guessing the
    // duration first (that would cost an ffprobe run per thumbnail).
    let mut last_err = String::new();
    for seek in ["1", "0"] {
        let scale =
            format!("scale=w={target_px}:h={target_px}:force_original_aspect_ratio=decrease");
        let output = Command::new(&bin)
            .args(["-v", "error", "-ss", seek, "-i", path_str])
            .args(["-frames:v", "1", "-vf", &scale])
            .args(["-f", "image2pipe", "-c:v", "mjpeg", "-"])
            .output()
            .map_err(|e| {
                DomainError::Infra(anyhow::anyhow!("{path_str}: ffmpeg spawn failed: {e}"))
            })?;
        if output.status.success() && !output.stdout.is_empty() {
            return Ok(output.stdout);
        }
        last_err = String::from_utf8_lossy(&output.stderr).trim().to_string();
    }
    Err(DomainError::Infra(anyhow::anyhow!(
        "{path_str}: ffmpeg produced no frame: {last_err}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::material::KNOWN_VIDEO_MIMES;

    /// The drift tripwire. Every mime the extension table can produce
    /// must have a *deliberate* route — when a sixth format lands in
    /// `KNOWN_VIDEO_MIMES`, this match stops compiling over it
    /// conceptually and this test names the missing decision, instead
    /// of the format silently falling to AVFoundation and rendering
    /// as an empty tile.
    #[test]
    fn every_known_video_mime_has_a_deliberate_route() {
        for raw in KNOWN_VIDEO_MIMES {
            let parsed = MimeType::parse(raw);
            match *raw {
                "video/mp4" | "video/quicktime" => {
                    assert_eq!(route_for(Some(&parsed)), VideoThumbRoute::Native)
                }
                "video/webm" | "video/x-matroska" | "video/x-msvideo" => {
                    assert_eq!(route_for(Some(&parsed)), VideoThumbRoute::ExternalFfmpeg)
                }
                other => panic!(
                    "{other} is in KNOWN_VIDEO_MIMES but has no deliberate thumb route — \
                     extend route_for and this test together"
                ),
            }
        }
    }

    /// The sidecar rule is "an `ffmpeg` file beside the executable" —
    /// exercised against a fake exe path in a tempdir because the
    /// real one (`target/debug/deps/…`) must stay unpolluted: a
    /// planted `ffmpeg` there would outrank the host install for
    /// every other test in this binary.
    #[test]
    fn the_sidecar_candidate_is_the_ffmpeg_file_beside_the_exe() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let exe = tmp.path().join("Asterism");

        assert_eq!(sidecar_beside(&exe), None, "no sidecar file, no candidate");

        let sidecar = tmp.path().join("ffmpeg");
        std::fs::write(&sidecar, b"#!/bin/sh\n").expect("plant sidecar");
        assert_eq!(
            sidecar_beside(&exe),
            Some(sidecar),
            "an ffmpeg beside the exe is the bundled sidecar"
        );
    }

    #[test]
    fn an_unknown_or_missing_mime_stays_on_the_native_route() {
        assert_eq!(route_for(None), VideoThumbRoute::Native);
        // Parses into the video family without a named variant, and an
        // unnamed container is not assumed to need ffmpeg.
        assert_eq!(
            route_for(Some(&MimeType::parse("video/x-flv"))),
            VideoThumbRoute::Native
        );
    }

    /// End-to-end through a real ffmpeg: synthesise a 1-second WebM,
    /// extract a frame, get a JPEG back. Panics (with the install
    /// instruction) when ffmpeg is absent — this repo's fixture tests
    /// fail loudly rather than skip silently, because a skipped gate
    /// reads as a passed one.
    #[test]
    fn a_webm_yields_a_jpeg_frame_through_the_external_route() {
        let bin = ffmpeg_binary().expect(
            "ffmpeg is required for this test: brew install ffmpeg (or set $ASTERISM_FFMPEG)",
        );
        let tmp = tempfile::tempdir().expect("tempdir");
        let clip = tmp.path().join("test.webm");
        let status = Command::new(&bin)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1:size=160x120:rate=10",
            ])
            .arg(&clip)
            .status()
            .expect("synthesise webm");
        assert!(status.success(), "ffmpeg could not synthesise the fixture");

        let bytes = make_thumb(clip.to_str().unwrap(), 128).expect("frame extracted");
        assert!(
            bytes.starts_with(&[0xFF, 0xD8]),
            "the blob is a JPEG, same shape as every other thumbnail"
        );
    }
}
