//! JPEG's reading of the content axis: which of its segments are the
//! picture, and which are notes written about it.
//!
//! The segment framing — where one marker segment ends and the next
//! begins, how far the entropy-coded data runs — is
//! [`asterism_media_probe::jpeg`](asterism_media_probe::jpeg), and it has
//! no opinion about any of this. What is here is the opinion, on the same
//! terms as [`png`](super::png): a judgement about *this* corpus, which
//! is why it sits beside the application rather than in a crate that
//! walks JPEGs in general.
//!
//! # Both axes, and what the second one took to arrive
//!
//! This probe declares `image/jpeg` on **both** axes ([`CLAIMS`]). It
//! arrived on the content axis alone, and the meta axis was declined for
//! a stated reason rather than left unwritten: a meta digest over EXIF
//! groups photographs that share a camera body, a lens and a second of
//! wall-clock time — frames 3 and 4 of a burst, with the fields that
//! separate them stripped by whatever exported them. The duplicate panel
//! is driven by these columns, so a definition that admits that fills
//! the panel with pairs a person has to refuse one at a time. What that
//! argument concluded was that **the reading worth having is a narrower
//! one, over fields chosen for the purpose, and choosing them is its own
//! slice.**
//!
//! That slice is the series axis
//! ([`series`](asterism_core::domain::series)), and it landed. A
//! [`Strategy`](asterism_core::domain::series::Strategy) is a registered
//! rule that reads `meta_kv` and derives its own key, so *which* EXIF
//! fields decide that two photographs were made the same way is a value
//! somebody registers and edits rather than a definition compiled into
//! this file.
//!
//! **That answers half of the objection, and the other half is a live
//! defect.** Claiming this axis does two things at once, and only one of
//! them is the Series substrate: `image/jpeg` also enters the **meta
//! duplicate axis**, because the column it fills is one of the three
//! [`detect_duplicate`](asterism_core::application_support::duplicate_detection::detect_duplicate)
//! walks. A JPEG pair whose blocks agree can now stop there, and a
//! `duplicate_conflict` row is a question with a fold button on it —
//! which is the panel behaviour the burst argument was about, arriving
//! by a different door than it expected.
//!
//! What lands there is not a duplicate, and that is the defect rather
//! than a mitigation. The walk stops at the *first* agreement, so a pair
//! reaches `Meta` only when `Artefact` and `Content` both found
//! nothing — **the pictures differ**. So a meta-alone agreement is not
//! an identity claim at all; it is "made the same way", which is the
//! sentence [`series`](asterism_core::domain::series) exists to say and
//! deliberately says without folding anything.
//!
//! The root of it is one axis older than this slice: the algebra is
//! `Artefact = Content + Meta` — the whole bytes are the picture plus
//! the metadata — so there are **two** independent axes and `Artefact`
//! is the name for both agreeing, while
//! [`STRONGEST_FIRST`](asterism_core::domain::duplicate_conflict::DuplicateAxis::STRONGEST_FIRST)
//! lists three in parallel. **This build still does that**; correcting
//! it is its own slice, and nothing here anticipates the correction.
//!
//! **Nothing is selected here, deliberately.** Every field the block
//! states goes into `meta_kv`, including the ones a burst rewrites and
//! the ones nobody has a use for, because narrowing on this side would
//! decide for every Strategy at once what an author is allowed to
//! address — the same argument
//! [`asterism_media_probe::exif_tags`] makes for refusing to select one
//! layer further down, and the reason
//! [`series`](asterism_core::domain::series) offers `exclude` beside
//! `include`.
//!
//! ## Nobody has published the axis a narrower reading would need
//!
//! "Keep the fields a burst does not rewrite" sounds like a lookup, and
//! it was researched as one before this axis was claimed. A published
//! classification of every standard tag does exist — **Exif 3.0
//! Annex H**, the guidelines for handling tag information in
//! post-processing by application software (CIPA DC-008; the current
//! edition is 3.1) — and it gives each tag two labels. A **Category**:
//! I image structure, II shooting conditions, III the rest (model,
//! serial number, capture time, place, owner, copyright). And a
//! **Rank**: `Update 0` (update on every edit), `Update 1` (may be
//! updated on its own), `Freeze 0` (shall not be deleted or modified
//! under any circumstance), `Freeze 1` (needs no update), `Freeze 2`
//! (may be corrected where wrong). Two rules bind the pair: every
//! Category I tag is `Update 0`, and Category II — the shooting
//! settings — is `Freeze 1`.
//!
//! **It answers a different question.** Annex H's axis is *may a tool
//! rewrite this*; the axis a series needs is *does this change from one
//! exposure to the next*. They correlate, and they come apart in one
//! direction systematically:
//!
//! | Rank | tags | changes across a burst? |
//! |---|---|---|
//! | `Update 0` | `DateTime` (`0x0132`), `SubSecTime`, image structure | yes — the one part usable as a quotation |
//! | `Freeze 0` | `ImageUniqueID` (`0xa420`), alone | frozen *and* per-exposure |
//! | `Freeze 2` | `DateTimeOriginal` (`0x9003`), the GPS tags, the lens tags, `CameraFirmware` | the capture time does; the lens and the firmware do not |
//! | `Freeze 1` | `Make`, `Model`, `BodySerialNumber`, **and exposure time, aperture, ISO, focal length** | the body does not; under auto-exposure the exposure tags change frame to frame |
//!
//! So **reading `Freeze 1` / `Freeze 2` as "steady across a burst" is
//! this project's judgement rather than a citation**, and it is wrong
//! exactly where auto-exposure is working. Excluding `Update 0` **is** a
//! citation: the specification says an editor rewrites those fields, so
//! dropping them is following a rule somebody else wrote. The
//! specification illustrates the divergence itself — a subject position
//! is Category II and `Freeze 2` as a subject, and is reclassified
//! Category I/II and `Update 0` when it is expressed relatively, because
//! resizing the picture moves it.
//!
//! **No other published schema closes the gap**, which is itself the
//! finding rather than a gap in the search. MWG 2.0 reconciles one
//! field across containers (Exif ↔ IIM ↔ XMP) and has no stability axis
//! — and the group has not existed since 2018. IPTC's Photo Metadata
//! TechReference defines properties, machine-readably and openly
//! licensed, but defines no classification. XMP's `xmpMM`
//! (ISO 16684-1) versions a *document* — `DocumentID`, `InstanceID`,
//! `OriginalDocumentID` — which separates two edits of one photograph
//! and says nothing about two frames of one burst. C2PA asserts
//! provenance, and its hard bindings are *designed* not to match a
//! derivative or a related frame, which is the opposite direction.
//! Somebody has to make the judgement, so it is made by the author of a
//! [`Strategy`](asterism_core::domain::series::Strategy) — see
//! [`Decode::Exif`](asterism_core::domain::series::Decode::Exif) for
//! what that author is told, and this module for why the probe hands
//! them everything to choose from.
//!
//! Sources: <https://www.cipa.jp/e/std/std-sec.html>;
//! <https://archive.org/details/exif-specs-3.0-dc-008-translation-2023-e>
//! (Annex H, pp. 233–241). CIPA's own PDF is encrypted, so the ranks
//! above were read off the Internet Archive's OCR of the 2023 English
//! translation: the rank tokens come through cleanly, the Category
//! roman numerals do not always, which is why no per-tag Category is
//! quoted here.
//!
//! ## The one identifier the specification insists on is the one nobody trusts
//!
//! `ImageUniqueID` (`0xa420`) is what a reader reaches for first, and
//! the specification is at its most emphatic about it: `Freeze 0`, the
//! only tag carrying that rank, *shall not be deleted or modified under
//! any circumstance*. **Two independent implementations record that it
//! is not written reliably.** One validates the value's shape before
//! trusting it, because cameras were found writing their own model name
//! into the field; the other declined the tag outright as missing,
//! reused and inconsistent across vendors. So the tag is read like any
//! other — it lands in `meta_kv` and a Strategy may name it — but
//! nothing here treats it as an identity on its own.
//!
//! Neither is a standard, and the same holds for every other grouping
//! rule shipped in this space: they are timestamp windows, filename
//! stems and capture intervals, and at least one vendor has had to turn
//! one of them **off by default** after unrelated photographs
//! downloaded together were grouped by a shared prefix. That is the
//! merge-is-unrecoverable asymmetry this file argues from, observed in
//! production by somebody else, and it is why the axis a Strategy reads
//! is data rather than a rule compiled in here.
//!
//! Which products those are, and what each one groups on, is not
//! written here on purpose. A per-vendor survey in a module doc is read
//! once and then goes stale silently, and it is not what this file has
//! to decide — the decision is the sentence above, that no published
//! rule can be quoted for this axis, so the axis is data.
//!
//! ## What lands in `meta_kv`, and what a key may not carry
//!
//! [`exif_tags`](asterism_media_probe::exif_tags) produces the map
//! whole: `exif:0x829a` → `rational:1/125`, the key an address and the
//! value self-describing. The two properties this file leans on are
//! argued there — the IFD is in the key because one tag number means
//! different things in different IFDs, and the type is on the *value*
//! because `1/125` is otherwise ambiguous between a rational and an
//! ASCII tag whose text is literally that.
//!
//! ## The block's bytes are kept as well
//!
//! [`meta_raw_of`](JpegProbe::meta_raw_of) keeps the `APP1` payload
//! verbatim, which is what makes the rendering above revisable: `MakerNote`
//! is one opaque `undefined` value today, and Apple's `BurstUUID` — the
//! signal every other library reaches for — is inside it. A reader that
//! learns to open it later works from the column
//! ([`material_meta_raw`](asterism_core::domain::material_meta_raw))
//! rather than from somebody's disk.
//!
//! # Which segments are content
//!
//! Everything except three, named one at a time:
//!
//! - `APP1` whose payload begins `Exif\0\0` — the EXIF block. **One
//!   field of it comes back in by another door**; see below.
//! - `APP1` whose payload begins `http://ns.adobe.com/xap/1.0/\0` *and
//!   does not carry the bytes `tiff:Orientation` anywhere in it* — an
//!   XMP packet. `APP1` is shared by both and by anything else a vendor
//!   puts there, so the marker alone does not identify either; the
//!   payload's own identifier does, and an `APP1` carrying neither is
//!   **content**.
//! - `COM` (`0xFFFE`) — the comment segment.
//!
//! …and one thing that is in the region without being a segment: the
//! orientation the file renders under, fed as its own element.
//!
//! A denylist, not an allowlist of the segments known to matter, for the
//! reason written into the port's contract
//! ([`ArtefactProbe::content_of`]): the two are wrong in opposite
//! directions, and not by a little. An allowlist drops whatever nobody
//! thought of, and saying "the same" about two artefacts that are not is
//! a loss no later correction can undo — downstream a fold turns the
//! loser of a duplicate group into a tombstone. A denylist's error runs
//! the other way: a metadata segment nobody remembered is hashed, two
//! files differing only in it get two digests, and that is exactly the
//! state of a format with no content axis at all.
//!
//! ## The criterion is rendering, and EXIF Orientation meets it
//!
//! The denylist has a *criterion* behind it, and the criterion is not
//! "is this segment metadata" — it is **does this change what a person
//! is shown**. A rule that excludes by segment name is a shortcut
//! through that question, and it works right up until a segment carries
//! both kinds of byte at once. EXIF is that segment.
//!
//! Orientation codes `2`–`4` mirror the frame and `5`–`8` transpose it,
//! and this repository already treats that as display-affecting rather
//! than as trivia: the stored dimensions are documented as *coded*, with
//! the caller told to combine them with the orientation to get a
//! displayed shape
//! ([`AssetDto::width_px`](asterism_contract::dto::AssetDto::width_px)),
//! the image importer carries the code onto the asset's `extra`, and the
//! detail pane prints it. So the failure was concrete rather than
//! theoretical: a phone photograph tagged `Orientation=6` and the same
//! file through any EXIF stripper have **byte-identical entropy-coded
//! data**. Under a wholesale exclusion they take one digest, land in one
//! duplicate group, and a fold turns the row that displayed the right
//! way up into a tombstone. After the fold there is one picture, so
//! there is nothing left for a person to compare and notice — the
//! unrecoverable direction, reached through the door the denylist was
//! built to hold shut.
//!
//! The answer is not to stop excluding EXIF. Nearly all of it *is*
//! notes — timestamps, camera bodies, GPS, exposure — and excluding it
//! is the reason this axis exists. What the region takes is the one
//! field, normalised, on its own:
//!
//! - **Normalised**, because absent, unreadable and `1` all render
//!   identically, so all three feed the same value. A file that never
//!   had an orientation and a file explicitly tagged upright are one
//!   picture, and a probe that could tell them apart would be reporting
//!   on the EXIF block again.
//! - **In a fixed position**, ahead of every segment, because *where*
//!   the `APP1` sat is a fact about the writer and not about the
//!   picture. Two files whose EXIF blocks are at different offsets, or
//!   one of which has none at all, still agree.
//!
//! It costs one header-region EXIF parse per content walk
//! ([`jpeg::orientation`](asterism_media_probe::jpeg::orientation)),
//! which is the same read the importer already does at ingest. Reading
//! the tag is `asterism-media-probe`'s (a fact about the format);
//! deciding it belongs in a digest is this file's (a judgement about a
//! corpus), which is the same line every other rule here is drawn on.
//!
//! `tests::two_photographs_differing_only_in_their_orientation_are_two_pictures`
//! keeps it measured rather than asserted, the way the `APP2` case
//! below does: it runs a second walk with EXIF excluded whole and shows
//! the phone photograph and its stripped export collapsing onto one
//! digest under it.
//!
//! ### XMP carries it too, and is not parsed for it
//!
//! `tiff:Orientation` can appear in an XMP packet. Reading XMP would
//! mean an XML parser deciding what a picture is, so instead the
//! **exclusion is withdrawn**: an XMP packet with those bytes anywhere
//! in it stays in the region, in full. The cost is an improvement lost
//! — two files differing only in unrelated XMP that happens to mention
//! the property get two digests — and that is the direction this whole
//! selection is allowed to fail in.
//!
//! ### What is still excluded and might have mattered
//!
//! Stated rather than implied, because the criterion above does not
//! stop at one tag. `Gamma`, `ColorSpace`, and the TIFF colour
//! description tags (`WhitePoint`, `PrimaryChromaticities`,
//! `TransferFunction`) can all change a rendering, and all of them are
//! inside the excluded `APP1` today. Two files differing only in one of
//! those still take one digest.
//!
//! That is a smaller risk than the one being fixed, on this corpus, and
//! saying why is the point: those tags are rare, they are usually
//! redundant beside an `APP2` ICC profile that **is** in the region, and
//! a difference in them is subtle where a transposed photograph is
//! obvious at arm's length. Orientation is the one tag that is on nearly
//! every photograph a phone takes, that nearly every export pipeline
//! strips, and that this repo had already decided is display-affecting.
//! It earned the machinery; the rest have not yet, and the way they
//! would is somebody measuring a pair on this corpus.
//!
//! ## `APP2` is content, and this is the segment that says why
//!
//! **The ICC colour profile is inside the region.** Two files whose
//! pixels agree byte for byte and whose profiles differ are two
//! pictures: the profile is what the same numbers are rendered *as*, so
//! excluding it merges an sRGB export with a Display-P3 one and a viewer
//! shows the difference.
//!
//! That is not a hypothetical borrowed from a specification. It is the
//! failure the PNG probe measured on this repo's own corpus, in the same
//! place — colour-management chunks and APNG frame data, where two
//! visibly different pictures came out with one digest — and the shape
//! of the mistake there was exactly this one: a rule that looked like it
//! was excluding metadata was excluding rendering.
//! `tests::an_icc_profile_is_part_of_the_picture` keeps it measured
//! rather than asserted, by running a second walk with `APP2` on the
//! excluded side and showing that the two files collapse onto one
//! digest under it.
//!
//! Everything else stays in: `APP0` (JFIF), `APP13` (Photoshop's image
//! resource block), every other `APPn`, extended XMP — which announces
//! itself with a *different* identifier and is therefore not one of the
//! two `APP1` cases above — the coding tables (`DQT`, `DHT`, `SOF*`,
//! `DRI`), the `SOS` headers, the entropy-coded data, the restart
//! markers inside it, the closing `EOI`, and whatever the file carries
//! behind it. When a segment is arguable, it goes here: that error costs
//! a duplicate nobody spots.
//!
//! ## The bytes after `EOI` are content, and PNG's are not
//!
//! `EOI` ends the image and does not end the file. **Google and Samsung
//! Motion Photos are a complete JPEG with a complete MP4 appended**, and
//! that is how a phone ships a still and its couple of seconds of video
//! as one artefact — a default, not a curiosity. Excluding the tail
//! makes a Motion Photo and its still-only export one digest: measured,
//! a 598-byte still and the same still with 4 KB appended came out
//! identical. One duplicate group, one fold, and the video is gone with
//! the row that carried it.
//!
//! So the trailing span is in the region, as one element, fed whole and
//! unread — the walk has no grammar for what follows an image and does
//! not need one to know the bytes are there.
//!
//! **[`png`](super::png) excludes what follows its `IEND` and stays that
//! way**, which is a decision rather than an inconsistency the two
//! probes have not got round to reconciling. The question is not what
//! the container permits — both permit a trailing payload — but what
//! this corpus's files do with it. A PNG carrying something meaningful
//! after `IEND` is rare enough that the PNG probe's own fixture had to
//! construct one; a JPEG carrying something meaningful there is what a
//! phone hands you. Where the format's *use* differs, the region
//! definitions differ with it, and copying either shape onto the other
//! would be copying past the reasoning again.
//!
//! # What is fed to the hash
//!
//! The normalised orientation first, as one byte, before anything the
//! walk yields — the fixed position argued for above. Then, in the order
//! the file carries them:
//!
//! - a framed segment: `marker (1 byte) || length (2 bytes,
//!   big-endian) || payload`;
//! - a marker with no length field: its single byte;
//! - entropy-coded data: `length (8 bytes, big-endian) || bytes`, the
//!   bytes exactly as the file carries them, stuffing and restart
//!   markers included;
//! - the trailing span: `length (8 bytes, big-endian) || bytes`.
//!
//! `SOI` is not fed. It is the signature, it was checked to get here,
//! and it is the same two bytes in every JPEG, so feeding it would
//! separate nothing from nothing.
//!
//! ## Every element carries its own length, including the ones with no
//! length field
//!
//! A framed segment's length is in the file and gets fed (the next
//! section argues that against PNG, which omits its own). A scan's
//! length is *not* in the file — entropy-coded data is delimited by the
//! marker that follows it — and it is fed anyway, computed, for the
//! identical reason: **without it the region is one run of bytes with no
//! seam in it, and a scan can spell out the element behind it.**
//!
//! Measured, before this was fixed: a scan ending `0x11 0xE4 0x00 0x04
//! 0xAA 0xBB` followed by `EOI`, against a scan ending `0x11` followed
//! by an `APP4` carrying `0xAA 0xBB` and then `EOI`. Two entropy streams
//! differing by five bytes, one digest. The `0xFF` that introduces every
//! marker is not fed either — it is in every marker and separates
//! nothing — so nothing in the feed marked the boundary at all.
//! `tests::the_scans_length_separates_it_from_the_segment_behind_it` is
//! that pair.
//!
//! Eight bytes rather than the file's two, because the number being
//! described is not a segment's: a scan is as long as the picture is,
//! and the trailing span can be a whole video. A scan of no bytes is fed
//! as a length of zero rather than skipped, so the rule has no exception
//! in it.
//!
//! With that, the feed is *decodable*: read a byte, and whether it is a
//! bare marker or the head of a framed one is a property of the byte
//! alone; after an `SOS` comes a scan's length; after `EOI`, a trailing
//! length if anything is left. Two different element sequences cannot
//! produce one stream — which is the property "no collisions from
//! re-division" actually names, and it is what makes the claim checkable
//! instead of a hope.
//!
//! ## The length **is** fed, and PNG's is not
//!
//! The second of the two places these probes disagree about the shape of
//! an answer (the trailing span was the first), so it is worth being
//! explicit about why, and about why the difference is not an
//! inconsistency to be tidied away later.
//!
//! PNG omits the chunk length because an encoder is free to cut one
//! compressed stream into any number of `IDAT` chunks — zlib's buffer
//! size is not part of the image — and hashing the lengths would make
//! one picture written by two encoders two pictures. That was measured:
//! the same stream in 1, 8 and 63 chunks produces one digest, and a real
//! ComfyUI corpus writes its pixels as 17–24 chunks of 64 KiB.
//!
//! **A JPEG segment's length is not that kind of number.** It is fixed
//! by the segment's own contents — a quantisation table is as long as a
//! quantisation table — and there is no re-cutting for it to absorb,
//! because the one unbounded run in the container (the scan) is not
//! length-prefixed at all. So PNG's reason does not carry over, and with
//! no reason to drop it the length goes in, because feeding it removes a
//! class of collision: without it, `marker || payload` runs together, and
//! two adjacent segments of one marker can be re-divided into two
//! different segments of the same marker with the same concatenation.
//!
//! The general form of the rule, for whoever adds the next format:
//! **copy the reasoning, not the shape.** A format that borrows PNG's
//! omission without PNG's encoders has thrown away a distinction for
//! nothing.
//!
//! # A region with no scan in it is no region
//!
//! A reading that reached the end of the image and found no
//! entropy-coded bytes returns [`ContentRegion::EmptySpan`] rather than
//! a digest, and the same rule stands behind it as behind PNG's "no
//! `IDAT`, no region": a digest over what is left is perfectly real, and
//! every file in that state would share it. Concretely — two stubs
//! carrying nothing but different EXIF blocks would hash their `EOI` and
//! nothing else, come out identical, and be offered to the user as the
//! same picture, which they are not and which the fold cannot take back.
//! The scan is where a JPEG keeps its picture; a file with none has no
//! picture for this axis to be about.
//!
//! # Every structural defect is one outcome
//!
//! The walk distinguishes truncation from a lying length from a byte
//! where a marker belongs from too many segments, and all of them land
//! on [`ContentRegion::EmptySpan`]. The variants are worth having — a
//! reader of a stack trace can tell which happened — but the stored
//! value must not fork on them, because the true statement they share is
//! the one the column carries: there is no complete region to stand
//! behind. Same arrangement as [`png`](super::png), argued where the
//! marker is defined
//! ([`EMPTY_SPAN`](asterism_core::domain::content_region::EMPTY_SPAN)).
//!
//! # The whole file, and the convention that follows from it
//!
//! `content_of` takes a slice and reads all of it: the scan is the
//! largest part of a JPEG and it is the part that decides the answer, so
//! there is no prefix that settles this axis. A caller that wanted to
//! read only a file's first N bytes could not use this method, and the
//! convention it would need is worth writing down before somebody
//! reaches for it:
//!
//! **A prefix read may answer the meta axis and may never answer the
//! content axis.** Metadata lives in the header region, so a bounded
//! read either finds it or truthfully says it did not. A content digest
//! taken over a prefix says "these are the bytes that decide what this
//! decodes to" about bytes that do not — two different photographs from
//! one camera share their `APP0`, `DQT`, `DHT` and `SOF`, so they would
//! be handed the same digest, declared duplicates, and folded. The size
//! question belongs to the job that opens the file, and its answer for a
//! file it will not read whole is
//! [`TOO_LARGE`](asterism_core::domain::content_region::TOO_LARGE) — a
//! marker saying the region was not computed — never a digest over part
//! of one.
//!
//! # The rows that were here first, and how each axis got them back
//!
//! Every JPEG already in the library carried `unsupported:image/jpeg` on
//! both axes, and a marker is a final answer to "has anybody looked", so
//! the ordinary fingerprint pass would never have offered those rows
//! again: they would keep the marker while files imported afterwards
//! took digests, leaving one column with two meanings and nothing in it
//! to tell them apart. That debt was this probe's to leave and is paid
//! on both axes now, by one migration step apiece — V72 cleared the
//! content column and V76 the meta one, each writing NULL over that one
//! literal so the ordinary walk selects the row and re-reads the file.
//!
//! Neither is a version bump, and the distinction is worth keeping
//! straight (V72's doc argues it at length): a bump invalidates
//! **positives** — every digest ever written, on every format — where
//! these invalidate **negatives** on one format, rows saying "nothing
//! here reads JPEG", which was true when written and is not now.
//!
//! One row is answered as a side effect and is worth naming: the walk
//! recomputes every column from a single read, so a JPEG re-offered on
//! the meta axis also has its `meta_raw` filled in, replacing the
//! `unsupported:not-captured` V75 wrote across the whole table.

