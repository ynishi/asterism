# asterism-core::domain::embedded_text

`embedded_text` — the words a container wrote *into* an artefact,
recovered for search rather than for identity.

# Why this is not [`material_meta`](crate::domain::material_meta)

The two read the same chunks off the same bytes and answer different
questions, and the difference is the reason both exist.

`material_meta` defines a **digest**. Its reading of a file has to be
total and frozen: two equal metadata sets must render identically or
the axis stops grouping, so that module fixes one decoding, reads
`tEXt` and nothing else, and says so — "a different answer is a `m2-`
generation rather than an edit". Widening it would silently redefine
what every stored `meta_hash` meant.

This module defines a **document**. Nothing downstream compares two
of its outputs for equality; the output is tokenised and thrown into
a haystack. So it can be generous where the digest cannot:

- **`zTXt` and `iTXt` are read.** Compressed and international text
  were excluded from both stored axes — "a stated gap rather than a
  silent one" — and the gap is only defensible for a digest. A
  caption a person can see in their image viewer and cannot find by
  searching for it is the whole complaint this answers.
- **Latin-1 is recovered rather than replaced.** `tEXt` is Latin-1
  by the spec and arbitrary bytes in practice, so the digest side
  reads it with [`String::from_utf8_lossy`], which turns every byte
  above 0x7F that is not part of a valid UTF-8 sequence into
  U+FFFD — a total function, which is what a digest needs, and a
  shredder for the accented words in `Café` or `Größe`. Here the
  bytes are tried as UTF-8 first (what generators actually write,
  spec or no spec) and read as Latin-1 when that fails, so no byte
  is lost either way.
- **A malformed tail keeps what came before it.** The digest side
  refuses a chunk sequence that never reaches `IEND`, because what
  it collected is part of a file rather than a file. A truncated
  file's caption is still a caption, so the walk stops and keeps.

# PNG only

The same bound its two siblings carry. EXIF, XMP and ID3 all hold
words about their artefact and none of them is read here — the
recovery is per-container-format work, and this is the format the
corpus's text actually travels in. [`walks_format`] is what a caller
asks before spending a read.

# Where the bytes come from

From the caller, as a slice, and only ever from the pass that is
already holding them: `fingerprint::hash_artefact` reads an artefact
once and answers every axis off that one buffer, and this is a third
walk over memory already paid for. Nothing on the indexing path
opens a file for this — the recovered text is stored on the material
(`material.meta_text`), and a job that re-composes a document reads
the column.

## Functions

- `recover` — Recovers every text annotation an artefact carries, keyed by the
- `render` — Renders recovered fields the way the column stores them.
- `walks_format` — Whether a declared format has a recovery walk here — the question a

