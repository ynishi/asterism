# asterism-infra::jobs::thumb_video

Video frame extraction for `thumb_gen` on macOS.

A video has no thumbnail without this: ImageIO (`thumb_macos`) and
the pure-Rust `image` crate both decode stills only, so before this
path existed a video asset simply showed an empty tile in the grid.
`AVAssetImageGenerator` pulls one frame as a `CGImage`, which
`thumb_macos::encode_jpeg` then turns into the same JPEG blob every
other thumbnail is stored as — the cache, the UI and the palette
extractor cannot tell the two sources apart, which is the point.

Format coverage is AVFoundation's, not the mime map's. `video/mp4`
and `video/quicktime` open natively; `video/webm` does not (macOS
ships no WebM demuxer), so those assets fail here and keep the
empty tile they had before. The failure is soft — one logged job
failure per enqueue, no retry storm (`jobs::mod` has no retry
policy, and the UI stops re-kicking after its own budget) — but it
means the extension→mime table in `domain::material` and this
extractor's real capabilities are two lists that can drift apart.

The generator's synchronous `copyCGImageAtTime` is deprecated in
favour of the async form. It is the right call here anyway: the
whole `thumb_gen` handler already runs inside `spawn_blocking`
under a decode-slot semaphore, so blocking is the contract this
path is called under, and an async completion handler would only
add a channel round-trip to get back to it.

## Functions

- `make_thumb` — Extracts one frame from the video at `path_str`, scaled to fit a

