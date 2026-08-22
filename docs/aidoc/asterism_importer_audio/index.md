# asterism-importer-audio 0.0.0

Audio import adapter — turns a scanned audio file into
`Footprint::Audio`.

One of the per-modality adapters behind the unified
`asterism-importer` CLI: [`parser::AudioParser`] implements the
importer SDK's `SourceParser`, the SDK pipeline walks the source and
pushes the resulting footprints to a running `asterism-server` over
HTTP. The format specifics live in [`parser`]; this crate root only
re-exports the parser type.

## Modules

- [`parser`](parser.md): Audio `RawItem` → `Footprint::Audio` parser.

