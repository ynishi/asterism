# asterism-importer-tape 0.0.0

Tape import adapter — turns an exported conversation tape file into
`Footprint::Tape`.

One of the per-modality adapters behind the unified
`asterism-importer` CLI: [`parser::TapeParser`] implements the
importer SDK's `SourceParser`, the SDK pipeline walks the source and
pushes the resulting footprints to a running `asterism-server` over
HTTP. Tape is a non-Dialog modality — the file stem, not the
contents, drives identity; the details live in [`parser`].

## Modules

- [`parser`](parser.md): Persona Tape parser — one terminal transcript file becomes one document.