use super::under_region_tag;
use asterism_core::domain::content_hash::ContentHasher;
use asterism_core::domain::content_region::{ContentRegion, UNKNOWN_FORMAT};
use asterism_core::domain::material_meta::{self, MaterialMeta};
use asterism_core::domain::material_meta_raw::MetaRaw;
use asterism_core::domain::probe::{ArtefactProbe, FormatClaim, GateOpen};
use asterism_core::domain::value::{ImageFormat, MimeType};
use asterism_media_probe::{exif_tags, jpeg};

/// The application segment EXIF and XMP both travel in — and so does
/// anything else a vendor decided to put there, which is why the marker
/// alone excludes nothing.
const APP1: u8 = 0xE1;

/// The comment segment.
const COM: u8 = 0xFE;

/// What an EXIF `APP1` payload begins with, NUL padding included.
const EXIF_IDENTIFIER: &[u8] = b"Exif\0\0";

/// What an XMP `APP1` payload begins with: the namespace URI and its
/// terminator.
///
/// Extended XMP — the continuation packets a large sidecar is split
/// into — announces itself with `http://ns.adobe.com/xmp/extension/\0`
/// instead, and is deliberately **not** matched here. That leaves it in
/// the region, so two files differing only in a large XMP sidecar get
/// two digests: an improvement lost, which is the direction this whole
/// selection is allowed to fail in. Adding it is a one-line decision
/// somebody can make on evidence; making it silently would be the other
/// direction.
const XMP_IDENTIFIER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// The property name an XMP packet states an orientation under.
///
/// Looked for as a substring of the whole packet, not parsed out of it:
/// XMP is RDF/XML, the property can be an attribute or an element, and
/// the namespace prefix is nominally rebindable. A packet that mentions
/// it **stays in the region entirely** — the question being asked is not
/// "what does it say" but "might this packet be about the rendering",
/// and the answer that costs a duplicate is the one to give when unsure.
const XMP_ORIENTATION_PROPERTY: &[u8] = b"tiff:Orientation";

/// What a file with no readable orientation renders as, and what it
/// therefore feeds.
///
/// `1` is EXIF's own code for upright, so this is not a sentinel value
/// standing in for absence: it is the claim that a file saying nothing
/// and a file saying "upright" are the same picture, which they are.
const UPRIGHT: u8 = 1;

/// JPEG's reading of the content axis.
///
/// Stateless — the registry holds it as a constant.
#[derive(Debug, Clone, Copy, Default)]
pub struct JpegProbe;

