# asterism-core::domain::material_meta_raw

`material_meta_raw` — the container's metadata bytes, kept verbatim,
so that the rule which expands them can be rewritten afterwards.

[`material_meta`](crate::domain::material_meta) renders what a probe
read into the canonical form and hashes it. This is the other half of
the same axis: the bytes that rendering was made *from*, stored as
they sat in the file. One is the answer, the other is the question it
was derived from.

# What the rendering loses, measured

`meta_kv` looks lossless for PNG — the `tEXt` values go into it as
text — and it is not, in two ways that were measured rather than
imagined:

- **`tEXt` is Latin-1 by the spec** and the reader takes it through
  [`String::from_utf8_lossy`], which is a total function with one
  answer per input and therefore the right thing for a digest. An
  accented byte becomes `\u{fffd}` and **the original byte is gone**
  (`asterism_media_probe::png::text_fields` states this where it
  happens).
- **`zTXt` / `iTXt` / `tIME` / `eXIf` are not read at all.** They are
  outside the content region and outside the meta digest, which the
  PNG probe's own doc calls a stated gap that an `m2-` generation
  would close.

Both are recoverable from the raw **without opening the file again**,
which is the whole of why this column is worth its bytes. Changing
how metadata is expanded is otherwise a re-read of somebody's entire
library, and that is a decision about their disk rather than a
consequence of shipping (the argument is written out at
[`needs_content_walk`](crate::domain::content_hash::needs_content_walk)).

# Why a second column and not another key

`meta_kv` **is** the digest's input — `canonical = render(fields)`,
`meta_hash = digest_of(canonical)` — so a key added there moves every
`m1-` value in the library, including the digests frozen as literals
in this workspace. The raw has to sit beside it or it redefines the
axis it exists to make revisable.

# The stored vocabulary

```text
undefined:<base64>          the bytes
unsupported:too-large       a probe read them and the policy declined to keep them
unsupported:not-captured    the build that read this row kept no bytes
NULL                        nothing here keeps bytes for this format
```

**`undefined:` is the point of the prefix.** A value under it carries
bytes and *no claim about what they mean*: it is not a digest and not
a rendering, and the expansion rule that would give it meaning is
exactly the thing this column exists to let somebody replace. A
prefix naming a reading (`png-chunks:`, `exif:`) would be that claim,
and the first reading to change would leave every row labelled with
the one it was written under. What the prefix does have to do is
separate a payload from a statement, so that a reader never takes
`unsupported:too-large` for content.

Base64 because the column is `TEXT` and the bytes are a container's,
which is to say arbitrary. The standard alphabet with canonical
padding, the same one [`series`](crate::domain::series) decodes a
character card with, so there is one answer in this workspace to what
base64 means.

# Which bytes those are is not decided here

Per format, by the probe that reads the container, on the same terms
as every other judgement about a corpus
([`ArtefactProbe`](crate::domain::probe::ArtefactProbe)). "How much of
this container counts as metadata" has no answer that is true of every
file, and a ceiling on how much of it is worth keeping is a statement
about one format's structure — JPEG's `APP1` cannot exceed 64 KiB
because its segment length is two bytes, and PNG has no equivalent
bound, so PNG's probe states one.

## Functions

- `bytes_of` — Reads a stored value back into the bytes it carries — **the whole

## Types

- `MetaRaw` — What a reading of a container's metadata bytes concluded.

## Constants

- `NOT_CAPTURED` — The value for a row a build that kept no bytes had already read.
- `RAW_PREFIX` — The prefix on a value that carries bytes rather than a statement

