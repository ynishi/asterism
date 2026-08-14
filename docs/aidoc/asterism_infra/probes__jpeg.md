# asterism-infra::probes::jpeg

JPEG's reading of the content axis: which of its segments are the
picture, and which are notes written about it.

The segment framing — where one marker segment ends and the next
begins, how far the entropy-coded data runs — is
[`asterism_media_probe::jpeg`](asterism_media_probe::jpeg), and it has
no opinion about any of this. What is here is the opinion, on the same
terms as [`png`](super::png): a judgement about *this* corpus, which
is why it sits beside the application rather than in a crate that
walks JPEGs in general.

# Both axes, and what the second one took to arrive

This probe declares `image/jpeg` on **both** axes ([`CLAIMS`]). It
arrived on the content axis alone, and the meta axis was declined for
a stated reason rather than left unwritten: a meta digest over EXIF
groups photographs that share a camera body, a lens and a second of
wall-clock time — frames 3 and 4 of a burst, with the fields that
separate them stripped by whatever exported them. The duplicate panel
is driven by these columns, so a definition that admits that fills
the panel with pairs a person has to refuse one at a time. What that
argument concluded was that **the reading worth having is a narrower
one, over fields chosen for the purpose, and choosing them is its own
slice.**

That slice is the series axis
([`series`](asterism_core::domain::series)), and it landed. A
[`Strategy`](asterism_core::domain::series::Strategy) is a registered
rule that reads `meta_kv` and derives its own key, so *which* EXIF
fields decide that two photographs were made the same way is a value
somebody registers and edits rather than a definition compiled into
this file.

**That answers half of the objection, and the other half is a live
defect.** Claiming this axis does two things at once, and only one of
them is the Series substrate: `image/jpeg` also enters the **meta
duplicate axis**, because the column it fills is one of the three
[`detect_duplicate`](asterism_core::application_support::duplicate_detection::detect_duplicate)
walks. A JPEG pair whose blocks agree can now stop there, and a
`duplicate_conflict` row is a question with a fold button on it —
which is the panel behaviour the burst argument was about, arriving
by a different door than it expected.

What lands there is not a duplicate, and that is the defect rather
than a mitigation. The walk stops at the *first* agreement, so a pair
reaches `Meta` only when `Artefact` and `Content` both found
nothing — **the pictures differ**. So a meta-alone agreement is not
an identity claim at all; it is "made the same way", which is the
sentence [`series`](asterism_core::domain::series) exists to say and
deliberately says without folding anything.

The root of it is one axis older than this slice: the algebra is
`Artefact = Content + Meta` — the whole bytes are the picture plus
the metadata — so there are **two** independent axes and `Artefact`
is the name for both agreeing, while
[`STRONGEST_FIRST`](asterism_core::domain::duplicate_conflict::DuplicateAxis::STRONGEST_FIRST)
lists three in parallel. **This build still does that**; correcting
it is its own slice, and nothing here anticipates the correction.

**Nothing is selected here, deliberately.** Every field the block
states goes into `meta_kv`, including the ones a burst rewrites and
the ones nobody has a use for, because narrowing on this side would
decide for every Strategy at once what an author is allowed to
address — the same argument
[`asterism_media_probe::exif_tags`] makes for refusing to select one
layer further down, and the reason
[`series`](asterism_core::domain::series) offers `exclude` beside
`include`.

## Nobody has published the axis a narrower reading would need

"Keep the fields a burst does not rewrite" sounds like a lookup, and
it was researched as one before this axis was claimed. A published
classification of every standard tag does exist — **Exif 3.0
Annex H**, the guidelines for handling tag information in
post-processing by application software (CIPA DC-008; the current
edition is 3.1) — and it gives each tag two labels. A **Category**:
I image structure, II shooting conditions, III the rest (model,
serial number, capture time, place, owner, copyright). And a
**Rank**: `Update 0` (update on every edit), `Update 1` (may be
updated on its own), `Freeze 0` (shall not be deleted or modified
under any circumstance), `Freeze 1` (needs no update), `Freeze 2`
(may be corrected where wrong). Two rules bind the pair: every
Category I tag is `Update 0`, and Category II — the shooting
settings — is `Freeze 1`.

**It answers a different question.** Annex H's axis is *may a tool
rewrite this*; the axis a series needs is *does this change from one
exposure to the next*. They correlate, and they come apart in one
direction systematically:

