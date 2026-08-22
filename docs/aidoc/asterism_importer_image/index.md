# asterism-importer-image 0.0.0

Image import adapter — parses one image file into one
`Footprint::Image`.

One of the per-modality adapters behind the unified
`asterism-importer` CLI: [`parser::ImageParser`] implements the
importer SDK's `SourceParser`, the SDK pipeline walks the source and
pushes the resulting footprints to a running `asterism-server` over
HTTP. The format specifics live in [`parser`]; this crate root only
re-exports the parser type.

## Modules

- [`parser`](parser.md): Image `RawItem` → `Footprint::Image` parser.

