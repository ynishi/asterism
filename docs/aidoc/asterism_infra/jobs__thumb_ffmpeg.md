# asterism-infra::jobs::thumb_ffmpeg

Video frame extraction through an external `ffmpeg`, for the
formats AVFoundation cannot open.

macOS ships no WebM / Matroska / AVI demuxer, so before this path
existed a `.webm` clip kept an empty tile forever — the thumb job
failed softly and the grid showed nothing (journal 2026-07-27, the
"webm mkv avi" carry). Bundling an ffmpeg *library* was rejected
back then for its build cost, and still is; this module instead
shells out to an ffmpeg *binary* when one is installed, and fails
with an instruction naming the fix when none is.

# Routing, and the drift this module closes

[`route_for`] is the single answer to "which extractor can turn
this video mime into a frame". The extension→mime table
(`asterism_core::domain::material`) and the extractors' real
capabilities used to be two lists growing independently — a format
added to the table without an extractor showed up as an empty
tile, not an error. The test at the bottom walks
`KNOWN_VIDEO_MIMES` and fails the moment an entry has no
deliberate route, so the two lists can no longer drift silently.

# Why the binary is probed at fixed paths too

A GUI app launched through LaunchServices inherits a minimal
`PATH` (`/usr/bin:/bin:…`) that does not contain Homebrew's
prefix, so `Command::new("ffmpeg")` alone would report "not
installed" on exactly the machines that have it. The probe order
is: `$ASTERISM_FFMPEG` (explicit override) → the bundled sidecar
beside the executable → `PATH` → the usual install prefixes.

The sidecar is an LGPL-clean ffmpeg (no libx264 — H.264 encoding
rides `h264_videotoolbox`) that `tauri build` copies to
`Asterism.app/Contents/MacOS/ffmpeg` via `bundle.externalBin`;
`scripts/build-ffmpeg-sidecar.sh` produces it. It outranks `PATH`
so the app prefers the binary it shipped and was tested with over
whatever the host happens to carry, and a clean machine (no
Homebrew) plays video out of the box.

## Functions

- `ffmpeg_binary` — Locates an ffmpeg binary, or says how to get one. Shared with the
- `make_thumb` — Extracts one frame from the video at `path_str`, scaled to fit a
- `route_for` — Picks the extraction route for a video mime.

## Types

- `VideoThumbRoute` — Which extractor owns a video mime.