| Rank | tags | changes across a burst? |
|---|---|---|
| `Update 0` | `DateTime` (`0x0132`), `SubSecTime`, image structure | yes — the one part usable as a quotation |
| `Freeze 0` | `ImageUniqueID` (`0xa420`), alone | frozen *and* per-exposure |
| `Freeze 2` | `DateTimeOriginal` (`0x9003`), the GPS tags, the lens tags, `CameraFirmware` | the capture time does; the lens and the firmware do not |
| `Freeze 1` | `Make`, `Model`, `BodySerialNumber`, **and exposure time, aperture, ISO, focal length** | the body does not; under auto-exposure the exposure tags change frame to frame |

So **reading `Freeze 1` / `Freeze 2` as "steady across a burst" is
this project's judgement rather than a citation**, and it is wrong
exactly where auto-exposure is working. Excluding `Update 0` **is** a
citation: the specification says an editor rewrites those fields, so
dropping them is following a rule somebody else wrote. The
specification illustrates the divergence itself — a subject position
is Category II and `Freeze 2` as a subject, and is reclassified
Category I/II and `Update 0` when it is expressed relatively, because
resizing the picture moves it.

**No other published schema closes the gap**, which is itself the
finding rather than a gap in the search. MWG 2.0 reconciles one
field across containers (Exif ↔ IIM ↔ XMP) and has no stability axis
— and the group has not existed since 2018. IPTC's Photo Metadata
TechReference defines properties, machine-readably and openly
licensed, but defines no classification. XMP's `xmpMM`
(ISO 16684-1) versions a *document* — `DocumentID`, `InstanceID`,
`OriginalDocumentID` — which separates two edits of one photograph
and says nothing about two frames of one burst. C2PA asserts
provenance, and its hard bindings are *designed* not to match a
derivative or a related frame, which is the opposite direction.
Somebody has to make the judgement, so it is made by the author of a
[`Strategy`](asterism_core::domain::series::Strategy) — see
[`Decode::Exif`](asterism_core::domain::series::Decode::Exif) for
what that author is told, and this module for why the probe hands
them everything to choose from.

Sources: <https://www.cipa.jp/e/std/std-sec.html>;
<https://archive.org/details/exif-specs-3.0-dc-008-translation-2023-e>
(Annex H, pp. 233–241). CIPA's own PDF is encrypted, so the ranks
above were read off the Internet Archive's OCR of the 2023 English
translation: the rank tokens come through cleanly, the Category
roman numerals do not always, which is why no per-tag Category is
quoted here.

## The one identifier the specification insists on is the one nobody trusts

`ImageUniqueID` (`0xa420`) is what a reader reaches for first, and
the specification is at its most emphatic about it: `Freeze 0`, the
only tag carrying that rank, *shall not be deleted or modified under
any circumstance*. **Two independent implementations record that it
is not written reliably.** One validates the value's shape before
trusting it, because cameras were found writing their own model name
into the field; the other declined the tag outright as missing,
reused and inconsistent across vendors. So the tag is read like any
other — it lands in `meta_kv` and a Strategy may name it — but
nothing here treats it as an identity on its own.

Neither is a standard, and the same holds for every other grouping
rule shipped in this space: they are timestamp windows, filename
stems and capture intervals, and at least one vendor has had to turn
one of them **off by default** after unrelated photographs
downloaded together were grouped by a shared prefix. That is the
merge-is-unrecoverable asymmetry this file argues from, observed in
production by somebody else, and it is why the axis a Strategy reads
is data rather than a rule compiled in here.

Which products those are, and what each one groups on, is not
written here on purpose. A per-vendor survey in a module doc is read
once and then goes stale silently, and it is not what this file has
to decide — the decision is the sentence above, that no published
rule can be quoted for this axis, so the axis is data.

## What lands in `meta_kv`, and what a key may not carry

[`exif_tags`](asterism_media_probe::exif_tags) produces the map
whole: `exif:0x829a` → `rational:1/125`, the key an address and the
value self-describing. The two properties this file leans on are
argued there — the IFD is in the key because one tag number means
different things in different IFDs, and the type is on the *value*
because `1/125` is otherwise ambiguous between a rational and an
ASCII tag whose text is literally that.

## The block's bytes are kept as well

[`meta_raw_of`](JpegProbe::meta_raw_of) keeps the `APP1` payload
verbatim, which is what makes the rendering above revisable: `MakerNote`
is one opaque `undefined` value today, and Apple's `BurstUUID` — the
signal every other library reaches for — is inside it. A reader that
learns to open it later works from the column
([`material_meta_raw`](asterism_core::domain::material_meta_raw))
rather than from somebody's disk.

# Which segments are content

Everything except three, named one at a time:

- `APP1` whose payload begins `Exif\0\0` — the EXIF block. **One
  field of it comes back in by another door**; see below.