/// What this probe answers for: `image/jpeg`, on both axes.
///
/// One claim, and the only place this probe's formats are written. The
/// gates a caller asks are read off it
/// ([`ProbeGates`](asterism_core::domain::probe::ProbeGates)), and so is
/// the registry's completeness check, so there is no second list to
/// forget to edit.
///
/// `meta` stood at `false` until the series axis existed to make a
/// narrow reading expressible — see the module doc — and flipping it is
/// the load-bearing half of that slice: it is what makes
/// [`meta_of`](JpegProbe::meta_of) reachable at all, since the port
/// refuses a declined axis before the body is entered, and what stops
/// every JPEG in the library storing "nobody looked" for good.
///
/// The alternative was to leave the axis declined and let a
/// [`Strategy`](asterism_core::domain::series::Strategy) read the EXIF
/// itself, and what rules it out is arithmetic rather than taste: a
/// Strategy reads `meta_kv`, so a JPEG whose fields never reach that
/// column could only be addressed by opening the file — and then
/// *editing a rule* would mean re-reading every JPEG in the library
/// off disk, which is precisely the property the series axis is sold
/// on giving up nothing of. The reading lands in a column so that
/// changing one's mind about it costs a scan.
const CLAIMS: &[FormatClaim] = &[FormatClaim {
    mime: MimeType::Image(ImageFormat::Jpeg),
    content: true,
    meta: true,
}];

impl JpegProbe {
    /// `Some(refusal)` when these bytes are not a JPEG's — the half of
    /// the refusal that is this probe's to write.
    ///
    /// The claim selects and then the signature is checked against it,
    /// in that order, and only the second is here. Anything [`CLAIMS`]
    /// does not cover is refused in the port before this file is
    /// reached, and the value stored is
    /// [`content_region::unsupported_format`](asterism_core::domain::content_region::unsupported_format)'s
    /// — the same value the caller stores for a file it decided not to
    /// open.
    ///
    /// This half exists because a mime is a guess from a filename and
    /// lies in both directions: a `.jpg` that is really a PNG would
    /// otherwise be handed to a marker walk, and pointing a container
    /// parser at whatever the file really is, on the caller's word for
    /// it, is the shape of problem this kind of code is known for.
    /// Refusing costs nothing real — the file axis groups renamed copies
    /// perfectly well, since renaming does not change a byte.
    fn refusal(bytes: &[u8]) -> Option<String> {
        if !jpeg::is_jpeg(bytes) {
            return Some(UNKNOWN_FORMAT.to_string());
        }
        None
    }
}

/// Whether this segment is one of the three the region excludes.
///
/// Written as a function of the marker **and the payload** because two
/// of the three cannot be recognised from the marker: `APP1` carries
/// EXIF, XMP and whatever else a producer put there, and only the
/// payload's own leading identifier says which. An `APP1` matching
/// neither is content, which is the denylist's direction — a private
/// `APP1` nobody here has seen gets hashed, and two files differing only
/// in it get two digests.
///
/// The XMP arm has an escape and the EXIF arm does not, which is the
/// asymmetry to notice: EXIF's one rendering field is read out and fed
/// separately, so excluding the block loses nothing that matters, while
/// XMP is not parsed at all, so the only way not to lose its copy of the
/// same field is to keep the packet.
fn is_metadata(marker: u8, payload: &[u8]) -> bool {
    match marker {
        COM => true,
        APP1 => {
            payload.starts_with(EXIF_IDENTIFIER)
                || (payload.starts_with(XMP_IDENTIFIER) && !states_an_orientation(payload))
        }
        _ => false,
    }
}

/// Whether an XMP packet mentions [`XMP_ORIENTATION_PROPERTY`] anywhere.
fn states_an_orientation(payload: &[u8]) -> bool {
    payload
        .windows(XMP_ORIENTATION_PROPERTY.len())
        .any(|window| window == XMP_ORIENTATION_PROPERTY)
}

