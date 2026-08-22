# asterism-importer-image::parser

Image `RawItem` → `Footprint::Image` parser.

Reads EXIF from the payload with `kamadak-exif` (V2 design D-15 —
Rust-side, never JS-side). Falls back to file mtime when no EXIF is
present so PNG screenshots and freshly-generated images still land
with a meaningful `occurred_at`.

# One file, one footprint — including the text inside it

This parser used to read the PNG's `tEXt` chunks and emit one
`Footprint::Note` per chunk, each addressed `<image>.png#<keyword>`,
so a ComfyUI export arrived as N + 1 assets. It does not any more:
**an image file is one record, and the text inside it is that
record's metadata**. The chunks reach the image's own row on the
`Meta` axis, and
nothing on this side has to carry them there — the server's
`material_hash` job already reads the artefact's bytes and hands
them to the PNG probe, which writes `material.meta_hash` and
`material.meta_kv` on the row this parser produced.

So there is no hand-off to build here, and deliberately none to add:
a metadata set declared by the importer *and* computed from the
bytes by the server would be two authorities for one value, which is
the failure `declared_content_hash` is fenced against. The importer
states where the bytes are; what they contain is read from them.

## Types

- `ImageParser` — Parses one image file into one `Footprint::Image`.

