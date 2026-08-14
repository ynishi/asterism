# asterism-media-probe 0.0.0

Header-only media measurement: what the bytes say about themselves.

Every function here is a pure function of a byte slice, and none of
them decodes pixels — they read far enough into a container to expose
what it declares and stop. That is the whole contract, and it is what
lets the same code run in an importer walking a directory and in a
server job walking a table.

# Why this is a crate and not a helper in one of its callers

Two paths write the same columns and must not disagree:

- **Ingest.** `asterism-importer-image` / `-video` measure a file as
  it arrives and put the result on a `Footprint`.
- **Backfill.** The server measures rows that predate the columns
  (`asset.width_px` / `height_px` landed in schema V69; every row
  older than it carries `NULL`).

The importers run as separate processes and talk to the server over
HTTP, so neither can call the other. Before this crate the only ways
to give the server a measurement were to write a second
implementation of it, or to have the server link an importer binary's
crate. The first puts two definitions of "the dimensions of these
bytes" in the tree, told apart by nothing once both have written to
the same column; the second inverts the out-of-process design that
makes importers replaceable.

# Coded, not displayed

**[`coded_dims`] returns the dimensions of the stored byte stream,
before any orientation is applied.** A photo shot upright with EXIF
Orientation 6 is *stored* as a landscape frame plus a rotation flag,
and that is the pair returned here — the flag is reported separately
([`ExifFields::orientation`]) and applying it is the caller's
decision. Video is weaker still: neither container probe reads the
MP4 display matrix or Matroska's `DisplayWidth` / `DisplayHeight`,
so an upright phone clip measures 1920×1080 and nothing in the
returned value says otherwise.

The consequence is worth stating where the measurement is defined
rather than only where it is consumed: **the product of the pair is
invariant under that rotation and the sides are not**, which is why
Asterism's resolution facet is a pixel count rather than a width
band. A caller that compares widths is comparing storage layout.

# Container structure, beside container headers

[`png`] is the same contract one level down: a pure function of a
byte slice that reads a container's framing — where each chunk
begins, what the text chunks say — and stops. It is here for the
reason the dimension probes are: a second caller appeared. The
server's fingerprint pass walks a PNG's chunks to decide which bytes
are the picture and which are notes written about it, and that walk
used to live in `asterism-core::domain`, where every new format made
the domain layer wider. **The judgement stayed there and the parsing
came here** — this module reports what the bytes are and never what
they are worth, which is why it can be read by a consumer that has no
opinion about identity at all.

[`jpeg`] is the second one, on the same terms, and the pair is what
says the terms were terms rather than one module's habits. It is not
shaped like [`png`]: a JPEG is marker segments up to its scan,
unframed entropy-coded bytes after it, and whatever a phone appended
behind its end marker, so the walk yields four kinds of element where
the PNG walk yields one. What the two share is the
contract — boundaries out, judgement left to the caller — and that is
the part worth having twice.