- `APP1` whose payload begins `http://ns.adobe.com/xap/1.0/\0` *and
  does not carry the bytes `tiff:Orientation` anywhere in it* — an
  XMP packet. `APP1` is shared by both and by anything else a vendor
  puts there, so the marker alone does not identify either; the
  payload's own identifier does, and an `APP1` carrying neither is
  **content**.
- `COM` (`0xFFFE`) — the comment segment.

…and one thing that is in the region without being a segment: the
orientation the file renders under, fed as its own element.

A denylist, not an allowlist of the segments known to matter, for the
reason written into the port's contract
([`ArtefactProbe::content_of`]): the two are wrong in opposite
directions, and not by a little. An allowlist drops whatever nobody
thought of, and saying "the same" about two artefacts that are not is
a loss no later correction can undo — downstream a fold turns the
loser of a duplicate group into a tombstone. A denylist's error runs
the other way: a metadata segment nobody remembered is hashed, two
files differing only in it get two digests, and that is exactly the
state of a format with no content axis at all.

## The criterion is rendering, and EXIF Orientation meets it

The denylist has a *criterion* behind it, and the criterion is not
"is this segment metadata" — it is **does this change what a person
is shown**. A rule that excludes by segment name is a shortcut
through that question, and it works right up until a segment carries
both kinds of byte at once. EXIF is that segment.

Orientation codes `2`–`4` mirror the frame and `5`–`8` transpose it,
and this repository already treats that as display-affecting rather
than as trivia: the stored dimensions are documented as *coded*, with
the caller told to combine them with the orientation to get a
displayed shape
([`AssetDto::width_px`](asterism_contract::dto::AssetDto::width_px)),
the image importer carries the code onto the asset's `extra`, and the
detail pane prints it. So the failure was concrete rather than
theoretical: a phone photograph tagged `Orientation=6` and the same
file through any EXIF stripper have **byte-identical entropy-coded
data**. Under a wholesale exclusion they take one digest, land in one
duplicate group, and a fold turns the row that displayed the right
way up into a tombstone. After the fold there is one picture, so
there is nothing left for a person to compare and notice — the
unrecoverable direction, reached through the door the denylist was
built to hold shut.

The answer is not to stop excluding EXIF. Nearly all of it *is*
notes — timestamps, camera bodies, GPS, exposure — and excluding it
is the reason this axis exists. What the region takes is the one
field, normalised, on its own:

- **Normalised**, because absent, unreadable and `1` all render
  identically, so all three feed the same value. A file that never
  had an orientation and a file explicitly tagged upright are one
  picture, and a probe that could tell them apart would be reporting
  on the EXIF block again.
- **In a fixed position**, ahead of every segment, because *where*
  the `APP1` sat is a fact about the writer and not about the
  picture. Two files whose EXIF blocks are at different offsets, or
  one of which has none at all, still agree.

It costs one header-region EXIF parse per content walk
([`jpeg::orientation`](asterism_media_probe::jpeg::orientation)),
which is the same read the importer already does at ingest. Reading
the tag is `asterism-media-probe`'s (a fact about the format);
deciding it belongs in a digest is this file's (a judgement about a
corpus), which is the same line every other rule here is drawn on.

`tests::two_photographs_differing_only_in_their_orientation_are_two_pictures`
keeps it measured rather than asserted, the way the `APP2` case
below does: it runs a second walk with EXIF excluded whole and shows
the phone photograph and its stripped export collapsing onto one
digest under it.

### XMP carries it too, and is not parsed for it

`tiff:Orientation` can appear in an XMP packet. Reading XMP would
mean an XML parser deciding what a picture is, so instead the
**exclusion is withdrawn**: an XMP packet with those bytes anywhere
in it stays in the region, in full. The cost is an improvement lost
— two files differing only in unrelated XMP that happens to mention
the property get two digests — and that is the direction this whole
selection is allowed to fail in.

### What is still excluded and might have mattered

Stated rather than implied, because the criterion above does not
stop at one tag. `Gamma`, `ColorSpace`, and the TIFF colour
description tags (`WhitePoint`, `PrimaryChromaticities`,
`TransferFunction`) can all change a rendering, and all of them are
inside the excluded `APP1` today. Two files differing only in one of
those still take one digest.

That is a smaller risk than the one being fixed, on this corpus, and
saying why is the point: those tags are rare, they are usually
redundant beside an `APP2` ICC profile that **is** in the region, and
a difference in them is subtle where a transposed photograph is
obvious at arm's length. Orientation is the one tag that is on nearly
every photograph a phone takes, that nearly every export pipeline
strips, and that this repo had already decided is display-affecting.
It earned the machinery; the rest have not yet, and the way they
would is somebody measuring a pair on this corpus.

