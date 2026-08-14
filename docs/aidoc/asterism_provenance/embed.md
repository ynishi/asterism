# asterism-provenance::embed

Putting an XMP packet into a container, and taking the old one out.

Two containers, because those are the two still-image formats this
corpus arrives in. Both are handled as byte transforms over a whole
file rather than as a decode/re-encode: an export must hand back the
pixels it was given, and the only way to be sure of that is to never
decode them. It also means a PNG whose ancillary chunks nothing here
understands comes out with those chunks intact and in order.

# Replace, never append

Both containers permit a second XMP packet to sit next to the first,
and neither says which one wins. Readers disagree in practice, so a
file with two packets has a disclosure that depends on who opens it —
the failure mode being that a stale `digitalSourceType` shadows a
corrected one. Every writer here removes *every* packet it can see
before adding its own, and the tests pin that a file arriving with
two leaves with one.

What a writer can see is what its walk covers: a PNG chunk after
`IEND`, or a JPEG `APP1` after the scan, is neither collected nor
dropped. Those positions are also outside what [`read_xmp`] reads,
so no reader of this crate can be shown one — the promise is about
the part of the file both halves agree on rather than about every
byte present.

# Where the packet goes

Positions below are where a *new* packet lands. A file that already
carries one keeps it where it is: the first packet is replaced in
place, so a re-stamped file's chunk order does not drift with every
export.

PNG: an `iTXt` chunk before the first `IDAT`. The chunk is
uncompressed — the compression flag exists, and the packet is small
enough that a reader unable to inflate it would be a worse outcome
than a few hundred spare bytes.

JPEG: an `APP1` segment before the first non-`APPn` marker, so it
lands after a JFIF `APP0` and after an EXIF `APP1` rather than
displacing either. A metadata-only file that reaches `EOI` without
any such marker takes the position before the `EOI`, so the segment
is inside the image either way. (A *truncated* file has no `EOI`
either, and is refused as malformed rather than written into.)

# How much a packet may hold

Both containers have a ceiling and only one of them is reachable.
JPEG's `APP1` segment carries 65,533 bytes, of which the 29-byte XMP
identifier is one part, leaving the packet [`JPEG_MAX_PACKET`] — a
budget a generator prompt exhausts on its own, which is why
[`DisclosureRecord::essential`](crate::record::DisclosureRecord::essential)
exists and why [`stamp`] falls back to it. PNG's is the
specification's limit on a chunk's data field, about 2 GiB, which no
record approaches; it is checked rather than assumed because the
assumption used to be written as a silent truncation. Both refusals
are [`EmbedError::PacketTooLarge`].

## Functions

- `embed_xmp` — Writes `packet` into `bytes`, replacing any packet already there.
- `read_xmp` — Reads back the XMP packet a file carries, if it carries one.
- `sniff` — Identifies the container from its own first bytes.
- `stamp` — Convenience: renders the record and writes it, doing nothing when

## Types

- `Container` — A still-image container this module can write into.
- `EmbedError` — What went wrong writing a packet.

## Constants

- `JPEG_MAX_PACKET` — Largest XMP packet a JPEG can carry in one segment.

