# asterism-infra::jobs::preview_ffmpeg

Preview-rendition transcode: an unplayable video in, an H.264 MP4
out.

The embedded webview cannot display VP9 at all and rejects the
Matroska container (measured 2026-07-31), and the
corpus this app is for — generation-tool output — emits VP9 WebM
by default. No delivery trick fixes a codec the engine will not
decode in the DOM, so the fix is the one video sites use: keep the
original untouched in the ledger and play a transcoded rendition.
H.264 + AAC in MP4 is the one combination every measured route
plays.

The rendition is a **cache**, not a copy of record: capped
resolution, disposable, regenerable from the original at any time
— the thumbnail relationship at video scale. It lives beside the
profile database (`<profile>/previews/<asset_id>.mp4`) so tests
that sandbox the database sandbox the renditions with it.

Like `thumb_ffmpeg`, this shells out to an installed ffmpeg
binary; when none is present the job fails naming the fix instead
of leaving a silent crossed-out player.

## Functions

- `make_preview` — Transcodes `src` into `dest` (H.264 + AAC MP4, capped to

## Constants

- `PREVIEW_MAX_EDGE` — Box the rendition fits in (longer edge, pixels). Preview quality

