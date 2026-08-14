# asterism-infra::jobs::thumb_macos

ImageIO fast path for `thumb_gen` on macOS.

`CGImageSourceCreateThumbnailAtIndex` reads a JPEG (or HEIC / PNG),
resizes to a target longer edge, and returns a `CGImage` in one
call. On Apple Silicon this route flips to the hardware JPEG
decoder, keeping CPU load close to zero during a large import
wave — the pure-Rust `image` crate path (see `handlers::make_thumb`
fallback) burns a full core per decode even at `Lanczos3`.

The returned bytes are JPEG-encoded via `CGImageDestination` so
the DB blob shape stays identical across platforms.

## Functions

- `make_thumb` — Decodes the JPEG at `path_str`, resizes so the longer edge is

