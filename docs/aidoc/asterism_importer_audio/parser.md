# asterism-importer-audio::parser

Audio `RawItem` → `Footprint::Audio` parser.

Metadata via `lofty` (pure-Rust, MIT). Header-only reads — no
decoding. `codec_slug` is the list of what this names, and it is the
list rather than a copy of it here: what a container is called comes
back from that function, and a second enumeration in this header is
the one nobody edits when an arm is added.

## Types

- `AudioParser` — Turns a scanned audio file into `Footprint::Audio`.

