# asterism-importer-video::parser

Video `RawItem` → `Footprint::Video` parser.

Two probe layers, tried in order: `mp4parse` for ISOBMFF (MP4 /
MOV), then `matroska` for EBML (WebM / MKV). Each rejects the
other's container at the first magic bytes, so the order is cost
only, not correctness. A container neither probe reads (AVI) still
lands — as a footprint without dims / duration / codec, which is
the contract for every probe miss.

## Types

- `VideoParser` — Turns a scanned video file into `Footprint::Video`.

