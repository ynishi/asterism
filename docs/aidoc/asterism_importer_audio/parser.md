# asterism-importer-audio::parser

Audio `RawItem` → `Footprint::Audio` parser.

Metadata via `lofty` (pure-Rust, MIT). Covers MP3 / M4A (AAC in
MP4) / WAV / FLAC / OGG (Vorbis + Opus) plus WavPack / APE /
MPC / AIFF. Header-only reads — no decoding.

## Types

- `AudioParser` — Turns a scanned audio file into `Footprint::Audio`.