/// Feeds a run of bytes the file does not length-prefix, behind a length
/// this does.
///
/// The two of them — the entropy-coded scan and whatever trails the
/// `EOI` — are the elements with no framing of their own, so a bare
/// `update` would let one of them run into the next element and spell it
/// out. Eight bytes big-endian because neither is bounded by a `u16`
/// the way a segment is: a scan is the size of the picture and a
/// trailing span can be a video. See the module doc for the pair that
/// collided before this existed.
fn feed_span(hasher: &mut ContentHasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

impl ArtefactProbe for JpegProbe {
    fn declares(&self) -> &'static [FormatClaim] {
        CLAIMS
    }

    fn content_of(
        &self,
        bytes: &[u8],
        _declared_mime: Option<&MimeType>,
        _gate: GateOpen,
    ) -> ContentRegion {
        if let Some(format) = Self::refusal(bytes) {
            return ContentRegion::Unsupported(format);
        }

        // The only failure `segments` reports up front is a signature
        // that is not a JPEG's, which `refusal` has already asked about
        // — so this arm is unreachable in practice. It is written out
        // rather than unwrapped because the answer to "these bytes are
        // not a JPEG" is a value already in hand, and panicking on
        // untrusted input to save a line is how a parser earns its
        // reputation.
        let Ok(walk) = jpeg::segments(bytes) else {
            return ContentRegion::Unsupported(UNKNOWN_FORMAT.to_string());
        };

        let mut hasher = ContentHasher::new();
        // The rendering the file declares, ahead of everything the walk
        // yields — not where the `APP1` that stated it happened to sit,
        // which is a fact about the writer. Absent, unreadable and
        // upright are one value here; see the module doc.
        hasher.update(&[jpeg::orientation(bytes).unwrap_or(UPRIGHT)]);

        // Whether the picture itself was reached. Nothing else here
        // accumulates: elements are fed to the hash where they sit, in
        // file order, so a JPEG costs one pass and no list — the reason
        // `jpeg::MAX_SEGMENTS` has no counterpart on this side to agree
        // with, where PNG's ceiling bounds a `Vec` in its probe.
        let mut scanned = false;

        for item in walk {
            let Ok(element) = item else {
                return ContentRegion::EmptySpan;
            };
            match element {
                jpeg::Element::Bare(marker) => hasher.update(&[marker]),
                jpeg::Element::Framed { marker, payload } => {
                    if is_metadata(marker, payload) {
                        continue;
                    }
                    // The length the file declared, re-derived. The walk
                    // refuses a segment whose declared length does not
                    // match the bytes present, so this is that number,
                    // and it cannot exceed a `u16` because it came out
                    // of one. Saturating rather than `expect`, on the
                    // rule above about panicking on untrusted input:
                    // the saturated value exists only in a branch the
                    // framing cannot produce.
                    let declared = u16::try_from(payload.len() + 2).unwrap_or(u16::MAX);
                    hasher.update(&[marker]);
                    hasher.update(&declared.to_be_bytes());
                    hasher.update(payload);
                }
                jpeg::Element::Scan(entropy) => {
                    // Fed even when it is empty — a length of zero — so
                    // that "after an SOS comes a scan's length" has no
                    // exception in it. Whether the *picture* was reached
                    // is the separate question below.
                    scanned |= !entropy.is_empty();
                    feed_span(&mut hasher, entropy);
                }
                // Not part of the image, and part of the artefact — a
                // Motion Photo's video lives here. See the module doc
                // for why this is the one place the two probes' region
                // definitions diverge on purpose.
                jpeg::Element::Trailing(appended) => feed_span(&mut hasher, appended),
            }
        }

        if !scanned {
            return ContentRegion::EmptySpan;
        }
        ContentRegion::Digest(under_region_tag(hasher))
    }

    /// Every field the EXIF block states, rendered into the canonical
    /// form and hashed.
    ///
    /// The reading is [`exif_tags`](asterism_media_probe::exif_tags)'s
    /// and the selection is nobody's: the whole map goes in. Which
    /// fields are worth grouping on is a
    /// [`Strategy`](asterism_core::domain::series::Strategy)'s to say,
    /// and narrowing here would settle it for every Strategy at once —
    /// see the module doc.
    ///
    /// Three answers, on [`png`](super::png)'s terms:
    ///
    /// - a **digest** where the block states at least one field;
    /// - [`MaterialMeta::EmptySpan`] where the container states no EXIF,
    ///   or states a block with no fields in it. Two facts, one answer,
    ///   which is the same collapse the content walk makes across its
    ///   structural defects: what they share is the sentence the column
    ///   carries, that a reading ran and found nothing to hash. Most
    ///   JPEGs in this corpus are here — of 250 sampled from a real
    ///   download directory, 246 carried no EXIF at all;
    /// - [`MaterialMeta::Unsupported`] where the bytes are not a JPEG's,
    ///   which is the same refusal [`content_of`](Self::content_of)
    ///   makes and for the same reason.
    ///
    /// **The `APP1` this reads is the one the content region excludes**,
    /// which is the port's own obligation
    /// ([`ArtefactProbe::meta_of`]): an element read into the metadata is
    /// an element the content digest must not also carry, or the two axes
    /// stop being two. The one field that crosses back is the
    /// orientation, and it crosses as a *rendering* rather than as a
    /// field — argued in the module doc, and visible here as the
    /// asymmetry that the content walk feeds one normalised byte while
    /// this map carries `ifd0:0x0112` whole.
    fn meta_of(
        &self,
        bytes: &[u8],
        _declared_mime: Option<&MimeType>,
        _gate: GateOpen,
    ) -> MaterialMeta {
        if let Some(format) = Self::refusal(bytes) {
            return MaterialMeta::Unsupported(format);
        }
        match exif_tags(bytes) {
            Some(fields) if !fields.is_empty() => {
                let canonical = material_meta::render(&fields);
                MaterialMeta::Digest {
                    digest: material_meta::digest_of(&canonical),
                    canonical,
                }
            }
            _ => MaterialMeta::EmptySpan,
        }
    }

    /// The EXIF `APP1`'s payload, exactly as the file carries it —
    /// identifier and all, so what comes back is a segment body rather
    /// than a TIFF structure somebody has to guess the framing of.
    ///
    /// # The ceiling is the format's
    ///
    /// [`png`](super::png) has to state a number because nothing bounds
    /// a PNG's text chunks; **this one cannot exceed 65,533 bytes
    /// because an `APP1`'s length field is two bytes**, and the walk
    /// refuses a segment whose declared length is not the bytes present.
    /// So there is no [`MetaRaw::TooLarge`] arm here, and its absence is
    /// the format answering rather than this file forgetting: a ceiling
    /// invented beside a structural one would be a second bound that can
    /// disagree with it. A file carrying several EXIF `APP1`s is not a
    /// way around that — the **first** is kept, which is also the one
    /// every reader parses, so the bytes in the column are the bytes the
    /// reading above was taken over.
    ///
    /// # It stops where the reading stops, and PNG's does not
    ///
    /// The walk ends at the scan, and a defect before that ends it
    /// wherever it lands: the bytes found up to there are kept rather
    /// than discarded. [`png`](super::png) makes the opposite choice —
    /// an incomplete chunk walk keeps nothing — and copying either shape
    /// onto the other would be copying past the reasoning, which this
    /// file has now had to say three times (the trailing span and the
    /// length field are the other two).
    ///
    /// What differs is where each container keeps its metadata. A PNG's
    /// text chunks sit on **both sides** of the pixel data — a ComfyUI
    /// export carries one before and one after — so a walk that stopped
    /// early may have missed some, and there is no way to say which. A
    /// JPEG's EXIF is in the header region ahead of the scan, and the
    /// reader that produced `meta_kv` stops there too: it walks markers
    /// until `SOS` and gives up at it. So a truncated JPEG whose header
    /// is intact has one complete answer on both columns, and keeping
    /// the bytes is what makes the pair a round trip instead of a digest
    /// beside a NULL.
    fn meta_raw_of(
        &self,
        bytes: &[u8],
        _declared_mime: Option<&MimeType>,
        _gate: GateOpen,
    ) -> MetaRaw {
        if Self::refusal(bytes).is_some() {
            return MetaRaw::Absent;
        }
        let Ok(walk) = jpeg::segments(bytes) else {
            return MetaRaw::Absent;
        };

        for item in walk {
            // A defect is the end of the header region for this purpose,
            // not a reason to throw away what was in it — see above.
            let Ok(element) = item else { break };
            match element {
                jpeg::Element::Framed { marker, payload }
                    if marker == APP1 && payload.starts_with(EXIF_IDENTIFIER) =>
                {
                    return MetaRaw::Captured(payload.to_vec());
                }
                // The picture starts here and the metadata did not turn
                // up before it, which is the ordinary case in this
                // corpus rather than a defect.
                jpeg::Element::Scan(_) => break,
                _ => {}
            }
        }
        MetaRaw::Absent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX;
    use asterism_core::domain::content_region::EMPTY_SPAN;
    // The gates and the two public readings: what a caller reaches, and
    // what every assertion below goes through, so that a refusal this
    // probe does not write is still asserted where it is now decided.
    use asterism_core::domain::probe::ProbeGates;

    /// The markers the fixtures are assembled from, **spelled out here
    /// instead of borrowed from the implementation**.
    ///
    /// A test that reuses the probe's constants agrees with them by
    /// construction, which is the one thing a fixture builder must not
    /// do: point `APP1` at a different byte and every variant below
    /// would keep building whatever the implementation now calls an
    /// EXIF block, and keep passing.
    const APP0_TAG: u8 = 0xE0;
    const APP1_TAG: u8 = 0xE1;
    const APP2_TAG: u8 = 0xE2;
    const APP13_TAG: u8 = 0xED;
    const DQT_TAG: u8 = 0xDB;
    const SOF0_TAG: u8 = 0xC0;
    const DHT_TAG: u8 = 0xC4;
    const DRI_TAG: u8 = 0xDD;
    const SOS_TAG: u8 = 0xDA;
    const COM_TAG: u8 = 0xFE;
    const EOI_TAG: u8 = 0xD9;

    /// `0xFF` + marker + the length the payload implies + the payload.
    fn framed(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, marker];
        let declared = u16::try_from(payload.len() + 2).expect("fixture payload fits a length");
        out.extend_from_slice(&declared.to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn bare(marker: u8) -> Vec<u8> {
        vec![0xFF, marker]
    }

    /// An `APP1` carrying an EXIF block: the identifier, then whatever
    /// stands in for the TIFF structure.
    fn exif(body: &[u8]) -> Vec<u8> {
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(body);
        framed(APP1_TAG, &payload)
    }

    /// An `APP1` carrying an XMP packet.
    fn xmp(body: &str) -> Vec<u8> {
        let mut payload = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
        payload.extend_from_slice(body.as_bytes());
        framed(APP1_TAG, &payload)
    }

    /// A TIFF structure as an EXIF block carries one: little-endian, a
    /// single IFD, and only the fields a case names.
    ///
    /// **Built rather than encoded, because one field of it is now
    /// read.** Every fixture in this module used to put arbitrary bytes
    /// behind `Exif\0\0` — which was honest while the whole block was
    /// excluded and is not any more: a case that says "these two differ
    /// only in their timestamp" has to produce a block a reader agrees
    /// is a timestamp, or it is asserting something about two
    /// unparseable blocks.
    ///
    /// `orientation` is the 1–8 code (`None` for a block that states
    /// none) and `datetime` is TIFF's `DateTime`, chosen for the
    /// control cases because it is the metadata a re-save rewrites and
    /// it changes nothing about the picture.
    fn exif_block(orientation: Option<u16>, datetime: Option<&str>) -> Vec<u8> {
        // Tag, type, count, and the bytes the value either is or points
        // at. In tag order, which is what an IFD is required to be in.
        let mut fields: Vec<(u16, u16, u32, Vec<u8>)> = Vec::new();
        if let Some(code) = orientation {
            fields.push((0x0112, 3, 1, code.to_le_bytes().to_vec()));
        }
        if let Some(stamp) = datetime {
            let mut ascii = stamp.as_bytes().to_vec();
            ascii.push(0);
            fields.push((0x0132, 2, ascii.len() as u32, ascii));
        }

        // Header, then the IFD at offset 8: an entry count, twelve
        // bytes each, and a link to the next IFD. A value wider than
        // the four-byte field lands behind all of that and is pointed
        // at from inside it.
        let data_at = 8 + 2 + fields.len() * 12 + 4;
        let mut ifd = (fields.len() as u16).to_le_bytes().to_vec();
        let mut data: Vec<u8> = Vec::new();
        for (tag, kind, count, value) in &fields {
            ifd.extend_from_slice(&tag.to_le_bytes());
            ifd.extend_from_slice(&kind.to_le_bytes());
            ifd.extend_from_slice(&count.to_le_bytes());
            if value.len() <= 4 {
                let mut inline = value.clone();
                inline.resize(4, 0);
                ifd.extend_from_slice(&inline);
            } else {
                ifd.extend_from_slice(&((data_at + data.len()) as u32).to_le_bytes());
                data.extend_from_slice(value);
            }
        }
        ifd.extend_from_slice(&0u32.to_le_bytes());

        let mut out = b"II\x2a\0\x08\0\0\0".to_vec();
        out.extend_from_slice(&ifd);
        out.extend_from_slice(&data);
        out
    }

    /// The `APP1` an [`exif_block`] travels in.
    fn exif_tiff(orientation: Option<u16>, datetime: Option<&str>) -> Vec<u8> {
        exif(&exif_block(orientation, datetime))
    }

    /// One IFD entry as the file carries it: the tag, the **type code**,
    /// the count, and the value's own bytes.
    ///
    /// The type is a number here and not a name, which is the point of
    /// the fixture: what the meta cases below measure is the mapping
    /// from a code in the file to a marker in the stored value, and a
    /// builder that named its types would be agreeing with the
    /// implementation's vocabulary by construction rather than writing
    /// twelve bytes and asking what came back.
    struct Entry {
        tag: u16,
        kind: u16,
        count: u32,
        value: Vec<u8>,
    }

    /// Type 2, `ASCII` — the count includes the terminating NUL, which
    /// is the format's rule and not this builder's convenience.
    fn ascii(tag: u16, text: &str) -> Entry {
        let mut value = text.as_bytes().to_vec();
        value.push(0);
        Entry {
            tag,
            kind: 2,
            count: value.len() as u32,
            value,
        }
    }

    /// Type 3, `SHORT`.
    fn short(tag: u16, value: u16) -> Entry {
        Entry {
            tag,
            kind: 3,
            count: 1,
            value: value.to_le_bytes().to_vec(),
        }
    }

    /// Type 4, `LONG`.
    fn long(tag: u16, value: u32) -> Entry {
        Entry {
            tag,
            kind: 4,
            count: 1,
            value: value.to_le_bytes().to_vec(),
        }
    }

    /// Type 5, `RATIONAL` — two 32-bit numbers per element, and **never
    /// a fraction this builder reduces**: the pairs go in as written, so
    /// a case can state `2/4` and ask what comes back.
    fn rational(tag: u16, pairs: &[(u32, u32)]) -> Entry {
        let mut value = Vec::new();
        for (num, denom) in pairs {
            value.extend_from_slice(&num.to_le_bytes());
            value.extend_from_slice(&denom.to_le_bytes());
        }
        Entry {
            tag,
            kind: 5,
            count: pairs.len() as u32,
            value,
        }
    }

    /// Type 7, `UNDEFINED` — bytes the format assigns no reading to,
    /// which is what a maker note is.
    fn undefined(tag: u16, bytes: &[u8]) -> Entry {
        Entry {
            tag,
            kind: 7,
            count: bytes.len() as u32,
            value: bytes.to_vec(),
        }
    }

    /// A type code this crate's reader has no parser for, so the value
    /// arrives as `Unknown`.
    fn of_unknown_type(tag: u16, kind: u16) -> Entry {
        Entry {
            tag,
            kind,
            count: 1,
            value: vec![0xde, 0xad, 0xbe, 0xef],
        }
    }

    /// A whole TIFF structure with up to four IFDs in it: the primary
    /// one, the Exif and GPS sub-IFDs it points at, and the thumbnail
    /// IFD behind it.
    ///
    /// **The sub-IFDs are what make the key's namespace measurable.** A
    /// tag number alone says nothing about which of these it sat in, and
    /// the four are four namespaces (`ifd0`, `exif`, `gps`, `ifd1`), so
    /// a fixture with one IFD in it could not tell a key that carries
    /// the namespace from one that does not.
    ///
    /// Little-endian, values wider than four bytes in a data area behind
    /// every IFD, and the pointers computed rather than hand-placed —
    /// which is what lets a case add a field without every offset in the
    /// file having to be re-derived by hand.
    fn tiff(ifd0: Vec<Entry>, exif_ifd: Vec<Entry>, gps: Vec<Entry>, ifd1: Vec<Entry>) -> Vec<u8> {
        fn size(entries: usize) -> usize {
            2 + entries * 12 + 4
        }

        // The pointer entries are part of IFD0's own count, so they are
        // counted before any offset is computed.
        let pointers = usize::from(!exif_ifd.is_empty()) + usize::from(!gps.is_empty());
        let at_exif = 8 + size(ifd0.len() + pointers);
        let at_gps = at_exif
            + if exif_ifd.is_empty() {
                0
            } else {
                size(exif_ifd.len())
            };
        let at_ifd1 = at_gps + if gps.is_empty() { 0 } else { size(gps.len()) };
        let data_at = at_ifd1 + if ifd1.is_empty() { 0 } else { size(ifd1.len()) };

        let mut ifd0 = ifd0;
        if !exif_ifd.is_empty() {
            ifd0.push(long(0x8769, at_exif as u32));
        }
        if !gps.is_empty() {
            ifd0.push(long(0x8825, at_gps as u32));
        }

        fn ifd_bytes(entries: &[Entry], next: u32, data_at: usize, data: &mut Vec<u8>) -> Vec<u8> {
            let mut out = (entries.len() as u16).to_le_bytes().to_vec();
            for entry in entries {
                out.extend_from_slice(&entry.tag.to_le_bytes());
                out.extend_from_slice(&entry.kind.to_le_bytes());
                out.extend_from_slice(&entry.count.to_le_bytes());
                if entry.value.len() <= 4 {
                    let mut inline = entry.value.clone();
                    inline.resize(4, 0);
                    out.extend_from_slice(&inline);
                } else {
                    out.extend_from_slice(&((data_at + data.len()) as u32).to_le_bytes());
                    data.extend_from_slice(&entry.value);
                }
            }
            out.extend_from_slice(&next.to_le_bytes());
            out
        }

        let mut data: Vec<u8> = Vec::new();
        let next_ifd = if ifd1.is_empty() { 0 } else { at_ifd1 as u32 };
        let mut out = b"II\x2a\0\x08\0\0\0".to_vec();
        out.extend_from_slice(&ifd_bytes(&ifd0, next_ifd, data_at, &mut data));
        for block in [&exif_ifd, &gps, &ifd1] {
            if !block.is_empty() {
                out.extend_from_slice(&ifd_bytes(block, 0, data_at, &mut data));
            }
        }
        out.extend_from_slice(&data);
        out
    }

    /// What the pass would store in `meta_kv` for these bytes.
    fn meta_kv(bytes: &[u8]) -> std::collections::BTreeMap<String, String> {
        match JpegProbe.meta(bytes, Some(&jpeg_mime())) {
            MaterialMeta::Digest { canonical, .. } => {
                serde_json::from_str(&canonical).expect("the canonical form is a JSON object")
            }
            other => panic!("expected a reading, got {other:?}"),
        }
    }

    /// An `APP2` carrying an ICC profile chunk: sequence header, then
    /// the profile bytes.
    fn icc(profile: &[u8]) -> Vec<u8> {
        let mut payload = b"ICC_PROFILE\0".to_vec();
        payload.extend_from_slice(&[1, 1]);
        payload.extend_from_slice(profile);
        framed(APP2_TAG, &payload)
    }

    fn build(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        for part in parts {
            out.extend_from_slice(part);
        }
        out
    }

    /// The coding tables and frame header a photograph carries between
    /// its application segments and its scan — the part of the file that
    /// two exports of one picture share exactly.
    fn coding_tables() -> Vec<Vec<u8>> {
        vec![
            framed(DQT_TAG, &[0x00, 16, 11, 10, 16, 24, 40, 51, 61]),
            framed(SOF0_TAG, &[0x08, 0, 64, 0, 64, 3, 1, 0x22, 0, 2, 0x11, 1]),
            framed(DHT_TAG, &[0x00, 0, 1, 5, 1, 1, 1, 1, 1]),
            framed(DRI_TAG, &[0x00, 0x08]),
        ]
    }

    /// 512 bytes of deterministic entropy-coded data, **with a stuffed
    /// `0xFF` and a restart marker in it**.
    ///
    /// The stuffing is not decoration. A walk that read `0xFF 0x00` as a
    /// marker would end the scan at the first one, so every fixture here
    /// would be hashed over its first few bytes — and that is the shape
    /// in which two different photographs quietly become one duplicate
    /// group. Putting it in the *shared* scan means every assertion in
    /// this module is taken over bytes that would move if the walk
    /// stopped early.
    fn entropy(seed: u32) -> Vec<u8> {
        let mut state = seed;
        let mut out: Vec<u8> = Vec::new();
        for at in 0..512 {
            if at == 100 {
                // A literal 0xFF inside the scan, as an encoder writes it.
                out.extend_from_slice(&[0xFF, 0x00]);
                continue;
            }
            if at == 300 {
                // RST0 — an encoder emits one every restart interval,
                // which is why the DRI segment above is there.
                out.extend_from_slice(&[0xFF, 0xD0]);
                continue;
            }
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let byte = (state >> 24) as u8;
            // Anything the generator produces that would itself look
            // like a marker is nudged, so the two above are the only
            // ones and a failure names which of them was misread.
            out.push(if byte == 0xFF { 0xFE } else { byte });
        }
        out
    }

    /// A photograph: JFIF header, coding tables, one scan, end marker.
    ///
    /// `extras` go between the `APP0` and the tables, which is where a
    /// camera and an editor both write their application segments.
    fn photo(extras: &[Vec<u8>], seed: u32) -> Vec<u8> {
        let mut parts = vec![framed(APP0_TAG, b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0")];
        parts.extend_from_slice(extras);
        parts.extend(coding_tables());
        parts.push(framed(
            SOS_TAG,
            &[0x03, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0],
        ));
        parts.push(entropy(seed));
        parts.push(bare(EOI_TAG));
        build(&parts)
    }

    /// `len` bytes standing in for the MP4 a Motion Photo appends.
    ///
    /// Shaped like one — an ISOBMFF box header, then filler — and read
    /// by nothing, because the region takes the tail as an opaque span.
    /// What makes it usable as a fixture is that it is deterministic and
    /// that two lengths give two different blobs.
    fn mp4_ish(len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        out.extend_from_slice(&(len as u32).to_be_bytes());
        out.extend_from_slice(b"ftypmp42");
        let mut state = len as u32 | 1;
        while out.len() < len {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            out.push((state >> 24) as u8);
        }
        out.truncate(len);
        out
    }

    fn mime(raw: &str) -> MimeType {
        MimeType::parse(raw)
    }

    fn jpeg_mime() -> MimeType {
        mime("image/jpeg")
    }

    fn region(bytes: &[u8], declared: Option<&MimeType>) -> ContentRegion {
        JpegProbe.content(bytes, declared)
    }

    fn digest_of(bytes: &[u8]) -> String {
        match region(bytes, Some(&jpeg_mime())) {
            ContentRegion::Digest(value) => value,
            other => panic!("expected a digest, got {other:?}"),
        }
    }

    /// **What this probe answers for, written out rather than read from
    /// [`CLAIMS`].**
    ///
    /// A test that asked the constant would agree with it by
    /// construction and would keep passing whatever was added to it —
    /// and what gets added is the failure worth catching. The registry
    /// cannot catch it: a probe's declaration *is* the list of formats
    /// the build covers, so a format added here is a format the registry
    /// covers, and completeness has nothing to compare.
    ///
    /// The axes are asserted with the mime rather than after it, because
    /// dropping one is the quiet half of the same edit: the column stops
    /// being computed and every row of the format keeps whatever it
    /// already had.
    #[test]
    fn this_probe_declares_image_jpeg_on_both_axes_and_nothing_else() {
        let declared: Vec<(&str, bool, bool)> = JpegProbe
            .declares()
            .iter()
            .map(|claim| (claim.mime.as_str(), claim.content, claim.meta))
            .collect();
        assert_eq!(
            declared,
            vec![("image/jpeg", true, true)],
            "this probe's segment walk, its signature check and its EXIF reading are \
             JPEG's; anything else claimed here is read whole and then answered wrongly"
        );
    }

    // ---- the three excluded segments -----------------------------------

    /// **The reason this slice exists: two exports of one photograph
    /// that differ only in their EXIF are one picture.**
    ///
    /// Same pixels, different camera metadata — the state a corpus
    /// arrives in after any tool that rewrites timestamps, strips a
    /// GPS tag or re-saves through an editor. If this does not hold,
    /// nothing here has been achieved.
    ///
    /// The control is at the end: the same two files with their scans
    /// made different, which have to come apart. Without it a probe that
    /// hashed nothing at all would satisfy the first half.
    #[test]
    fn two_photographs_differing_only_in_exif_are_one_picture() {
        let stripped = photo(&[], 0x1111_2222);
        let dated = photo(&[exif_tiff(None, Some("2019:04:01 09:12:33"))], 0x1111_2222);
        let redated = photo(&[exif_tiff(None, Some("2024:11:30 22:05:01"))], 0x1111_2222);

        assert_ne!(stripped, dated, "the files have to differ");
        assert_ne!(dated, redated, "the files have to differ");
        assert_eq!(digest_of(&stripped), digest_of(&dated));
        assert_eq!(digest_of(&dated), digest_of(&redated));

        // And an upright tag is the same picture as no tag, which is
        // the normalisation the orientation feed is written around:
        // three ways of saying nothing about rotation, one value.
        assert_eq!(
            digest_of(&photo(&[exif_tiff(Some(1), None)], 0x1111_2222)),
            digest_of(&stripped),
            "an explicit Orientation=1 and no EXIF at all render alike"
        );
        assert_eq!(
            digest_of(&photo(
                &[exif_tiff(Some(1), Some("2019:04:01 09:12:33"))],
                0x1111_2222
            )),
            digest_of(&stripped)
        );

        // The control: one different photograph, everything else equal.
        assert_ne!(
            digest_of(&photo(
                &[exif_tiff(None, Some("2019:04:01 09:12:33"))],
                0x3333_4444
            )),
            digest_of(&dated),
            "a different scan is a different picture"
        );
    }

    /// The same walk with EXIF excluded whole — the region as it was
    /// defined before the orientation was pulled out of it.
    ///
    /// A measuring instrument on the model of [`digest_excluding_icc`],
    /// not a candidate implementation. Two differences from the probe
    /// and no others: nothing reads the file's orientation, and an XMP
    /// packet is excluded whatever it says.
    ///
    /// It feeds the upright byte rather than no byte, which the pre-fix
    /// code did not do. That is the same rule shifted by one constant —
    /// a walk that never looks answers "upright" for every file — and
    /// having the byte there lets the control below compare this
    /// against the probe directly instead of against a second offset.
    fn digest_excluding_all_of_exif(bytes: &[u8]) -> String {
        let mut hasher = ContentHasher::new();
        hasher.update(&[1]);
        for item in jpeg::segments(bytes).expect("fixture is a JPEG") {
            match item.expect("fixture walks to a complete image") {
                jpeg::Element::Bare(marker) => hasher.update(&[marker]),
                jpeg::Element::Framed { marker, payload } => {
                    // The wholesale rule: an EXIF block is metadata,
                    // all of it, whatever it says.
                    if marker == COM_TAG
                        || (marker == APP1_TAG
                            && (payload.starts_with(b"Exif\0\0")
                                || payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0")))
                    {
                        continue;
                    }
                    let declared = u16::try_from(payload.len() + 2).expect("a JPEG length");
                    hasher.update(&[marker]);
                    hasher.update(&declared.to_be_bytes());
                    hasher.update(payload);
                }
                jpeg::Element::Scan(entropy) => super::feed_span(&mut hasher, entropy),
                jpeg::Element::Trailing(appended) => super::feed_span(&mut hasher, appended),
            }
        }
        super::under_region_tag(hasher)
    }

    /// **A photograph and its EXIF-stripped copy are two pictures when
    /// the tag said the frame was rotated.**
    ///
    /// This is the failure the wholesale exclusion had. The two files
    /// below carry the same entropy-coded data byte for byte — one
    /// tagged `Orientation=6`, one with no EXIF, which is a phone
    /// photograph and the same photograph through any stripper. A viewer
    /// shows one portrait and one on its side. Merge them and the fold
    /// leaves a tombstone where the one that displayed correctly was,
    /// and there is then a single picture on screen, so nobody can spot
    /// the loss by looking at it.
    ///
    /// The last block is the instrument: under the rule that excluded
    /// the block whole, the same pair collapses onto one digest.
    /// [measured: replacing the orientation feed with a constant lands both
    /// on `cr1-sha256:f5b94f89…` and fails the first assertion below.]
    #[test]
    fn two_photographs_differing_only_in_their_orientation_are_two_pictures() {
        let stripped = photo(&[], 0xf0f0_0f0f);
        let rotated = photo(&[exif_tiff(Some(6), None)], 0xf0f0_0f0f);
        let upright = photo(&[exif_tiff(Some(1), None)], 0xf0f0_0f0f);

        assert_ne!(
            digest_of(&rotated),
            digest_of(&stripped),
            "a transposed frame and an untagged one are not the same picture"
        );
        assert_ne!(digest_of(&rotated), digest_of(&upright));
        assert_eq!(
            digest_of(&upright),
            digest_of(&stripped),
            "upright and untagged are, though"
        );

        // Every code the format assigns is its own rendering, so every
        // one of them is its own digest — mirrored (2–4) as much as
        // transposed (5–8).
        let digests: std::collections::BTreeSet<String> = (1..=8)
            .map(|code| digest_of(&photo(&[exif_tiff(Some(code), None)], 0xf0f0_0f0f)))
            .collect();
        assert_eq!(digests.len(), 8, "eight renderings, eight digests");

        // A code outside the eight names no rendering, so it is the
        // same answer as no tag at all rather than a ninth picture.
        assert_eq!(
            digest_of(&photo(&[exif_tiff(Some(42), None)], 0xf0f0_0f0f)),
            digest_of(&stripped)
        );

        // The instrument, and the shape of the mistake. First where the
        // two rules have to agree: a file with nothing rotated to
        // disagree about, or what follows compares two harnesses rather
        // than two rules.
        assert_eq!(
            digest_excluding_all_of_exif(&stripped),
            digest_of(&stripped),
            "with no orientation stated the two rules have to agree"
        );
        // …and then the merge: under the wholesale exclusion the
        // rotated file lands on the stripped file's digest, so the
        // photograph and every export of it that lost its EXIF are one
        // duplicate group.
        assert_eq!(
            digest_excluding_all_of_exif(&rotated),
            digest_excluding_all_of_exif(&stripped),
            "the pair the wholesale exclusion merged"
        );
        assert_eq!(
            digest_excluding_all_of_exif(&rotated),
            digest_excluding_all_of_exif(&upright),
        );
    }

    /// An XMP packet is the other `APP1`, and an `APP1` that is neither
    /// is content.
    ///
    /// The second half is the denylist's own direction made visible: the
    /// exclusion is keyed on the payload's identifier, so a vendor
    /// segment sharing the marker is hashed rather than assumed to be
    /// metadata.
    #[test]
    fn an_xmp_packet_is_excluded_and_an_unrecognised_app1_is_not() {
        let bare_photo = photo(&[], 0x5555_6666);
        let rated = photo(
            &[xmp(r#"<x:xmpmeta><xmp:Rating>3</xmp:Rating></x:xmpmeta>"#)],
            0x5555_6666,
        );
        let rerated = photo(
            &[xmp(r#"<x:xmpmeta><xmp:Rating>5</xmp:Rating></x:xmpmeta>"#)],
            0x5555_6666,
        );

        assert_ne!(rated, rerated, "the files have to differ");
        assert_eq!(digest_of(&bare_photo), digest_of(&rated));
        assert_eq!(digest_of(&rated), digest_of(&rerated));

        // An APP1 whose payload announces neither: content, because
        // nothing here knows what it is.
        let vendor = photo(
            &[framed(APP1_TAG, b"AcmeCam\0some private block")],
            0x5555_6666,
        );
        assert_ne!(
            digest_of(&vendor),
            digest_of(&bare_photo),
            "an APP1 nobody recognises stays in the region"
        );

        // …and the extended-XMP identifier is deliberately not the one
        // above, so it stays in too. A change that started matching it
        // moves this assertion rather than a duplicate group.
        let extended = photo(
            &[framed(
                APP1_TAG,
                b"http://ns.adobe.com/xmp/extension/\0a continuation packet",
            )],
            0x5555_6666,
        );
        assert_ne!(digest_of(&extended), digest_of(&bare_photo));
    }

    /// **An XMP packet that mentions `tiff:Orientation` is not excluded,
    /// because it might be about the rendering and nothing here reads
    /// it.**
    ///
    /// XMP is the other place an orientation travels. Parsing it would
    /// mean an XML reader deciding what a picture is, so the exclusion
    /// is withdrawn on a substring instead: the packet stays in the
    /// region whole, and two files differing in one are two pictures.
    ///
    /// That over-counts — a packet mentioning the property while saying
    /// nothing else different is kept too, and the last assertion is
    /// that case, stated rather than hidden. It is the direction this
    /// selection is allowed to fail in: an improvement lost, not a
    /// duplicate invented.
    #[test]
    fn an_xmp_packet_that_could_rotate_the_picture_stays_in_the_region() {
        let plain = photo(&[], 0xaced_1234);
        let sideways = photo(&[xmp(r#"<x:xmpmeta tiff:Orientation="6"/>"#)], 0xaced_1234);
        let upright = photo(&[xmp(r#"<x:xmpmeta tiff:Orientation="1"/>"#)], 0xaced_1234);

        assert_ne!(
            digest_of(&sideways),
            digest_of(&plain),
            "a packet that may transpose the frame is not a note about it"
        );
        assert_ne!(
            digest_of(&sideways),
            digest_of(&upright),
            "and two of them stating different codes are two pictures"
        );

        // The cost, admitted: the property is mentioned and nothing
        // else differs, and the two still come apart — because the rule
        // is "might this be about the rendering", answered without
        // reading it.
        let noted = photo(
            &[xmp(
                r#"<x:xmpmeta><rdf:Description tiff:Orientation="1" xmp:Rating="3"/></rdf:Description></x:xmpmeta>"#,
            )],
            0xaced_1234,
        );
        let renoted = photo(
            &[xmp(
                r#"<x:xmpmeta><rdf:Description tiff:Orientation="1" xmp:Rating="5"/></rdf:Description></x:xmpmeta>"#,
            )],
            0xaced_1234,
        );
        assert_ne!(digest_of(&noted), digest_of(&renoted));
    }

    /// A `COM` segment is a note written about the picture.
    #[test]
    fn two_photographs_differing_only_in_a_comment_are_one_picture() {
        let plain = photo(&[], 0x7777_8888);
        let noted = photo(&[framed(COM_TAG, b"exported for print")], 0x7777_8888);
        let renoted = photo(&[framed(COM_TAG, b"exported for the web")], 0x7777_8888);

        assert_ne!(noted, renoted, "the files have to differ");
        assert_eq!(digest_of(&plain), digest_of(&noted));
        assert_eq!(digest_of(&noted), digest_of(&renoted));
    }

    // ---- the segment that must not be excluded --------------------------

    /// The same walk with exactly one line changed: `APP2` is metadata.
    ///
    /// Not a candidate implementation — a measuring instrument, on the
    /// model of `png::tests::ancillary_digest`. The module doc claims
    /// that excluding the ICC profile merges two pictures a viewer shows
    /// differently; this makes the claim a value two assertions can
    /// compare, so it cannot quietly stop being true. Everything else is
    /// copied deliberately (the other two exclusions, the length, the
    /// scan handling, the tag) so that the only thing the comparison can
    /// be measuring is whether `APP2` is inside the region.
    fn digest_excluding_icc(bytes: &[u8]) -> String {
        let mut hasher = ContentHasher::new();
        hasher.update(&[jpeg::orientation(bytes).unwrap_or(1)]);
        for item in jpeg::segments(bytes).expect("fixture is a JPEG") {
            match item.expect("fixture walks to a complete image") {
                jpeg::Element::Bare(marker) => hasher.update(&[marker]),
                jpeg::Element::Framed { marker, payload } => {
                    if is_metadata(marker, payload) || marker == APP2_TAG {
                        continue;
                    }
                    let declared = u16::try_from(payload.len() + 2).expect("a JPEG length");
                    hasher.update(&[marker]);
                    hasher.update(&declared.to_be_bytes());
                    hasher.update(payload);
                }
                jpeg::Element::Scan(entropy) => super::feed_span(&mut hasher, entropy),
                jpeg::Element::Trailing(appended) => super::feed_span(&mut hasher, appended),
            }
        }
        super::under_region_tag(hasher)
    }

    /// **An ICC profile is part of the picture, and this is the test
    /// that says so.**
    ///
    /// Identical pixels under two profiles are two pictures: the profile
    /// decides what the numbers are rendered as, so an sRGB export and a
    /// Display-P3 one look different on the same screen. Excluding
    /// `APP2` would give them one digest, and downstream a fold turns
    /// the loser of a duplicate group into a tombstone — the exact
    /// failure the PNG probe measured on this repo's corpus with
    /// colour-management chunks.
    ///
    /// Three assertions, and the third is the one that makes this more
    /// than a preference. It shows the *shape* of the mistake: under the
    /// excluding rule the two files land on the digest of the file with
    /// no profile at all, so the whole family collapses into one
    /// duplicate group.
    #[test]
    fn an_icc_profile_is_part_of_the_picture() {
        let none = photo(&[], 0x9999_aaaa);
        let srgb = photo(&[icc(b"sRGB IEC61966-2.1 curve data")], 0x9999_aaaa);
        let p3 = photo(&[icc(b"Display P3 ......... curve data")], 0x9999_aaaa);

        assert_ne!(srgb, p3, "the files have to differ");
        assert_ne!(
            digest_of(&srgb),
            digest_of(&p3),
            "two profiles over one pixel stream are two pictures"
        );
        assert_ne!(digest_of(&srgb), digest_of(&none));

        // Where the two rules agree: with no APP2 to disagree about, the
        // instrument and the probe have to answer alike, or what follows
        // is measuring a difference between two harnesses rather than
        // between two rules.
        assert_eq!(
            digest_excluding_icc(&none),
            digest_of(&none),
            "with no profile present the two rules have to agree"
        );

        // And the shape of the difference: the excluding rule collapses
        // all three onto one digest.
        assert_eq!(digest_excluding_icc(&srgb), digest_excluding_icc(&p3));
        assert_eq!(digest_excluding_icc(&srgb), digest_excluding_icc(&none));
    }

    /// Everything else a JPEG carries stays in the region.
    ///
    /// One assertion per segment kind rather than one over a file with
    /// all of them, so a rule that started dropping exactly one is named
    /// by the failure instead of hidden in a single moved digest.
    #[test]
    fn the_segments_nobody_excluded_are_all_in_the_region() {
        let baseline = photo(&[], 0xbbbb_cccc);
        for (label, extra) in [
            (
                "APP13, Photoshop's resource block",
                framed(APP13_TAG, b"Photoshop 3.0\08BIM"),
            ),
            (
                "an APP4 nobody here has an opinion about",
                framed(0xE4, b"a private block"),
            ),
            ("a second DQT", framed(DQT_TAG, &[0x01, 17, 18, 24, 47, 99])),
            ("a second DHT", framed(DHT_TAG, &[0x10, 0, 2, 1, 3, 3, 2])),
        ] {
            let variant = photo(&[extra], 0xbbbb_cccc);
            assert_ne!(variant, baseline, "{label}: the file has to differ");
            assert_ne!(digest_of(&variant), digest_of(&baseline), "{label}");
        }
    }

    /// **The declared length is fed to the hash, and PNG's is not.**
    ///
    /// The fixtures are one `APP4` whose payload begins with the byte
    /// `0xE4`, against two `APP4`s — an empty one and one carrying the
    /// rest. With the lengths omitted both feed `E4 E4 01`: the first
    /// segment's payload spells the second segment's marker, which is
    /// the collision the length exists to remove.
    ///
    /// **The fixture has to be that exact, and the one that stood here
    /// before was not.** It used `APP4[1,2,3] + APP4[4,5,6]` against
    /// `APP4[1,2] + APP4[3,4,5,6]`, which feed `E4 01 02 03 E4 04 05 06`
    /// and `E4 01 02 E4 03 04 05 06` with no lengths at all — the marker
    /// byte sits in a different place, so they differ either way and
    /// deleting `hasher.update(&declared.to_be_bytes())` left the
    /// assertion passing. A test that cannot fail for the reason its
    /// name gives is worse than no test, because it is counted.
    /// [measured: with the length feed deleted the old fixture passes and
    /// this one fails on `assert_ne!`.]
    ///
    /// This is also the assertion that a later tidy-up — "make it
    /// consistent with the PNG probe" — has to argue with. The module
    /// doc holds the argument; this holds the measurement.
    #[test]
    fn the_length_separates_two_segments_that_would_otherwise_run_together() {
        let one_segment = photo(&[framed(0xE4, &[0xE4, 0x01])], 0xdddd_eeee);
        let two_segments = photo(&[framed(0xE4, &[]), framed(0xE4, &[0x01])], 0xdddd_eeee);
        assert_ne!(one_segment, two_segments, "the files have to differ");
        assert_ne!(
            digest_of(&one_segment),
            digest_of(&two_segments),
            "one segment whose payload spells a marker, against the two segments \
             it would otherwise be indistinguishable from"
        );
    }

    /// **The scan carries its length too, and the file does not give it
    /// one.**
    ///
    /// Entropy-coded data is delimited by the marker behind it rather
    /// than by a length field, and the `0xFF` that introduces that
    /// marker is not fed — so with the scan fed bare, its trailing bytes
    /// can spell out the element that follows it.
    ///
    /// The fixtures are the pair review measured it on: a scan ending
    /// `11 E4 00 04 AA BB` and then `EOI`, against a scan ending `11`
    /// followed by a real `APP4` carrying `AA BB` and then `EOI`. Two
    /// different pictures — the entropy streams differ by five bytes —
    /// and one digest before the length was fed. [measured: with
    /// `feed_span`'s length line deleted, both files here reach
    /// `cr1-sha256:8f1f6b49…` and this assertion fails; review's own
    /// build of the same shape reached `cr1-sha256:615d9b69…`, the
    /// difference being the entropy prefix each chose.]
    #[test]
    fn the_scans_length_separates_it_from_the_segment_behind_it() {
        let head: Vec<u8> = (0..64).map(|at| (at * 11 % 251) as u8).collect();

        let mut spelled_out = head.clone();
        spelled_out.extend_from_slice(&[0x11, 0xE4, 0x00, 0x04, 0xAA, 0xBB]);
        let inside_the_scan = build(&[
            framed(SOS_TAG, &[0x01, 1, 0x00, 0, 63, 0]),
            spelled_out,
            bare(EOI_TAG),
        ]);

        let mut short = head.clone();
        short.push(0x11);
        let as_a_segment = build(&[
            framed(SOS_TAG, &[0x01, 1, 0x00, 0, 63, 0]),
            short,
            framed(0xE4, &[0xAA, 0xBB]),
            bare(EOI_TAG),
        ]);

        // The files differ by one byte — the `0xFF` that introduces the
        // marker, which is in every marker and is therefore never fed.
        // That is the whole of what used to separate them.
        assert_eq!(inside_the_scan.len() + 1, as_a_segment.len());
        assert_ne!(inside_the_scan, as_a_segment, "the files have to differ");
        assert_ne!(
            digest_of(&inside_the_scan),
            digest_of(&as_a_segment),
            "a scan's tail must not be able to spell the element behind it"
        );
    }

    // ---- the scan -------------------------------------------------------

    /// **The scan is walked past everything that merely looks like a
    /// marker.**
    ///
    /// A `0xFF 0x00` stuffing pair and an `RSTn` both appear in ordinary
    /// encoder output — the first every time a coefficient byte comes
    /// out `0xFF`, the second every restart interval — and a walk that
    /// read either as the end of the scan would report the prefix
    /// before it as the whole of the picture.
    ///
    /// **Two assertions, guarding two different mistakes, and the first
    /// is the one this implementation actually makes.** Stop at a
    /// stuffing pair here and the walk resumes at `0xFF 0x00`, which
    /// `take_marker` refuses because `0x00` is not a marker; stop at an
    /// `RSTn` and it resumes in the middle of entropy data, whose next
    /// byte is not `0xFF`. Either way the file loses its region
    /// entirely rather than getting a short one — every JPEG with a
    /// stuffed byte in it, which is very nearly all of them, would store
    /// `unsupported:empty-span`. That is an improvement lost rather than
    /// a duplicate invented, and it is not luck: it is the scan walker
    /// and `take_marker`'s strictness holding together. [measured: mutating
    /// `scan_length`'s stuffing arm to `return Ok(at)`, and separately
    /// its restart arm, both land this test on
    /// `expected a digest, got EmptySpan`.]
    ///
    /// The second guards the version of the mistake that would collide:
    /// a walk that stopped early and then **resynchronised** — skipped
    /// to the next plausible marker instead of refusing — would hand
    /// back a short span, and two photographs agreeing up to their first
    /// stuffed byte would be handed one digest and folded. The fixtures
    /// differ only *after* that byte, so they are the pair such a walk
    /// would merge.
    #[test]
    fn a_scan_is_read_past_its_stuffing_and_its_restart_markers() {
        let head: Vec<u8> = (0..100).map(|at| (at * 7 % 251) as u8).collect();

        let mut with_stuffing = head.clone();
        with_stuffing.extend_from_slice(&[0xFF, 0x00]);
        with_stuffing.extend_from_slice(b"the tail after a stuffed byte");
        let mut differing_after_stuffing = head.clone();
        differing_after_stuffing.extend_from_slice(&[0xFF, 0x00]);
        differing_after_stuffing.extend_from_slice(b"a different tail entirely!!!!");

        let mut with_restart = head.clone();
        with_restart.extend_from_slice(&[0xFF, 0xD3]);
        with_restart.extend_from_slice(b"the tail after a restart marker");
        let mut differing_after_restart = head.clone();
        differing_after_restart.extend_from_slice(&[0xFF, 0xD3]);
        differing_after_restart.extend_from_slice(b"a different tail entirely!!!!!");

        let walked = |entropy: &[u8]| {
            let mut parts = vec![framed(APP0_TAG, b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0")];
            parts.extend(coding_tables());
            parts.push(framed(SOS_TAG, &[0x01, 1, 0x00, 0, 63, 0]));
            parts.push(entropy.to_vec());
            parts.push(bare(EOI_TAG));
            region(&build(&parts), Some(&jpeg_mime()))
        };
        let scanned = |entropy: &[u8]| match walked(entropy) {
            ContentRegion::Digest(value) => value,
            other => panic!("expected a digest, got {other:?}"),
        };

        // First: a scan carrying either of them walks to a region at
        // all. A walk that stopped at the pair reaches `EmptySpan`
        // here, so this is the assertion, not a formality.
        for (label, entropy) in [
            ("a stuffed 0xFF", &with_stuffing),
            ("a restart marker", &with_restart),
        ] {
            assert!(
                matches!(walked(entropy), ContentRegion::Digest(_)),
                "{label}: a scan carrying one is still a region, and this file lost its"
            );
        }

        // Second: and the bytes behind it are in that region, so a walk
        // that stopped early and resynchronised rather than refusing
        // would be caught here instead of in a duplicate group.
        assert_ne!(
            scanned(&with_stuffing),
            scanned(&differing_after_stuffing),
            "a walk that reported only the bytes before the 0xFF 0x00 pair \
             would give these two one digest"
        );
        assert_ne!(
            scanned(&with_restart),
            scanned(&differing_after_restart),
            "and one that stopped at RST3 would do the same"
        );

        // The control, which is what stops the two assertions above from
        // being satisfied by a probe that hashes the whole file blindly:
        // the same tails with the marker pair taken out are still told
        // apart, and the pairs themselves are inside the region, so a
        // walk that *dropped* them rather than stopping at them is
        // caught too.
        let mut restart_removed = head.clone();
        restart_removed.extend_from_slice(b"the tail after a restart marker");
        assert_ne!(
            scanned(&with_restart),
            scanned(&restart_removed),
            "the restart marker is scan data, not punctuation to be skipped"
        );
    }

    /// A JPEG whose walk reached the end and found no entropy-coded
    /// bytes gets a marker, not a digest over what was left.
    ///
    /// The failure this prevents is precise: `EOI` is in the region and
    /// is in every JPEG, so two stubs carrying nothing else but
    /// different EXIF blocks would hash the same single byte and be
    /// offered to the user as the same picture. The last two assertions
    /// are that pair.
    #[test]
    fn a_jpeg_with_no_scan_gets_a_marker_not_a_digest_over_its_headers() {
        let header_only = build(&[
            framed(APP0_TAG, b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0"),
            bare(EOI_TAG),
        ]);
        let walked = region(&header_only, Some(&jpeg_mime()));
        assert_eq!(walked, ContentRegion::EmptySpan);
        assert_eq!(walked.stored_value(), EMPTY_SPAN);
        assert!(walked.digest().is_none());
        assert!(!walked.stored_value().starts_with(CONTENT_DIGEST_PREFIX));

        // An SOS with nothing behind it is the same answer: a scan of
        // zero bytes is not a picture.
        let empty_scan = build(&[
            framed(APP0_TAG, b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0"),
            framed(SOS_TAG, &[0x01, 1, 0x00, 0, 63, 0]),
            bare(EOI_TAG),
        ]);
        assert_eq!(
            region(&empty_scan, Some(&jpeg_mime())),
            ContentRegion::EmptySpan
        );

        // The pair the rule exists for: two metadata-only stubs that a
        // digest would have merged.
        let one = build(&[exif(b"II*\0\x08\0\0\0 one"), bare(EOI_TAG)]);
        let two = build(&[exif(b"II*\0\x08\0\0\0 two"), bare(EOI_TAG)]);
        assert_ne!(one, two, "the files have to differ");
        assert_eq!(region(&one, Some(&jpeg_mime())), ContentRegion::EmptySpan);
        assert_eq!(region(&two, Some(&jpeg_mime())), ContentRegion::EmptySpan);
    }

    // ---- refusals and defects -------------------------------------------

    #[test]
    fn bytes_that_are_not_a_jpeg_are_not_walked() {
        let real = photo(&[], 0x0101_0202);

        // The mime says one thing, the bytes say another; both
        // directions refuse, and each says as much as it knows.
        assert_eq!(
            region(&real, Some(&mime("image/png"))),
            ContentRegion::Unsupported("image/png".to_string())
        );
        assert_eq!(
            region(b"\x89PNG\r\n\x1a\n and the rest", Some(&jpeg_mime())),
            ContentRegion::Unsupported(UNKNOWN_FORMAT.to_string())
        );
        assert_eq!(
            region(b"plain text, no claim", None),
            ContentRegion::Unsupported(UNKNOWN_FORMAT.to_string())
        );
        // Two bytes of signature and nothing behind them is not a JPEG
        // either — a marker has to follow the SOI.
        assert_eq!(
            region(&[0xFF, 0xD8], Some(&jpeg_mime())),
            ContentRegion::Unsupported(UNKNOWN_FORMAT.to_string())
        );

        // A parameterised or shouted mime is the same claim — settled at
        // the parse boundary rather than by this probe.
        assert!(matches!(
            region(&real, Some(&mime("IMAGE/JPEG; charset=binary"))),
            ContentRegion::Digest(_)
        ));
    }

    #[test]
    fn a_broken_jpeg_falls_to_a_marker_without_trusting_its_lengths() {
        let intact = photo(&[], 0x0303_0404);

        // A length that runs past the end of the file. The APP0 sits
        // right behind the SOI, so its length field is at offset 4.
        let mut overrun = intact.clone();
        overrun[4..6].copy_from_slice(&0xfff0u16.to_be_bytes());
        assert_eq!(
            region(&overrun, Some(&jpeg_mime())),
            ContentRegion::EmptySpan
        );

        // A length below the two bytes it occupies itself.
        let mut impossible = intact.clone();
        impossible[4..6].copy_from_slice(&1u16.to_be_bytes());
        assert_eq!(
            region(&impossible, Some(&jpeg_mime())),
            ContentRegion::EmptySpan
        );

        // Cut in half: the scan runs off the end of the file, and a walk
        // that returned what it had would have handed back a prefix
        // every truncation of this photograph shares.
        let truncated = &intact[..intact.len() / 2];
        assert_eq!(
            region(truncated, Some(&jpeg_mime())),
            ContentRegion::EmptySpan
        );

        // Segments that never reach EOI.
        let unterminated = &intact[..intact.len() - 2];
        assert_eq!(
            region(unterminated, Some(&jpeg_mime())),
            ContentRegion::EmptySpan
        );

        // A byte where a marker belongs — after a first segment that
        // walked, so this is a defect in the sequence rather than a
        // signature the probe refuses before reading anything. The APP0
        // is 18 bytes behind the SOI, so the next marker starts here.
        let mut stray = intact.clone();
        assert_eq!(stray[20], 0xFF, "the second segment starts where expected");
        stray[20] = 0x00;
        assert_eq!(region(&stray, Some(&jpeg_mime())), ContentRegion::EmptySpan);

        // More segments than the walk will follow. `TEM` carries no
        // length, so this is the cheapest swarm a file can hold.
        let mut swarm: Vec<Vec<u8>> = (0..=jpeg::MAX_SEGMENTS).map(|_| bare(0x01)).collect();
        swarm.push(framed(SOS_TAG, &[0x01, 1, 0x00, 0, 63, 0]));
        swarm.push(entropy(1));
        swarm.push(bare(EOI_TAG));
        assert_eq!(
            region(&build(&swarm), Some(&jpeg_mime())),
            ContentRegion::EmptySpan
        );
    }

    /// **What follows the end marker is part of the artefact, and a
    /// phone puts a video there.**
    ///
    /// Google and Samsung Motion Photos are a complete JPEG with a
    /// complete MP4 appended past `EOI`. Excluding the tail gives a
    /// Motion Photo and its still-only export one digest, and the fold
    /// that follows takes the video with the row — measured on the
    /// smallest possible pair below, a 598-byte still against the same
    /// still with 4 KB behind it.
    ///
    /// This assertion used to run the other way, and the doc it carried
    /// argued that stopping at `EOI` was the point. It was the defect.
    /// [measured: with the walk restored to `done = true` at `EOI`, the two
    /// files below both reach `cr1-sha256:741f2565…`.]
    ///
    /// The `COM` in the tail is the second half: `COM` is excluded
    /// *inside* the image, and these bytes are not inside the image —
    /// they are an opaque span that nothing parses, so a comment-shaped
    /// run of bytes in it counts like any other. The last block is that
    /// pair, and it is the one a rule which walked the tail as segments
    /// would fail.
    #[test]
    fn what_follows_the_end_marker_reaches_the_region() {
        let intact = photo(&[], 0x0505_0606);

        let mut motion_photo = intact.clone();
        motion_photo.extend_from_slice(&mp4_ish(4096));
        assert_eq!(
            intact.len(),
            598,
            "the still, so the sizes in this doc stay true"
        );
        assert_ne!(
            digest_of(&motion_photo),
            digest_of(&intact),
            "a still and the same still carrying two seconds of video are two artefacts"
        );

        // Two different videos behind one still are two artefacts too —
        // otherwise "the tail is in the region" could be satisfied by a
        // walk that only noticed whether there was one.
        let mut other_video = intact.clone();
        other_video.extend_from_slice(&mp4_ish(4097));
        assert_ne!(digest_of(&motion_photo), digest_of(&other_video));

        // And the tail is bytes, not structure: a `COM` segment is
        // excluded inside the image and counts behind it, because
        // behind `EOI` nothing is read as a segment at all.
        let mut commented = intact.clone();
        commented.extend_from_slice(&framed(COM_TAG, b"past the end of the image"));
        assert_ne!(digest_of(&commented), digest_of(&intact));
        let inside = photo(
            &[framed(COM_TAG, b"past the end of the image")],
            0x0505_0606,
        );
        assert_eq!(
            digest_of(&inside),
            digest_of(&intact),
            "the same bytes inside the image are still a note about it"
        );
    }

    // ---- the two axes, over one file ------------------------------------

    /// **One file, two axes, two different sentences — through the
    /// registry a caller actually uses.**
    ///
    /// The two readings select opposite halves of one container: the
    /// content digest is taken with the EXIF `APP1` excluded, and the
    /// meta digest is taken over that same segment's contents. So the
    /// assertion is not merely that both answer, but that **the same
    /// bytes move one and not the other** — the last block edits the
    /// EXIF and watches the meta digest move while the content digest
    /// stays where it was. A probe that fed the block to both would fail
    /// there and nowhere else.
    #[test]
    fn the_two_axes_read_opposite_halves_of_one_file() {
        use crate::probes;

        let block = tiff(
            vec![ascii(0x010F, "ACME"), short(0x0112, 1)],
            vec![rational(0x829A, &[(1, 125)])],
            vec![],
            vec![],
        );
        let bytes = photo(&[exif(&block)], 0x0707_0808);
        let declared = jpeg_mime();

        assert!(probes::walks_content(Some(&declared)));
        assert!(probes::walks_meta(Some(&declared)));

        let content = probes::content(&bytes, Some(&declared));
        assert!(
            matches!(content, ContentRegion::Digest(_)),
            "got {content:?}"
        );
        assert!(content.stored_value().starts_with(CONTENT_DIGEST_PREFIX));

        let meta = probes::meta(&bytes, Some(&declared));
        assert!(matches!(meta, MaterialMeta::Digest { .. }), "got {meta:?}");
        assert!(meta.stored_value().starts_with("m1-"));

        // The registry's answer and the probe's own are one value — the
        // registry picks by the declared mime and nothing else, so a
        // probe reached directly must not answer differently.
        assert_eq!(content, JpegProbe.content(&bytes, Some(&declared)));
        assert_eq!(meta, JpegProbe.meta(&bytes, Some(&declared)));

        // The same photograph out of a camera that spells its name
        // differently: one axis moves, the other does not, and that is
        // the whole of what two axes over one container means.
        let other = photo(
            &[exif(&tiff(
                vec![ascii(0x010F, "OTHER"), short(0x0112, 1)],
                vec![rational(0x829A, &[(1, 125)])],
                vec![],
                vec![],
            ))],
            0x0707_0808,
        );
        assert_eq!(
            probes::content(&other, Some(&declared)),
            content,
            "the maker's name is not part of the picture"
        );
        assert_ne!(
            probes::meta(&other, Some(&declared)),
            meta,
            "and it is part of what the container carried"
        );

        // And PNG's rows are untouched by any of it: the registry routes
        // by format, so adding a probe must not move the other one's
        // gates.
        let png = mime("image/png");
        assert!(probes::walks_content(Some(&png)));
        assert!(probes::walks_meta(Some(&png)));
    }

    // ---- the meta axis ---------------------------------------------------

    /// **Every field arrives under the type the file gave it, addressed
    /// by the IFD it sat in.**
    ///
    /// Four types over four namespaces, asserted as exact strings rather
    /// than as properties, because both halves of the value are load
    /// bearing and a property test would let either drift: the marker is
    /// what stops `1/125` being ambiguous between a rational and an
    /// ASCII tag whose text is literally that, and the rendering is what
    /// the digest is taken over.
    ///
    /// **Teeth**: every assertion here is an equality against a spelling
    /// no other part of this file produces, so a reading that dropped
    /// the marker, reduced the fraction, decoded the maker note or left
    /// the IFD out of the key fails on the entry it changed and names
    /// it.
    #[test]
    fn every_tag_arrives_with_the_type_the_file_gave_it() {
        let block = tiff(
            vec![ascii(0x010F, "ACME"), short(0x0112, 6)],
            vec![
                rational(0x829A, &[(1, 125)]),
                undefined(0x927C, b"Apple iOS\0\0\x01MM"),
            ],
            vec![ascii(0x0001, "N")],
            vec![short(0x0103, 6)],
        );
        let fields = meta_kv(&photo(&[exif(&block)], 0x4444_5555));

        assert_eq!(
            fields.get("ifd0:0x010f").map(String::as_str),
            Some("ascii:ACME")
        );
        assert_eq!(
            fields.get("ifd0:0x0112").map(String::as_str),
            Some("short:6")
        );
        assert_eq!(
            fields.get("exif:0x829a").map(String::as_str),
            Some("rational:1/125"),
        );
        assert_eq!(
            fields.get("exif:0x927c").map(String::as_str),
            // Base64 of the bytes, undecoded — which is what makes Apple's
            // `BurstUUID` reachable later without re-reading the file.
            Some("undefined:QXBwbGUgaU9TAAABTU0="),
        );
        assert_eq!(
            fields.get("gps:0x0001").map(String::as_str),
            Some("ascii:N")
        );
        assert_eq!(
            fields.get("ifd1:0x0103").map(String::as_str),
            Some("short:6")
        );

        // Six fields and no seventh: nothing here filters, and the three
        // entries that do not arrive are the sub-IFD pointers, which the
        // reader follows rather than reports. That narrowing is the
        // reader's and is worth pinning where it shows — the whole
        // block came back, the addresses of the sub-blocks did not, and
        // their contents are above under their own namespaces.
        assert_eq!(fields.len(), 6, "{fields:?}");
        assert!(!fields.contains_key("ifd0:0x8769"), "{fields:?}");
        assert!(!fields.contains_key("ifd0:0x8825"), "{fields:?}");

        // A type this reader has no parser for is recorded rather than
        // dropped — the field was there, and a file that carries it is
        // not the same file as one that does not.
        let odd = meta_kv(&photo(
            &[exif(&tiff(
                vec![ascii(0x010F, "ACME"), of_unknown_type(0x0110, 0x00ff)],
                vec![],
                vec![],
                vec![],
            ))],
            0x4444_5555,
        ));
        assert!(
            odd.get("ifd0:0x0110")
                .is_some_and(|value| value.starts_with("unknown:255/1@")),
            "{odd:?}"
        );
    }

    /// **A rational survives unreduced.**
    ///
    /// `2/4` is not `1/2` here, and the reason is not tidiness: a
    /// reduction is a division, and two of the values real files carry
    /// have none. A lens whose minimum focal length is unrecorded writes
    /// `0/0`, and `1/0` occurs beside it — reduce either and the field
    /// stops saying what the file said.
    ///
    /// **Teeth**: adding a `gcd` to the rational arm turns the first
    /// assertion red (`rational:2/4` → `rational:1/2`) and panics on the
    /// second pair, since neither reduces to anything.
    #[test]
    fn a_rational_is_not_reduced() {
        let fields = meta_kv(&photo(
            &[exif(&tiff(
                vec![rational(0x011A, &[(2, 4)])],
                vec![
                    // A four-element rational, which is how a lens states
                    // its range — and two of the four are the fractions
                    // no reduction can express.
                    rational(0xA432, &[(24, 1), (70, 1), (0, 0), (1, 0)]),
                ],
                vec![],
                vec![],
            ))],
            0x6666_7777,
        ));

        assert_eq!(
            fields.get("ifd0:0x011a").map(String::as_str),
            Some("rational:2/4"),
            "two quarters is what the file said"
        );
        assert_eq!(
            fields.get("exif:0xa432").map(String::as_str),
            Some("rational:24/1,70/1,0/0,1/0"),
            "a count of four is four fractions, comma-separated, none of them divided"
        );

        // And the distinction a reduction would destroy: two files whose
        // fields reduce alike are two files.
        let halved = meta_kv(&photo(
            &[exif(&tiff(
                vec![rational(0x011A, &[(1, 2)])],
                vec![rational(0xA432, &[(24, 1), (70, 1), (0, 0), (1, 0)])],
                vec![],
                vec![],
            ))],
            0x6666_7777,
        ));
        assert_ne!(halved.get("ifd0:0x011a"), fields.get("ifd0:0x011a"));
    }

    /// **One tag number in two IFDs is two fields.**
    ///
    /// `0x0112` in IFD0 is how the photograph is rotated; the same
    /// number in IFD1 is how the *thumbnail* is. A key built from the
    /// tag number alone would put them in one slot, so a photograph and
    /// its thumbnail disagreeing about their orientation would come back
    /// as a single field with whichever value the walk reached last.
    ///
    /// **Teeth**: dropping the namespace from
    /// [`exif_tags`](asterism_media_probe::exif_tags)'s key leaves one
    /// entry where this expects two, and the map's length is asserted so
    /// the collapse cannot hide behind a value that happens to match.
    #[test]
    fn one_tag_number_in_two_ifds_is_two_fields() {
        let fields = meta_kv(&photo(
            &[exif(&tiff(
                vec![short(0x0112, 6)],
                vec![],
                vec![],
                vec![short(0x0112, 1)],
            ))],
            0x8888_9999,
        ));

        assert_eq!(fields.len(), 2, "{fields:?}");
        assert_eq!(
            fields.get("ifd0:0x0112").map(String::as_str),
            Some("short:6")
        );
        assert_eq!(
            fields.get("ifd1:0x0112").map(String::as_str),
            Some("short:1")
        );

        // The pair that collapses if the namespace goes: the same two
        // numbers the other way round. Under a key of the tag number
        // alone both files carry one entry, and which value survives is
        // whichever IFD the walk reached last — so the two files would
        // differ in the map only by accident.
        let swapped = meta_kv(&photo(
            &[exif(&tiff(
                vec![short(0x0112, 1)],
                vec![],
                vec![],
                vec![short(0x0112, 6)],
            ))],
            0x8888_9999,
        ));
        assert_ne!(swapped, fields);
    }

    /// **The raw column walks back to the `meta_kv` column** — the
    /// whole reason the bytes are kept.
    ///
    /// The kept bytes are an `APP1` payload, so what reads them is a
    /// JPEG: the signature, the segment put back in its own framing, and
    /// an end marker. That is a **complete file** rather than a fragment
    /// of one, which is where this differs from
    /// [`png`](super::png)'s round trip — a PNG's kept chunks need a
    /// terminator supplied and are not a picture, while a JPEG's EXIF
    /// segment reassembles into a JPEG that is simply missing its image.
    ///
    /// Every step goes through the surface a later consumer would use —
    /// `material_meta_raw::bytes_of` for the decode, the media probe's
    /// own `exif_tags` for the reading — rather than through a reader
    /// written here to agree with the writer.
    ///
    /// **Teeth**: `expect` on the column rather than `if let`, so a
    /// probe that stopped keeping the bytes fails here instead of
    /// passing over an empty round trip.
    #[test]
    fn the_raw_column_walks_back_to_the_meta_kv_column() {
        use asterism_core::domain::material_meta_raw::{self, MetaRaw};

        let block = tiff(
            vec![ascii(0x010F, "ACME"), short(0x0112, 6)],
            vec![
                rational(0x829A, &[(1, 125)]),
                undefined(
                    0x927C,
                    b"Apple iOS\0\0\x01MM a maker note nobody here opens",
                ),
            ],
            vec![ascii(0x0001, "N")],
            vec![short(0x0103, 6)],
        );
        let bytes = photo(&[exif(&block)], 0xaaaa_bbbb);

        let stored = JpegProbe
            .meta_raw(&bytes, Some(&jpeg_mime()))
            .stored_value()
            .expect("the probe kept no bytes for a JPEG carrying EXIF");
        let raw = material_meta_raw::bytes_of(&stored).expect("the column decodes");

        // The payload as the file carries it, identifier and all — so
        // putting it back in a segment produces something a reader
        // opens without being told what the bytes are.
        assert!(
            raw.starts_with(b"Exif\0\0"),
            "{:?}",
            &raw[..8.min(raw.len())]
        );
        let mut walkable = vec![0xFF, 0xD8];
        walkable.extend_from_slice(&framed(APP1_TAG, &raw));
        walkable.extend_from_slice(&bare(EOI_TAG));

        let walked = asterism_media_probe::exif_tags(&walkable)
            .expect("the kept payload reads back as an EXIF block");
        assert_eq!(
            walked,
            meta_kv(&bytes),
            "the raw renders to the same map the pass stored"
        );
        assert!(!walked.is_empty());

        // A subset of the file rather than the file: a reading that kept
        // everything would satisfy the round trip and put the picture in
        // the row.
        assert!(raw.len() < bytes.len() / 2, "{} bytes", raw.len());

        // And the two columns are one statement about one region: a
        // truncated file whose header survives keeps both, where a PNG
        // in the same state keeps neither. The cut is inside the scan,
        // past every segment.
        let truncated = &bytes[..bytes.len() - 200];
        assert_eq!(
            JpegProbe.meta(truncated, Some(&jpeg_mime())),
            JpegProbe.meta(&bytes, Some(&jpeg_mime())),
            "the reading is of the header region, which is intact"
        );
        assert_eq!(
            JpegProbe.meta_raw(truncated, Some(&jpeg_mime())),
            MetaRaw::Captured(raw),
            "so the bytes it was taken over are kept too"
        );
        // …while the content axis, which is about the part that was cut,
        // says it has no region.
        assert_eq!(
            region(truncated, Some(&jpeg_mime())),
            ContentRegion::EmptySpan
        );
    }

    /// A JPEG with no EXIF says a reading ran and found nothing, which
    /// is not the same sentence as "nobody looked".
    ///
    /// This is most of the corpus rather than an edge: of 250 JPEGs
    /// sampled from a real download directory, 246 carried no EXIF at
    /// all. The distinction matters because
    /// `unsupported:image/jpeg` is a **final** answer to whether anybody
    /// looked — the ordinary pass never returns to a row holding it — so
    /// a probe that answered it here would leave every ordinary JPEG in
    /// the state the V76 migration exists to get rows out of.
    #[test]
    fn a_jpeg_with_no_exif_says_a_reading_ran_and_found_nothing() {
        use asterism_core::domain::material_meta_raw::MetaRaw;

        let plain = photo(&[], 0xcccc_dddd);
        assert_eq!(
            JpegProbe.meta(&plain, Some(&jpeg_mime())),
            MaterialMeta::EmptySpan
        );
        assert_eq!(
            JpegProbe.meta(&plain, Some(&jpeg_mime())).stored_value(),
            EMPTY_SPAN
        );
        assert_eq!(
            JpegProbe.meta_raw(&plain, Some(&jpeg_mime())),
            MetaRaw::Absent,
            "no EXIF segment is no bytes, and NULL is what that says"
        );

        // An APP1 that is not EXIF is not a reading either: the XMP
        // packet this probe keeps in the content region carries no
        // fields for this axis, and nothing pretends otherwise.
        let xmp_only = photo(
            &[xmp(r#"<x:xmpmeta><xmp:Rating>3</xmp:Rating></x:xmpmeta>"#)],
            0xcccc_dddd,
        );
        assert_eq!(
            JpegProbe.meta(&xmp_only, Some(&jpeg_mime())),
            MaterialMeta::EmptySpan
        );
        assert_eq!(
            JpegProbe.meta_raw(&xmp_only, Some(&jpeg_mime())),
            MetaRaw::Absent
        );

        // Bytes that are not a JPEG's are refused on this axis the way
        // they are on the other, and the marker names the format the row
        // claimed rather than the one the bytes are.
        assert_eq!(
            JpegProbe.meta(b"\x89PNG\r\n\x1a\n and the rest", Some(&jpeg_mime())),
            MaterialMeta::Unsupported(UNKNOWN_FORMAT.to_string())
        );
        assert_eq!(
            JpegProbe.meta_raw(b"\x89PNG\r\n\x1a\n and the rest", Some(&jpeg_mime())),
            MetaRaw::Absent
        );
        assert_eq!(
            JpegProbe.meta(&plain, Some(&mime("image/png"))),
            MaterialMeta::Unsupported("image/png".to_string()),
            "an axis this probe did not declare for this format is the port's refusal"
        );
    }

    // ---- the frozen value ------------------------------------------------

    /// **The content digest of a photograph carrying every excluded
    /// segment and every included one.**
    ///
    /// Every other assertion in this module is *relative* — these two
    /// agree, that one moves — and a probe that fed the hasher nothing
    /// at all would satisfy a great many of them at once. This literal
    /// is the absolute anchor, and its subject is not the code: it is the
    /// strings that will sit in a Dogfood database's
    /// `material.content_region_hash` column for every JPEG imported
    /// from here on. A build that produces a different one cannot be
    /// compared against the rows it has already written.
    ///
    /// **It was measured against this implementation**, and there is no
    /// way for it to have been measured against another: this is the
    /// first reading of a JPEG's content region in this repo, so unlike
    /// the PNG literals — which were computed before their walk moved
    /// crates and did not move with it — this one cannot be a comparison
    /// of new code with older code. What it pins is everything from here
    /// forward. A change to it is a `cr2-` decision — a new prefix and a
    /// re-walk of every stored row, see
    /// [`NOT_WALKED`](asterism_core::domain::content_region::NOT_WALKED)
    /// — never a refactor. A diff that edits this literal to make a test
    /// pass has inverted the reason it exists.
    ///
    /// **It moved once, before it pinned anything.** Review found four
    /// defects in the region definition — the orientation, the trailing
    /// span, the undelimited scan, and a length test that could not
    /// fail — and all four were fixed while `image/jpeg` had never been
    /// claimed, so no database anywhere held a JPEG content digest. That
    /// window is what made re-deriving these literals a free edit rather
    /// than a migration, and it is closed the moment a build carrying
    /// them writes a row.
    const PHOTO_REGION: &str =
        "cr1-sha256:5672085d77400efa1e3e6474781067ebd5eba455281e17731ff6502363a1d795";

    /// The same photograph, tagged `Orientation=6`.
    ///
    /// One of the two things the frozen pair pins that a single literal
    /// could not: the orientation is *in* the region, and every code
    /// gets its own value rather than a flag saying "rotated somehow".
    const PHOTO_REGION_ROTATED: &str =
        "cr1-sha256:3aa61d34e4c1afd1fa9c4be8afd7e05a30250a3e0f5134cc70f4105dc3b7ba83";

    /// The same photograph with a Motion Photo's video behind its `EOI`.
    ///
    /// The other one: the trailing span is in the region. A build that
    /// went back to stopping at `EOI` would land this on
    /// [`PHOTO_REGION`], which is the collision that costs a user their
    /// video.
    const PHOTO_REGION_WITH_VIDEO: &str =
        "cr1-sha256:501678846285b56014e36de4855080351ec7026b63440d8a85cd7f0175d027d3";

    /// The three literals, and the photograph with no metadata segments
    /// at all — which has to reach the *same* value as the loaded one,
    /// since the excluded segments contribute nothing.
    ///
    /// One literal appearing in two places rather than a claim that it
    /// would.
    #[test]
    fn the_region_definition_produces_this_exact_digest() {
        let notes = |orientation: Option<u16>| {
            let extras = vec![
                exif_tiff(orientation, Some("2019:04:01 09:12:33")),
                xmp(r#"<x:xmpmeta><xmp:Rating>3</xmp:Rating></x:xmpmeta>"#),
                icc(b"sRGB IEC61966-2.1 curve data"),
                framed(COM_TAG, b"exported for print"),
                framed(APP13_TAG, b"Photoshop 3.0\08BIM"),
            ];
            photo(&extras, 0x1234_5678)
        };
        let loaded = notes(Some(1));
        let rotated = notes(Some(6));
        let stripped = photo(
            &[
                icc(b"sRGB IEC61966-2.1 curve data"),
                framed(APP13_TAG, b"Photoshop 3.0\08BIM"),
            ],
            0x1234_5678,
        );
        let mut with_video = loaded.clone();
        with_video.extend_from_slice(&mp4_ish(4096));

        assert_ne!(loaded, stripped, "the files have to differ");
        assert_eq!(digest_of(&loaded), PHOTO_REGION, "the loaded photograph");
        assert_eq!(
            digest_of(&stripped),
            PHOTO_REGION,
            "and the same picture with its notes removed"
        );
        assert_eq!(
            digest_of(&rotated),
            PHOTO_REGION_ROTATED,
            "the same picture, shown on its side"
        );
        assert_eq!(
            digest_of(&with_video),
            PHOTO_REGION_WITH_VIDEO,
            "the same picture with two seconds of video behind it"
        );

        for value in [PHOTO_REGION, PHOTO_REGION_ROTATED, PHOTO_REGION_WITH_VIDEO] {
            let hex = value
                .strip_prefix(CONTENT_DIGEST_PREFIX)
                .expect("the value declares its region version");
            assert_eq!(hex.len(), 64);
            assert!(
                hex.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
        }
        assert_eq!(
            [PHOTO_REGION, PHOTO_REGION_ROTATED, PHOTO_REGION_WITH_VIDEO]
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3,
            "three files a viewer shows differently, three values"
        );
    }
}
