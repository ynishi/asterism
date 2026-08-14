# asterism-infra::probes::png

PNG's reading of the two walking axes: which of its chunks are the
picture, and which are notes written about it.

The chunk framing — where one chunk ends and the next begins, what a
`tEXt` payload decodes to — is
[`asterism_media_probe::png`](asterism_media_probe::png), and it has
no opinion about any of this. What is here is the opinion: a
judgement about *this* corpus, argued from measurements taken in this
repo, which is why it sits beside the application rather than in a
crate that walks PNGs in general.

# From a slice, and what that costs the caller

Both readings take `&[u8]`: the caller already holds the bytes.
`ContentHasher` streams for a reason — an original can be a 4 GB
video and reading it into memory would be the process's largest
allocation — and that reason still stands, but it does not reach
here, for two reasons.

The first is the region's shape. The pixel stream is fed last, as one
concatenation, so that the same compressed data split across a
different number of `IDAT` chunks hashes the same (see below). A
reader would therefore have to either hold every `IDAT` payload in
memory until the end — the allocation we were avoiding, in a worse
form, since it is the largest part of the file — or seek back and
read the file a second time. A slice makes the second pass free: it
is a second walk over memory the caller already paid for.

The second is the corpus. This is the PNG probe; PNGs here run to a
few megabytes (a real character card is a few hundred KB), and the 4 GB
case is video, which needs a completely different reading (mp4 sample
tables) that will take a reader rather than a slice. Sizing this
probe's signature against a file type it does not handle would buy
nothing and cost the second pass.

So: **a 4 GB PNG would be a 4 GB allocation in the caller**, before
either method is entered. That decision belongs to the job that opens
the file — it is the one that knows the file's size before reading
it, and the one that can answer "too big to fingerprint" by storing
[`TOO_LARGE`](asterism_core::domain::content_region::TOO_LARGE). This
module never opens anything, so it cannot make that call; what it
guarantees is that once the bytes are in hand it adds no allocation
proportional to them.

# Which chunks are content

Everything except metadata. The excluded set is `tEXt`, `zTXt`,
`iTXt`, `tIME`, `eXIf`, plus the structural `IEND`.

A denylist, not an allowlist of the chunks known to matter, and the
reason is the one written into the port's own contract
([`ArtefactProbe::content_of`]): the two are wrong in opposite
directions. An allowlist drops any chunk nobody thought of: a private
chunk, a type added to the spec later, or — measured, not imagined —
the colour-management chunks and APNG frame data, where two visibly
different pictures came out with one digest. That error is a false
positive, and downstream a fold turns the loser into a tombstone,
which is not something the user can undo by looking at it again. A
denylist's error runs the other way: forget to exclude something and
two files that differ only in metadata get separate digests, which is
exactly what happens with no content axis at all. One failure loses
data, the other loses an improvement, so the unknown chunk goes on
the side that loses the improvement.

# What is fed to the hash

Every included chunk contributes `type (4 bytes) || payload`, in the
order it appears; all `IDAT` payloads are concatenated and fed once,
last, behind a single `IDAT` tag.

**The chunk's length field is never fed.** Encoders split the same
compressed stream at different boundaries — zlib's buffer size is not
part of the image — and hashing the lengths would make one picture
written by two encoders two pictures. Measured: the same stream in 1,
8 and 63 chunks produces one digest, and a real ComfyUI corpus writes
its pixels as 17–24 chunks of 64 KiB, so this is the ordinary case
rather than an adversarial one.

The type *is* fed, because a payload alone does not say which chunk
it was: two files carrying the same four bytes under different chunk
types would otherwise collide.

Neither the order nor the presence of a chunk is assumed. The real
fixture in this repo carries its `tEXt` chunks **after** the pixel
data; a ComfyUI export carries one on each side (the prompt before,
the workflow after), and files in the same corpus have the second one
missing entirely. Anything that treated "metadata comes first", or
"these two chunks are always there", would break on ordinary files.
Selection is by type and nothing else.

# Which chunks are metadata

Only `tEXt` is read. It is the chunk this corpus's metadata actually
travels in, and it is the one the reader that defines the form
(`asterism_media_probe::png::text_fields`, a `BTreeMap<String,
String>` keyed by chunk keyword) returns. `zTXt` is compressed and
`iTXt` may be; `tIME` and `eXIf` are binary, not text. Reading them
means deciding how each decodes to a string *before* the digest means
anything, and a decision made carelessly there is a redefinition of
the axis, not a widening of it. They are excluded from the content
region as well, so **neither digest is about them** — a stated gap
rather than a silent one, and one that a `m2-` generation would
close.

# Which chunks are kept

All five, as bytes. The gap above stays a gap in what is *hashed*,
and stops being a loss: [`meta_raw_of`](PngProbe::meta_raw_of) keeps
the frames of every chunk in [`METADATA_CHUNKS`] verbatim, so a later
generation can decide how a `zTXt` decompresses, or read the Latin-1
bytes an accented `tEXt` lost to `from_utf8_lossy`, **without opening
the file again** ([`material_meta_raw`](asterism_core::domain::material_meta_raw)).

The list is the same list on purpose. What the content digest drops
and what the raw keeps are one sentence about this container — the
bytes that are notes rather than picture — and stating it twice is
how the two stop agreeing. So "widen the denylist" and "keep more"
are one edit, and a chunk added to the five is excluded from the
digest *and* recoverable from the row that day.

# Every structural defect is one outcome

The walk distinguishes truncation from a lying length from a missing
`IEND` from too many chunks, and all four land on
[`ContentRegion::EmptySpan`]. The variants are worth having anyway —
a reader of a stack trace or a future diagnostic can tell which
happened — but the stored value must not fork on them, because the
true statement they share is the one the column carries: there is no
complete region to stand behind. Splitting them into separate markers
was argued down where the marker is defined
([`EMPTY_SPAN`](asterism_core::domain::content_region::EMPTY_SPAN)),
and doing it here instead would put the same vocabulary in two
places.

## Types

- `PngProbe` — PNG's reading of the content and meta axes.