## `APP2` is content, and this is the segment that says why

**The ICC colour profile is inside the region.** Two files whose
pixels agree byte for byte and whose profiles differ are two
pictures: the profile is what the same numbers are rendered *as*, so
excluding it merges an sRGB export with a Display-P3 one and a viewer
shows the difference.

That is not a hypothetical borrowed from a specification. It is the
failure the PNG probe measured on this repo's own corpus, in the same
place — colour-management chunks and APNG frame data, where two
visibly different pictures came out with one digest — and the shape
of the mistake there was exactly this one: a rule that looked like it
was excluding metadata was excluding rendering.
`tests::an_icc_profile_is_part_of_the_picture` keeps it measured
rather than asserted, by running a second walk with `APP2` on the
excluded side and showing that the two files collapse onto one
digest under it.

Everything else stays in: `APP0` (JFIF), `APP13` (Photoshop's image
resource block), every other `APPn`, extended XMP — which announces
itself with a *different* identifier and is therefore not one of the
two `APP1` cases above — the coding tables (`DQT`, `DHT`, `SOF*`,
`DRI`), the `SOS` headers, the entropy-coded data, the restart
markers inside it, the closing `EOI`, and whatever the file carries
behind it. When a segment is arguable, it goes here: that error costs
a duplicate nobody spots.

## The bytes after `EOI` are content, and PNG's are not

`EOI` ends the image and does not end the file. **Google and Samsung
Motion Photos are a complete JPEG with a complete MP4 appended**, and
that is how a phone ships a still and its couple of seconds of video
as one artefact — a default, not a curiosity. Excluding the tail
makes a Motion Photo and its still-only export one digest: measured,
a 598-byte still and the same still with 4 KB appended came out
identical. One duplicate group, one fold, and the video is gone with
the row that carried it.

So the trailing span is in the region, as one element, fed whole and
unread — the walk has no grammar for what follows an image and does
not need one to know the bytes are there.

**[`png`](super::png) excludes what follows its `IEND` and stays that
way**, which is a decision rather than an inconsistency the two
probes have not got round to reconciling. The question is not what
the container permits — both permit a trailing payload — but what
this corpus's files do with it. A PNG carrying something meaningful
after `IEND` is rare enough that the PNG probe's own fixture had to
construct one; a JPEG carrying something meaningful there is what a
phone hands you. Where the format's *use* differs, the region
definitions differ with it, and copying either shape onto the other
would be copying past the reasoning again.

# What is fed to the hash

The normalised orientation first, as one byte, before anything the
walk yields — the fixed position argued for above. Then, in the order
the file carries them:

- a framed segment: `marker (1 byte) || length (2 bytes,
  big-endian) || payload`;
- a marker with no length field: its single byte;
- entropy-coded data: `length (8 bytes, big-endian) || bytes`, the
  bytes exactly as the file carries them, stuffing and restart
  markers included;
- the trailing span: `length (8 bytes, big-endian) || bytes`.

`SOI` is not fed. It is the signature, it was checked to get here,
and it is the same two bytes in every JPEG, so feeding it would
separate nothing from nothing.

## Every element carries its own length, including the ones with no
length field

A framed segment's length is in the file and gets fed (the next
section argues that against PNG, which omits its own). A scan's
length is *not* in the file — entropy-coded data is delimited by the
marker that follows it — and it is fed anyway, computed, for the
identical reason: **without it the region is one run of bytes with no
seam in it, and a scan can spell out the element behind it.**

Measured, before this was fixed: a scan ending `0x11 0xE4 0x00 0x04
0xAA 0xBB` followed by `EOI`, against a scan ending `0x11` followed
by an `APP4` carrying `0xAA 0xBB` and then `EOI`. Two entropy streams
differing by five bytes, one digest. The `0xFF` that introduces every
marker is not fed either — it is in every marker and separates
nothing — so nothing in the feed marked the boundary at all.
`tests::the_scans_length_separates_it_from_the_segment_behind_it` is
that pair.

Eight bytes rather than the file's two, because the number being
described is not a segment's: a scan is as long as the picture is,
and the trailing span can be a whole video. A scan of no bytes is fed
as a length of zero rather than skipped, so the rule has no exception
in it.

With that, the feed is *decodable*: read a byte, and whether it is a
bare marker or the head of a framed one is a property of the byte
alone; after an `SOS` comes a scan's length; after `EOI`, a trailing
length if anything is left. Two different element sequences cannot
produce one stream — which is the property "no collisions from
re-division" actually names, and it is what makes the claim checkable
instead of a hope.

## The length **is** fed, and PNG's is not

The second of the two places these probes disagree about the shape of
an answer (the trailing span was the first), so it is worth being
explicit about why, and about why the difference is not an
inconsistency to be tidied away later.

PNG omits the chunk length because an encoder is free to cut one
compressed stream into any number of `IDAT` chunks — zlib's buffer
size is not part of the image — and hashing the lengths would make
one picture written by two encoders two pictures. That was measured:
the same stream in 1, 8 and 63 chunks produces one digest, and a real
ComfyUI corpus writes its pixels as 17–24 chunks of 64 KiB.

**A JPEG segment's length is not that kind of number.** It is fixed
by the segment's own contents — a quantisation table is as long as a
quantisation table — and there is no re-cutting for it to absorb,
because the one unbounded run in the container (the scan) is not
length-prefixed at all. So PNG's reason does not carry over, and with
no reason to drop it the length goes in, because feeding it removes a
class of collision: without it, `marker || payload` runs together, and
two adjacent segments of one marker can be re-divided into two
different segments of the same marker with the same concatenation.

The general form of the rule, for whoever adds the next format:
**copy the reasoning, not the shape.** A format that borrows PNG's
omission without PNG's encoders has thrown away a distinction for
nothing.

# A region with no scan in it is no region

A reading that reached the end of the image and found no
entropy-coded bytes returns [`ContentRegion::EmptySpan`] rather than
a digest, and the same rule stands behind it as behind PNG's "no
`IDAT`, no region": a digest over what is left is perfectly real, and
every file in that state would share it. Concretely — two stubs
carrying nothing but different EXIF blocks would hash their `EOI` and
nothing else, come out identical, and be offered to the user as the
same picture, which they are not and which the fold cannot take back.
The scan is where a JPEG keeps its picture; a file with none has no
picture for this axis to be about.

# Every structural defect is one outcome

The walk distinguishes truncation from a lying length from a byte
where a marker belongs from too many segments, and all of them land
on [`ContentRegion::EmptySpan`]. The variants are worth having — a
reader of a stack trace can tell which happened — but the stored
value must not fork on them, because the true statement they share is
the one the column carries: there is no complete region to stand
behind. Same arrangement as [`png`](super::png), argued where the
marker is defined
([`EMPTY_SPAN`](asterism_core::domain::content_region::EMPTY_SPAN)).

# The whole file, and the convention that follows from it

`content_of` takes a slice and reads all of it: the scan is the
largest part of a JPEG and it is the part that decides the answer, so
there is no prefix that settles this axis. A caller that wanted to
read only a file's first N bytes could not use this method, and the
convention it would need is worth writing down before somebody
reaches for it:

**A prefix read may answer the meta axis and may never answer the
content axis.** Metadata lives in the header region, so a bounded
read either finds it or truthfully says it did not. A content digest
taken over a prefix says "these are the bytes that decide what this
decodes to" about bytes that do not — two different photographs from
one camera share their `APP0`, `DQT`, `DHT` and `SOF`, so they would
be handed the same digest, declared duplicates, and folded. The size
question belongs to the job that opens the file, and its answer for a
file it will not read whole is
[`TOO_LARGE`](asterism_core::domain::content_region::TOO_LARGE) — a
marker saying the region was not computed — never a digest over part
of one.

# The rows that were here first, and how each axis got them back

Every JPEG already in the library carried `unsupported:image/jpeg` on
both axes, and a marker is a final answer to "has anybody looked", so
the ordinary fingerprint pass would never have offered those rows
again: they would keep the marker while files imported afterwards
took digests, leaving one column with two meanings and nothing in it
to tell them apart. That debt was this probe's to leave and is paid
on both axes now, by one migration step apiece — V72 cleared the
content column and V76 the meta one, each writing NULL over that one
literal so the ordinary walk selects the row and re-reads the file.

Neither is a version bump, and the distinction is worth keeping
straight (V72's doc argues it at length): a bump invalidates
**positives** — every digest ever written, on every format — where
these invalidate **negatives** on one format, rows saying "nothing
here reads JPEG", which was true when written and is not now.

One row is answered as a side effect and is worth naming: the walk
recomputes every column from a single read, so a JPEG re-offered on
the meta axis also has its `meta_raw` filled in, replacing the
`unsupported:not-captured` V75 wrote across the whole table.

## Types

- `JpegProbe` — JPEG's reading of the content axis.

