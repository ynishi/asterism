//! PNG's reading of the two walking axes: which of its chunks are the
//! picture, and which are notes written about it.
//!
//! The chunk framing — where one chunk ends and the next begins, what a
//! `tEXt` payload decodes to — is
//! [`asterism_media_probe::png`](asterism_media_probe::png), and it has
//! no opinion about any of this. What is here is the opinion: a
//! judgement about *this* corpus, argued from measurements taken in this
//! repo, which is why it sits beside the application rather than in a
//! crate that walks PNGs in general.
//!
//! # From a slice, and what that costs the caller
//!
//! Both readings take `&[u8]`: the caller already holds the bytes.
//! `ContentHasher` streams for a reason — an original can be a 4 GB
//! video and reading it into memory would be the process's largest
//! allocation — and that reason still stands, but it does not reach
//! here, for two reasons.
//!
//! The first is the region's shape. The pixel stream is fed last, as one
//! concatenation, so that the same compressed data split across a
//! different number of `IDAT` chunks hashes the same (see below). A
//! reader would therefore have to either hold every `IDAT` payload in
//! memory until the end — the allocation we were avoiding, in a worse
//! form, since it is the largest part of the file — or seek back and
//! read the file a second time. A slice makes the second pass free: it
//! is a second walk over memory the caller already paid for.
//!
//! The second is the corpus. This is the PNG probe; PNGs here run to a
//! few megabytes (a real character card is a few hundred KB), and the 4 GB
//! case is video, which needs a completely different reading (mp4 sample
//! tables) that will take a reader rather than a slice. Sizing this
//! probe's signature against a file type it does not handle would buy
//! nothing and cost the second pass.
//!
//! So: **a 4 GB PNG would be a 4 GB allocation in the caller**, before
//! either method is entered. That decision belongs to the job that opens
//! the file — it is the one that knows the file's size before reading
//! it, and the one that can answer "too big to fingerprint" by storing
//! [`TOO_LARGE`](asterism_core::domain::content_region::TOO_LARGE). This
//! module never opens anything, so it cannot make that call; what it
//! guarantees is that once the bytes are in hand it adds no allocation
//! proportional to them.
//!
//! # Which chunks are content
//!
//! Everything except metadata. The excluded set is `tEXt`, `zTXt`,
//! `iTXt`, `tIME`, `eXIf`, plus the structural `IEND`.
//!
//! A denylist, not an allowlist of the chunks known to matter, and the
//! reason is the one written into the port's own contract
//! ([`ArtefactProbe::content_of`]): the two are wrong in opposite
//! directions. An allowlist drops any chunk nobody thought of: a private
//! chunk, a type added to the spec later, or — measured, not imagined —
//! the colour-management chunks and APNG frame data, where two visibly
//! different pictures came out with one digest. That error is a false
//! positive, and downstream a fold turns the loser into a tombstone,
//! which is not something the user can undo by looking at it again. A
//! denylist's error runs the other way: forget to exclude something and
//! two files that differ only in metadata get separate digests, which is
//! exactly what happens with no content axis at all. One failure loses
//! data, the other loses an improvement, so the unknown chunk goes on
//! the side that loses the improvement.
//!
//! # What is fed to the hash
//!
//! Every included chunk contributes `type (4 bytes) || payload`, in the
//! order it appears; all `IDAT` payloads are concatenated and fed once,
//! last, behind a single `IDAT` tag.
//!
//! **The chunk's length field is never fed.** Encoders split the same
//! compressed stream at different boundaries — zlib's buffer size is not
//! part of the image — and hashing the lengths would make one picture
//! written by two encoders two pictures. Measured: the same stream in 1,
//! 8 and 63 chunks produces one digest, and a real ComfyUI corpus writes
//! its pixels as 17–24 chunks of 64 KiB, so this is the ordinary case
//! rather than an adversarial one.
//!
//! The type *is* fed, because a payload alone does not say which chunk
//! it was: two files carrying the same four bytes under different chunk
//! types would otherwise collide.
//!
//! Neither the order nor the presence of a chunk is assumed. The real
//! fixture in this repo carries its `tEXt` chunks **after** the pixel
//! data; a ComfyUI export carries one on each side (the prompt before,
//! the workflow after), and files in the same corpus have the second one
//! missing entirely. Anything that treated "metadata comes first", or
//! "these two chunks are always there", would break on ordinary files.
//! Selection is by type and nothing else.
//!
//! # Which chunks are metadata
//!
//! Only `tEXt` is read. It is the chunk this corpus's metadata actually
//! travels in, and it is the one the reader that defines the form
//! (`asterism_media_probe::png::text_fields`, a `BTreeMap<String,
//! String>` keyed by chunk keyword) returns. `zTXt` is compressed and
//! `iTXt` may be; `tIME` and `eXIf` are binary, not text. Reading them
//! means deciding how each decodes to a string *before* the digest means
//! anything, and a decision made carelessly there is a redefinition of
//! the axis, not a widening of it. They are excluded from the content
//! region as well, so **neither digest is about them** — a stated gap
//! rather than a silent one, and one that a `m2-` generation would
//! close.
//!
//! # Which chunks are kept
//!
//! All five, as bytes. The gap above stays a gap in what is *hashed*,
//! and stops being a loss: [`meta_raw_of`](PngProbe::meta_raw_of) keeps
//! the frames of every chunk in [`METADATA_CHUNKS`] verbatim, so a later
//! generation can decide how a `zTXt` decompresses, or read the Latin-1
//! bytes an accented `tEXt` lost to `from_utf8_lossy`, **without opening
//! the file again** ([`material_meta_raw`](asterism_core::domain::material_meta_raw)).
//!
//! The list is the same list on purpose. What the content digest drops
//! and what the raw keeps are one sentence about this container — the
//! bytes that are notes rather than picture — and stating it twice is
//! how the two stop agreeing. So "widen the denylist" and "keep more"
//! are one edit, and a chunk added to the five is excluded from the
//! digest *and* recoverable from the row that day.
//!
//! # Every structural defect is one outcome
//!
//! The walk distinguishes truncation from a lying length from a missing
//! `IEND` from too many chunks, and all four land on
//! [`ContentRegion::EmptySpan`]. The variants are worth having anyway —
//! a reader of a stack trace or a future diagnostic can tell which
//! happened — but the stored value must not fork on them, because the
//! true statement they share is the one the column carries: there is no
//! complete region to stand behind. Splitting them into separate markers
//! was argued down where the marker is defined
//! ([`EMPTY_SPAN`](asterism_core::domain::content_region::EMPTY_SPAN)),
//! and doing it here instead would put the same vocabulary in two
//! places.

use super::under_region_tag;
use asterism_core::domain::content_hash::ContentHasher;
use asterism_core::domain::content_region::{ContentRegion, UNKNOWN_FORMAT};
use asterism_core::domain::material_meta::{self, MaterialMeta};
use asterism_core::domain::material_meta_raw::MetaRaw;
use asterism_core::domain::probe::{ArtefactProbe, FormatClaim, GateOpen};
use asterism_core::domain::value::{ImageFormat, MimeType};
use asterism_media_probe::png;

/// The pixel stream's chunk type.
const IDAT: [u8; 4] = *b"IDAT";

/// End of the chunk sequence — structural, and in neither axis.
const IEND: [u8; 4] = *b"IEND";

/// The chunks excluded from the content region — everything a PNG
/// carries *about* the image rather than *of* it.
///
/// The list is short on purpose (see the module's denylist note): a
/// chunk missing from it is included in the digest, which is the
/// direction that fails safely.
///
/// # Why not PNG's ancillary bit
///
/// The crate that walks the chunks offers PNG's own split — critical
/// chunks (uppercase first letter) versus ancillary ones — and it looks
/// like this list with the maintenance removed. It is not. Ancillary
/// means "a decoder may skip this and still produce an image", which is
/// a statement about decoder obligations, not about what the image looks
/// like: `gAMA`, `sRGB`, `iCCP`, `cHRM` and `sBIT` are all ancillary and
/// all change what the viewer sees, and `acTL` / `fcTL` / `fdAT` — every
/// frame of an APNG after the first — are ancillary too. Both were
/// measured: two files differing only in `gAMA`, and two APNGs whose
/// second frame is all `0x00` against all `0xff`, come out with one
/// digest under the ancillary rule.
///
/// That failure is the one this axis must not make. "Same picture" said
/// about two different pictures ends with a fold turning one of them
/// into a tombstone, and the user cannot undo it by looking again. The
/// five names below are a claim about *these specific chunks carrying no
/// pixels*, which is why they have to be named one by one and why the
/// list is allowed to be incomplete: a metadata chunk nobody remembered
/// is hashed, and two files that differ only in it get two digests — the
/// behaviour of having no content axis at all.
///
/// `tests::pngs_own_ancillary_split_is_not_this_probes_excluded_five`
/// keeps that difference measured rather than asserted.
const METADATA_CHUNKS: [[u8; 4]; 5] = [*b"tEXt", *b"zTXt", *b"iTXt", *b"tIME", *b"eXIf"];

/// Most metadata bytes this probe will keep for one file, before base64
/// — past it the row stores
/// [`MetaRaw::TooLarge`](asterism_core::domain::material_meta_raw::MetaRaw::TooLarge)
/// instead.
///
/// **PNG is the format that needs a number chosen.** Every other
/// ceiling in this area is read off something: the size gate in
/// `fingerprint` is about this process's memory, `png::MAX_CHUNKS`
/// bounds a list of borrowed payloads, and JPEG's metadata cannot pass
/// 64 KiB because an `APP1` segment's length field is two bytes. PNG
/// bounds nothing usable — 65,536 chunks of up to 2^31-1 bytes each —
/// so a file whose `tEXt` runs to 60 MiB passes the 64 MiB gate and
/// would put 80 MiB of base64 in one column.
///
/// 1 MiB, against a measured worst case of **40,339 bytes** [measured: the
/// two `tEXt` frames of the character card this repo ships, by
/// `tests::the_ceiling_is_measured_against_the_largest_thing_this_corpus_carries`,
/// which freezes the number so the argument here cannot quietly stop
/// being about it — 40,315 bytes of payload plus 24 bytes of framing for
/// the two chunks]. That file is the worst case in reach because a
/// character card carries a whole persona document, base64 inside a
/// `tEXt`; the other thing that travels in these chunks is a ComfyUI
/// workflow, tens of kilobytes across a 4,601-image corpus. The ceiling
/// is 25 times the measurement, which is what makes it a bound on the
/// pathological case rather than a policy about ordinary files.
///
/// Counted before the encoding rather than after, so nothing that will
/// be thrown away is ever encoded. What lands in the column at the
/// ceiling is 1,398,104 bytes of base64 plus the prefix — the number to
/// weigh if this moves, since the column travels with the row. The
/// fixture's own 40,339 bytes store as 53,788.
const MAX_META_RAW_BYTES: usize = 1024 * 1024;

/// PNG's reading of the content and meta axes.
///
/// Stateless — the registry holds it as a constant.
#[derive(Debug, Clone, Copy, Default)]
pub struct PngProbe;

/// What this probe answers for: `image/png`, on both axes.
///
/// One claim, and the only place this probe's formats are written. The
/// gates a caller asks are read off it
/// ([`ProbeGates`](asterism_core::domain::probe::ProbeGates)), and so is
/// the registry's completeness check, so there is no second list to
/// forget to edit. Both axes together because this probe reads both;
/// [`FormatClaim`] carries them separately so that the day it reads one
/// and not the other is a `false` here rather than a redesign.
///
/// `None` — a locator whose extension named nothing — matches no claim,
/// and the gates answer `false`. The bytes might still be a PNG under an
/// unknown extension, and that costs a digest this corpus will not miss:
/// the row falls back to the file axis, which groups renamed copies
/// perfectly well. The alternative is to read every unrecognised file
/// whole on the chance that it is a picture, which is a real cost paid
/// on every unknown row for a case a `guess_mime` arm fixes properly.
const CLAIMS: &[FormatClaim] = &[FormatClaim {
    mime: MimeType::Image(ImageFormat::Png),
    content: true,
    meta: true,
}];

impl PngProbe {
    /// `Some(refusal)` when these bytes are not a PNG's — the half of
    /// the refusal that is this probe's to write.
    ///
    /// # The claim selects, then the signature is checked against it
    ///
    /// The two questions are asked in that order and they are not the
    /// same question. Only the second one is here.
    ///
    /// **The claim selects**, in the port. Anything [`CLAIMS`] does not
    /// cover on the axis being asked is refused by
    /// [`ProbeGates::content`](asterism_core::domain::probe::ProbeGates::content)
    /// before either reading below is entered — including a row that
    /// claims nothing at all, whatever its first eight bytes say — and
    /// the value it stores is the one
    /// [`content_region::unsupported_format`](asterism_core::domain::content_region::unsupported_format)
    /// builds, which is also what the caller stores for a file it
    /// decides not to open. This probe used to compute that branch
    /// itself from a `walks` flag the readings passed in; what replaced
    /// the flag is a
    /// [`GateOpen`](asterism_core::domain::probe::GateOpen) neither
    /// reading can construct, so the agreement no longer depends on this
    /// file remembering to ask.
    ///
    /// **Then the signature is checked**, on the claim that got the
    /// bytes here: a `.png` that is a renamed TIFF is refused, because a
    /// mime is a guess from a filename and pointing a chunk walk at
    /// whatever the file really is on that guess is the shape of problem
    /// this kind of code is known for. This half decides whether to
    /// *trust* a claim; it never accepts an artefact that made none.
    ///
    /// Both refusals cost nothing real. The file axis groups renamed
    /// copies perfectly well, since renaming does not change a byte, so
    /// what is lost is an improvement rather than a row — the direction
    /// the denylist chooses too.
    fn refusal(bytes: &[u8]) -> Option<String> {
        if !png::is_png(bytes) {
            return Some(UNKNOWN_FORMAT.to_string());
        }
        None
    }
}

impl ArtefactProbe for PngProbe {
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

        // The only failure `chunks` reports up front is a signature that
        // is not a PNG's, which `refusal` has already asked about — so
        // this arm is unreachable in practice. It is written out rather
        // than unwrapped because the answer to "these bytes are not a
        // PNG" is a value already in hand, and panicking on untrusted
        // input to save a line is how a parser earns its reputation.
        let Ok(walk) = png::chunks(bytes) else {
            return ContentRegion::Unsupported(UNKNOWN_FORMAT.to_string());
        };

        let mut hasher = ContentHasher::new();
        // One borrowed payload per `IDAT`, held until the end because
        // the stream is fed to the hash as a single concatenation. The
        // length of this list is what `png::MAX_CHUNKS` bounds — the
        // counter is over there with the walk, the allocation is here,
        // and `tests::the_pixel_list_this_probe_accumulates_cannot_outgrow_its_ceiling`
        // holds the two together.
        let mut pixels: Vec<&[u8]> = Vec::new();

        for item in walk {
            let Ok(chunk) = item else {
                return ContentRegion::EmptySpan;
            };
            if chunk.kind == IDAT {
                pixels.push(chunk.payload);
            } else if chunk.kind != IEND && !METADATA_CHUNKS.contains(&chunk.kind) {
                hasher.update(&chunk.kind);
                hasher.update(chunk.payload);
            }
        }

        if pixels.is_empty() {
            return ContentRegion::EmptySpan;
        }
        hasher.update(&IDAT);
        for payload in pixels {
            hasher.update(payload);
        }
        ContentRegion::Digest(under_region_tag(hasher))
    }

    fn meta_of(
        &self,
        bytes: &[u8],
        _declared_mime: Option<&MimeType>,
        _gate: GateOpen,
    ) -> MaterialMeta {
        if let Some(format) = Self::refusal(bytes) {
            return MaterialMeta::Unsupported(format);
        }
        match png::text_fields(bytes) {
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

    /// The frames of every chunk in [`METADATA_CHUNKS`], concatenated in
    /// file order — the container's own bytes, and nothing decided about
    /// them.
    ///
    /// Frames rather than payloads
    /// ([`Chunk::frame`](asterism_media_probe::png::Chunk::frame)): what
    /// is kept has to be walkable again, and a sequence of payloads with
    /// the lengths and types taken out is bytes nobody can take apart.
    /// What comes back is the file's own framing, so putting a PNG
    /// signature in front of it and an `IEND` behind it produces a
    /// sequence this same walk reads — which is how the round trip is
    /// asserted, through the reader that produced `meta_kv` rather than
    /// through a second one written to agree with it. It is **not a
    /// picture**: the five chunks kept here do not include `IHDR` or
    /// `IDAT`, so an image decoder has nothing to open.
    ///
    /// Order is the file's, like the content walk's: a ComfyUI export
    /// carries one text chunk before the pixels and one after, and a
    /// reading that sorted or grouped them would be a decision about
    /// what they mean, which is the one thing this column must not
    /// contain.
    ///
    /// # What is not kept, and why each answers NULL
    ///
    /// - **A file with none of the five.** There is nothing to keep, and
    ///   a marker would say something about this build where the column
    ///   holds a container's bytes. `meta_hash` already carries
    ///   `unsupported:empty-span` beside it.
    /// - **A walk that never reached `IEND`.** What was collected is
    ///   part of a file rather than a file, on the terms
    ///   `asterism_media_probe::png` sets: a caller must not read a
    ///   partial walk as a whole one. Keeping the fragment would offer a
    ///   later reader a container's metadata that is missing whatever
    ///   came after the defect, with nothing saying so.
    /// - **Bytes that are not a PNG's.** Refused before the walk, like
    ///   both readings above.
    ///
    /// The list of borrowed frames is bounded twice over: by
    /// [`MAX_META_RAW_BYTES`], since a frame costs at least 12 bytes, and
    /// by `png::MAX_CHUNKS` above that — the first is the tighter of the
    /// two by an order of magnitude, so the allocation this method makes
    /// is the ceiling's to answer for.
    fn meta_raw_of(
        &self,
        bytes: &[u8],
        _declared_mime: Option<&MimeType>,
        _gate: GateOpen,
    ) -> MetaRaw {
        if Self::refusal(bytes).is_some() {
            return MetaRaw::Absent;
        }
        let Ok(walk) = png::chunks(bytes) else {
            return MetaRaw::Absent;
        };

        let mut kept: Vec<&[u8]> = Vec::new();
        let mut total = 0usize;
        for item in walk {
            let Ok(chunk) = item else {
                return MetaRaw::Absent;
            };
            if !METADATA_CHUNKS.contains(&chunk.kind) {
                continue;
            }
            // Checked as it goes rather than after the walk: the point
            // of the ceiling is that a pathological file never becomes
            // an allocation, and summing first would have made it one.
            total = total.saturating_add(chunk.frame.len());
            if total > MAX_META_RAW_BYTES {
                return MetaRaw::TooLarge;
            }
            kept.push(chunk.frame);
        }

        if kept.is_empty() {
            return MetaRaw::Absent;
        }
        MetaRaw::Captured(kept.concat())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::content_hash::{self, CONTENT_DIGEST_PREFIX};
    use asterism_core::domain::content_region::{EMPTY_SPAN, UNSUPPORTED_PREFIX};
    use asterism_core::domain::material_meta_raw::RAW_PREFIX;
    // The gates and the two public readings: what a caller reaches, and
    // what every assertion below goes through, so that a refusal this
    // probe no longer writes is still asserted where it is now decided.
    use asterism_core::domain::probe::ProbeGates;

    /// The character-card PNG this repo already ships (IHDR / tEXt×2 /
    /// IDAT / IEND, written by `scripts/gen-test-fixtures.py`). Every
    /// variant below is built from its chunks rather than assembled
    /// inline here, so the assertions run over the same chunk layout
    /// the region definition was measured on.
    const CARD_PNG: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../asterism-importer-sdk/tests/fixtures/character-card-lyra.png"
    ));

    type Chunk = (Vec<u8>, Vec<u8>);

    /// The two structural types the fixtures are assembled from,
    /// spelled out here instead of borrowed from the probe.
    ///
    /// A test that reuses the implementation's constants agrees with
    /// them by construction, which is the one thing a fixture builder
    /// must not do: rename `IDAT` to anything at all and every variant
    /// below would keep building whatever the implementation now calls
    /// pixel data, and keep passing.
    const IDAT_TAG: &[u8; 4] = b"IDAT";
    const IEND_TAG: &[u8; 4] = b"IEND";

    fn is(kind: &[u8], tag: &[u8; 4]) -> bool {
        kind == tag.as_slice()
    }

    /// Splits a PNG into `(type, payload)` pairs. CRCs are dropped —
    /// [`build`] recomputes them, so a rebuilt variant is a real PNG
    /// rather than a shape only this probe would accept.
    ///
    /// Structural defects are a panic rather than a value: every caller
    /// hands this a fixture it built or a file this repo ships, so a
    /// failure here means the fixture is wrong, and a test that quietly
    /// walked half of one would assert about bytes nobody meant to
    /// write. The malformed inputs are asserted on through the probe,
    /// which is the surface that has to survive them.
    fn parse(buf: &[u8]) -> Vec<Chunk> {
        pngmeta::chunk_spans(buf)
            .expect("fixture is not a PNG")
            .map(|chunk| {
                let (span, payload) = chunk.expect("fixture walks to a complete chunk sequence");
                (span.kind.as_bytes().to_vec(), payload.to_vec())
            })
            .collect()
    }

    fn build(chunks: &[Chunk]) -> Vec<u8> {
        let mut out = pngmeta::SIGNATURE.to_vec();
        for (kind, payload) in chunks {
            let length = u32::try_from(payload.len()).expect("fixture chunk fits in a PNG length");
            out.extend_from_slice(&length.to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(payload);
            let mut crc_input = kind.clone();
            crc_input.extend_from_slice(payload);
            out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        }
        out
    }

    /// PNG's CRC-32 (the standard reflected polynomial), so the
    /// fixtures stay valid files.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn text(key: &str, value: &str) -> Chunk {
        let mut payload = key.as_bytes().to_vec();
        payload.push(0);
        payload.extend_from_slice(value.as_bytes());
        (b"tEXt".to_vec(), payload)
    }

    /// Parses a literal for the cases below. The probe takes the parsed
    /// form, so a test cannot hand it a spelling the boundary would
    /// already have normalised.
    fn mime(raw: &str) -> MimeType {
        MimeType::parse(raw)
    }

    fn png_mime() -> MimeType {
        mime("image/png")
    }

    /// **What this probe answers for, written out rather than read from
    /// [`CLAIMS`].**
    ///
    /// A test that asked the constant would agree with it by
    /// construction and would keep passing whatever was added to it —
    /// and what gets added is the failure worth catching. The registry
    /// cannot catch it: since a probe's declaration *is* the list of
    /// formats the build covers, a format added here is a format the
    /// registry covers, and completeness has nothing to compare. What it
    /// does not know is this file's other half. [`refusal`] checks the
    /// bytes' signature, so a claim on any format that is not a PNG is a
    /// claim this probe refuses for every artefact carrying it: the gate
    /// opens, the file is read whole up to the walk ceiling, the
    /// signature declines it, and the row stores `unsupported:unknown`
    /// where it used to store the mime it was declared as. A whole
    /// format's stored column, rewritten by one line above.
    ///
    /// The axes are asserted with the mime rather than after it, because
    /// dropping one is the quiet half of the same edit: the column stops
    /// being computed and every row of the format keeps whatever it
    /// already had.
    #[test]
    fn this_probe_declares_image_png_on_both_axes_and_nothing_else() {
        let declared: Vec<(&str, bool, bool)> = PngProbe
            .declares()
            .iter()
            .map(|claim| (claim.mime.as_str(), claim.content, claim.meta))
            .collect();
        assert_eq!(
            declared,
            vec![("image/png", true, true)],
            "this probe's chunk walk and its signature check are PNG's; anything else \
             claimed here is read whole and then refused"
        );
    }

    fn region(bytes: &[u8], declared: Option<&MimeType>) -> ContentRegion {
        PngProbe.content(bytes, declared)
    }

    fn meta_of(bytes: &[u8], declared: Option<&MimeType>) -> MaterialMeta {
        PngProbe.meta(bytes, declared)
    }

    fn digest_of(bytes: &[u8]) -> String {
        match region(bytes, Some(&png_mime())) {
            ContentRegion::Digest(value) => value,
            other => panic!("expected a digest, got {other:?}"),
        }
    }

    fn digest_and_body(bytes: &[u8]) -> (String, String) {
        match meta_of(bytes, Some(&png_mime())) {
            MaterialMeta::Digest { digest, canonical } => (digest, canonical),
            other => panic!("expected a digest, got {other:?}"),
        }
    }

    /// `CARD_PNG`'s chunks with its pixel stream re-cut into
    /// `size`-byte `IDAT` chunks and everything else left in place.
    ///
    /// Returns chunks rather than bytes because one caller asserts on
    /// the resulting chunk *count* — the whole point of the variant is
    /// how many pieces the stream arrives in — and the other only needs
    /// a file. Building both from one function is what makes the count
    /// assertion and the frozen digest describe the same bytes.
    fn resplit_pixels(size: usize) -> Vec<Chunk> {
        let base = parse(CARD_PNG);
        let pixels: Vec<u8> = base
            .iter()
            .filter(|(kind, _)| is(kind, IDAT_TAG))
            .flat_map(|(_, payload)| payload.clone())
            .collect();

        let mut variant: Vec<Chunk> = Vec::new();
        for (kind, payload) in &base {
            if is(kind, IDAT_TAG) {
                continue;
            }
            if is(kind, IEND_TAG) {
                for piece in pixels.chunks(size) {
                    variant.push((IDAT_TAG.to_vec(), piece.to_vec()));
                }
            }
            variant.push((kind.clone(), payload.clone()));
        }
        variant
    }

    /// `CARD_PNG` with a `gAMA` chunk inserted after its header.
    ///
    /// The one fixture where the excluded five and PNG's own
    /// critical/ancillary split give different answers: `gAMA` is
    /// ancillary, and it is not one of the five. Everything else this
    /// file carries is either critical or a `tEXt`, where the two rules
    /// happen to agree. Both the frozen digest below and the ancillary
    /// comparison further down are taken over these bytes, so the two
    /// measure the same disagreement.
    fn with_gamma() -> Vec<u8> {
        let mut chunks = parse(CARD_PNG);
        chunks.insert(1, (b"gAMA".to_vec(), 45_455u32.to_be_bytes().to_vec()));
        build(&chunks)
    }

    /// The digest of `CARD_PNG` itself, and of every variant that
    /// differs from it only in metadata or in where the encoder split
    /// the pixel stream.
    ///
    /// Every other assertion in this module is *relative* — these two
    /// agree, that one moves — and a probe that fed the hasher nothing
    /// at all would satisfy all of them at once. These three literals
    /// are the absolute anchor.
    ///
    /// They were measured against the hand-written chunk walker that
    /// stood in `asterism-core` before the boundary parsing moved out to
    /// `pngmeta`, and they did not move when the walk moved out of the
    /// domain layer to here. What they pin is the region *definition*,
    /// not one implementation of it or one crate's ownership of it: the
    /// code underneath may be replaced and these values must not move.
    /// That is the only way a swap of the parsing layer can be told
    /// apart from a silent change to what is hashed, since the two look
    /// identical in every relative assertion.
    ///
    /// A change to one of these is a `cr2-` decision — a new prefix and
    /// a re-walk of every stored row, see
    /// [`NOT_WALKED`](asterism_core::domain::content_region::NOT_WALKED)
    /// — never a refactor. A diff that edits a literal here to make a
    /// test pass has inverted the reason they exist.
    const CARD_PNG_REGION: &str =
        "cr1-sha256:10225b4d3a3709c47a985ecbf8ac9db0c4e3654cfbc0608032c8252a0205b7c9";

    /// The same, for the synthetic ComfyUI export — a different picture,
    /// so a distinct value. Without it the frozen set would be three
    /// spellings of one digest, and an implementation that returned a
    /// constant would pass.
    const COMFY_EXPORT_REGION: &str =
        "cr1-sha256:b60d7dc769d32fee9a8b417381612225545f815d52447588b46a4ae6af799988";

    /// The same, for `CARD_PNG` carrying a `gAMA` chunk. It differs
    /// from [`CARD_PNG_REGION`] because a colour-management chunk is
    /// not metadata under this definition — that is the measured hole
    /// the denylist exists to close, and freezing it means a rule that
    /// stopped hashing `gAMA` is caught here rather than in a duplicate
    /// group nobody can undo.
    const CARD_PNG_WITH_GAMMA_REGION: &str =
        "cr1-sha256:31dc58fcadfe1318e3ba9723b45f74faf7039b2c558ee930ef891f221350ffc3";

    #[test]
    fn the_region_definition_produces_these_exact_digests() {
        let base = parse(CARD_PNG);
        let stripped: Vec<Chunk> = base
            .iter()
            .filter(|(kind, _)| !is(kind, b"tEXt"))
            .cloned()
            .collect();

        for (label, bytes, expected) in [
            (
                "the fixture as it ships",
                CARD_PNG.to_vec(),
                CARD_PNG_REGION,
            ),
            ("its tEXt chunks removed", build(&stripped), CARD_PNG_REGION),
            (
                "its pixel stream re-cut into 8 KiB chunks",
                build(&resplit_pixels(8 * 1024)),
                CARD_PNG_REGION,
            ),
            (
                "a synthetic ComfyUI export",
                comfy_export(Some("a prompt"), Some(WORKFLOW)),
                COMFY_EXPORT_REGION,
            ),
            (
                "the fixture carrying a gAMA chunk",
                with_gamma(),
                CARD_PNG_WITH_GAMMA_REGION,
            ),
        ] {
            assert_eq!(digest_of(&bytes), expected, "{label}");
        }
    }

    /// The workflow blob a ComfyUI export carries — the metadata this
    /// whole axis exists to see past.
    const WORKFLOW: &str = r#"{"extra":{"ds":{"scale":1.21,"offset":[13,-402]}}}"#;

    #[test]
    fn adding_removing_or_editing_metadata_does_not_move_the_digest() {
        let base = parse(CARD_PNG);
        let original = digest_of(CARD_PNG);
        assert!(
            base.iter().any(|(kind, _)| is(kind, b"tEXt")),
            "the fixture has to carry metadata for its removal to prove anything"
        );

        let stripped: Vec<Chunk> = base
            .iter()
            .filter(|(kind, _)| !is(kind, b"tEXt"))
            .cloned()
            .collect();

        let mut appended = base.clone();
        appended.insert(appended.len() - 1, text("workflow", WORKFLOW));

        let mut prepended = base.clone();
        prepended.insert(1, text("workflow", WORKFLOW));

        let mut edited = base.clone();
        for chunk in &mut edited {
            if is(&chunk.0, b"tEXt") {
                let key_end = chunk.1.iter().position(|b| *b == 0).unwrap_or(0);
                let mut payload = chunk.1[..=key_end].to_vec();
                payload.extend_from_slice(b"MUTATED");
                chunk.1 = payload;
                break;
            }
        }

        for (label, variant) in [
            ("every tEXt removed", stripped),
            ("a tEXt added before IEND", appended),
            ("a tEXt added after IHDR", prepended),
            ("an existing tEXt's value rewritten", edited),
        ] {
            let bytes = build(&variant);
            assert_ne!(
                bytes.as_slice(),
                CARD_PNG,
                "{label}: the file has to differ, or the digest agreeing means nothing"
            );
            assert_eq!(digest_of(&bytes), original, "{label}");
        }
    }

    #[test]
    fn splitting_the_pixel_stream_does_not_move_the_digest() {
        let base = parse(CARD_PNG);
        let original = digest_of(CARD_PNG);
        assert_eq!(
            base.iter().filter(|(kind, _)| is(kind, IDAT_TAG)).count(),
            1,
            "the fixture's stream is one chunk; the splits below are the variation"
        );

        for (size, expected_chunks) in [(8 * 1024usize, 58usize), (64 * 1024, 8)] {
            let variant = resplit_pixels(size);
            assert_eq!(
                variant
                    .iter()
                    .filter(|(kind, _)| is(kind, IDAT_TAG))
                    .count(),
                expected_chunks,
                "{size}-byte split"
            );

            let bytes = build(&variant);
            assert_ne!(bytes.as_slice(), CARD_PNG, "{size}-byte split");
            assert_eq!(digest_of(&bytes), original, "{size}-byte split");
        }
    }

    #[test]
    fn changing_one_bit_of_the_pixel_stream_moves_the_digest() {
        // The control. Without it the two tests above are satisfied by
        // a probe that hashes nothing at all.
        let mut flipped = parse(CARD_PNG);
        for chunk in &mut flipped {
            if is(&chunk.0, IDAT_TAG) {
                chunk.1[100] ^= 0x01;
                break;
            }
        }
        let bytes = build(&flipped);
        assert_eq!(bytes.len(), CARD_PNG.len(), "one bit, not one byte more");
        assert_ne!(digest_of(&bytes), digest_of(CARD_PNG));
    }

    /// The same walk with exactly one line changed: metadata is
    /// whatever PNG calls ancillary, instead of the excluded five.
    ///
    /// Not a candidate implementation — a measuring instrument. The
    /// module doc and [`METADATA_CHUNKS`] both claim the two rules are
    /// different; this makes the difference a value two assertions can
    /// compare, so the claim cannot quietly stop being true. Everything
    /// else is copied deliberately (the `IEND` exclusion, the deferred
    /// pixel stream, the prefix) so that the only thing the comparison
    /// can be measuring is the selection rule.
    fn ancillary_digest(bytes: &[u8]) -> String {
        let mut hasher = ContentHasher::new();
        let mut pixels: Vec<&[u8]> = Vec::new();
        for chunk in pngmeta::chunk_spans(bytes).expect("fixture is a PNG") {
            let (span, payload) = chunk.expect("fixture walks to a complete chunk sequence");
            if span.kind == pngmeta::ChunkType::IDAT {
                pixels.push(payload);
            } else if span.kind != pngmeta::ChunkType::IEND && !span.kind.is_ancillary() {
                hasher.update(span.kind.as_bytes());
                hasher.update(payload);
            }
        }
        hasher.update(IDAT_TAG);
        for payload in pixels {
            hasher.update(payload);
        }
        under_region_tag(hasher)
    }

    #[test]
    fn pngs_own_ancillary_split_is_not_this_probes_excluded_five() {
        // Where the two rules agree. Everything `CARD_PNG` carries is
        // either critical or a tEXt, and tEXt is on both lists — so if
        // this line ever failed, what follows would be measuring a
        // difference between the two harnesses rather than between the
        // two rules.
        assert_eq!(
            ancillary_digest(CARD_PNG),
            digest_of(CARD_PNG),
            "with nothing but tEXt to disagree about, the two rules have to agree"
        );

        // Where they part. gAMA is ancillary and is not one of the
        // five: this probe hashes it, the ancillary rule drops it.
        let gamma = with_gamma();
        assert_ne!(
            ancillary_digest(&gamma),
            digest_of(&gamma),
            "gAMA is exactly the case the two rules answer differently"
        );

        // And the shape of the difference is the failure the denylist
        // exists to prevent, not a harmless disagreement: under the
        // ancillary rule the file with a gAMA chunk lands on the digest
        // of the file without one, so two pictures that display
        // differently become one duplicate group. Under this probe's
        // rule they stay two.
        assert_eq!(
            ancillary_digest(&gamma),
            ancillary_digest(CARD_PNG),
            "the ancillary rule collapses the two onto one digest"
        );
        assert_ne!(digest_of(&gamma), digest_of(CARD_PNG));
    }

    #[test]
    fn two_display_gammas_are_not_the_same_picture() {
        // Measured hole in the allowlist definition: gAMA 1.0 and
        // gAMA 2.2 decode to visibly different images from identical
        // pixel data.
        let base = parse(CARD_PNG);
        let mut digests = Vec::new();
        for gamma in [100_000u32, 45_455] {
            let mut variant = base.clone();
            variant.insert(1, (b"gAMA".to_vec(), gamma.to_be_bytes().to_vec()));
            digests.push(digest_of(&build(&variant)));
        }
        assert_ne!(digests[0], digests[1]);
        assert_ne!(digests[0], digest_of(CARD_PNG));
    }

    #[test]
    fn two_apng_second_frames_are_not_the_same_animation() {
        // The other measured hole: everything after frame 1 lives in
        // fdAT, which an IDAT-and-friends allowlist never sees.
        let base = parse(CARD_PNG);
        let mut digests = Vec::new();
        for fill in [0x00u8, 0xff] {
            let mut variant = base.clone();
            variant.insert(1, (b"acTL".to_vec(), [0, 0, 0, 2, 0, 0, 0, 0].to_vec()));
            let mut frame = vec![0, 0, 0, 1];
            frame.resize(frame.len() + 64, fill);
            variant.insert(variant.len() - 1, (b"fdAT".to_vec(), frame));
            digests.push(digest_of(&build(&variant)));
        }
        assert_ne!(digests[0], digests[1]);
    }

    #[test]
    fn a_chunk_type_is_part_of_what_is_hashed() {
        // Lengths are deliberately not fed to the hash, so without the
        // type the same four bytes under two different chunks would
        // collide.
        let base = parse(CARD_PNG);
        let mut digests = Vec::new();
        for kind in [b"gAMA", b"cHRM"] {
            let mut variant = base.clone();
            variant.insert(1, (kind.to_vec(), vec![0xde, 0xad, 0xbe, 0xef]));
            digests.push(digest_of(&build(&variant)));
        }
        assert_ne!(digests[0], digests[1]);
    }

    #[test]
    fn a_real_file_carrying_its_text_after_the_pixels_walks() {
        let chunks = parse(CARD_PNG);
        let last_pixels = chunks
            .iter()
            .rposition(|(kind, _)| is(kind, IDAT_TAG))
            .expect("fixture has pixel data");
        let first_text = chunks
            .iter()
            .position(|(kind, _)| is(kind, b"tEXt"))
            .expect("fixture has metadata");
        assert!(
            first_text > last_pixels,
            "this fixture's value is that its metadata sits after its pixels; \
             if that stopped being true the ordering claim is untested"
        );

        let value = digest_of(CARD_PNG);
        let hex = value
            .strip_prefix(CONTENT_DIGEST_PREFIX)
            .expect("the value declares its region version");
        assert_eq!(hex.len(), 64);
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        // The two axes must not be read across: a content digest is not
        // a file-axis duplicate key.
        assert!(!content_hash::is_duplicate_key(
            asterism_core::domain::duplicate_conflict::DuplicateAxis::Artefact,
            &value
        ));
    }

    /// The same workflow after the canvas was panned and zoomed — the
    /// only difference between the two files in the duplicate groups
    /// this axis exists to catch (9 such groups in a 4,601-image
    /// ComfyUI corpus, pixel bytes identical, file digests different).
    const WORKFLOW_MOVED_CANVAS: &str = r#"{"extra":{"ds":{"scale":0.87,"offset":[-451,88]}}}"#;

    /// 140 KiB of deterministic bytes standing in for a compressed
    /// stream — enough to land in several 64 KiB chunks.
    fn pixel_stream(seed: u32) -> Vec<u8> {
        let mut state = seed;
        (0..140 * 1024)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect()
    }

    /// A synthetic export in the shape a ComfyUI file actually has: the
    /// prompt before the pixels, the workflow after, and the pixel
    /// stream cut into 64 KiB chunks. Either text may be absent, which
    /// is also how the real corpus looks. The corpus itself is not in
    /// this repo, so the shape is rebuilt here.
    fn comfy_export(prompt: Option<&str>, workflow: Option<&str>) -> Vec<u8> {
        comfy_export_seeded(prompt, workflow, 0x1234_5678)
    }

    /// [`comfy_export`] with the pixel stream chosen by the caller — for
    /// the meta-axis cases, whose point is two files off one workflow
    /// that differ in nothing but their pixels.
    fn comfy_export_seeded(prompt: Option<&str>, workflow: Option<&str>, seed: u32) -> Vec<u8> {
        let mut chunks: Vec<Chunk> = vec![(
            b"IHDR".to_vec(),
            vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0],
        )];
        if let Some(prompt) = prompt {
            chunks.push(text("prompt", prompt));
        }
        for piece in pixel_stream(seed).chunks(64 * 1024) {
            chunks.push((IDAT_TAG.to_vec(), piece.to_vec()));
        }
        if let Some(workflow) = workflow {
            chunks.push(text("workflow", workflow));
        }
        chunks.push((IEND_TAG.to_vec(), Vec::new()));
        build(&chunks)
    }

    #[test]
    fn text_on_both_sides_of_the_pixels_is_metadata_on_both_sides() {
        let both = comfy_export(Some("a prompt"), Some(WORKFLOW));
        let chunks = parse(&both);
        let first_pixels = chunks
            .iter()
            .position(|(kind, _)| is(kind, IDAT_TAG))
            .expect("fixture has pixel data");
        let last_pixels = chunks
            .iter()
            .rposition(|(kind, _)| is(kind, IDAT_TAG))
            .expect("fixture has pixel data");
        let text_positions: Vec<usize> = chunks
            .iter()
            .enumerate()
            .filter(|(_, (kind, _))| is(kind, b"tEXt"))
            .map(|(at, _)| at)
            .collect();
        assert!(
            text_positions.first().is_some_and(|at| *at < first_pixels)
                && text_positions.last().is_some_and(|at| *at > last_pixels),
            "the point of this fixture is text on both sides of the pixels"
        );
        assert!(last_pixels - first_pixels >= 2, "several pixel chunks");

        let baseline = digest_of(&both);
        for (label, variant) in [
            (
                "the prompt, before the pixels, removed",
                comfy_export(None, Some(WORKFLOW)),
            ),
            (
                "the workflow, after the pixels, removed",
                comfy_export(Some("a prompt"), None),
            ),
            ("neither text present", comfy_export(None, None)),
            (
                "only the workflow's canvas position changed",
                comfy_export(Some("a prompt"), Some(WORKFLOW_MOVED_CANVAS)),
            ),
        ] {
            assert_ne!(variant, both, "{label}: the file has to differ");
            assert_eq!(digest_of(&variant), baseline, "{label}");
        }

        // The control, on this fixture too: metadata invariance means
        // nothing if the pixels are invariant as well.
        let mut moved = parse(&both);
        for chunk in &mut moved {
            if is(&chunk.0, IDAT_TAG) {
                chunk.1[7] ^= 0x01;
                break;
            }
        }
        assert_ne!(digest_of(&build(&moved)), baseline);
    }

    #[test]
    fn a_png_with_no_pixel_chunks_gets_a_marker_not_an_empty_digest() {
        let ihdr = (
            b"IHDR".to_vec(),
            vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0],
        );
        let bytes = build(&[ihdr, (IEND_TAG.to_vec(), Vec::new())]);

        let walked = region(&bytes, Some(&png_mime()));
        assert_eq!(walked, ContentRegion::EmptySpan);
        assert_eq!(walked.stored_value(), EMPTY_SPAN);
        assert!(walked.digest().is_none());

        // The failure this prevents: a digest over zero bytes is real,
        // and every truncated PNG would share it.
        let empty_region = format!(
            "{CONTENT_DIGEST_PREFIX}{}",
            content_hash::EMPTY
                .strip_prefix(content_hash::DIGEST_PREFIX)
                .expect("the empty digest carries its algorithm")
        );
        assert_ne!(walked.stored_value(), empty_region);
        assert!(!walked.stored_value().starts_with(CONTENT_DIGEST_PREFIX));
    }

    #[test]
    fn bytes_that_are_not_a_png_are_not_walked() {
        // The mime says one thing, the bytes say another; both
        // directions refuse, and each says as much as it knows.
        assert_eq!(
            region(CARD_PNG, Some(&mime("image/jpeg"))),
            ContentRegion::Unsupported("image/jpeg".to_string())
        );
        assert_eq!(
            meta_of(CARD_PNG, Some(&mime("image/jpeg"))),
            MaterialMeta::Unsupported("image/jpeg".to_string())
        );
        assert_eq!(
            region(
                b"\xff\xd8\xff\xe0 not a png at all",
                Some(&mime("image/png"))
            ),
            ContentRegion::Unsupported(UNKNOWN_FORMAT.to_string())
        );
        assert_eq!(
            meta_of(
                b"\xff\xd8\xff\xe0 not a png at all",
                Some(&mime("image/png"))
            ),
            MaterialMeta::Unsupported(UNKNOWN_FORMAT.to_string())
        );
        assert_eq!(
            region(b"plain text, no claim", None),
            ContentRegion::Unsupported(UNKNOWN_FORMAT.to_string())
        );
        assert_eq!(
            meta_of(b"plain text, no claim", None),
            MaterialMeta::Unsupported(UNKNOWN_FORMAT.to_string())
        );
        // A parameterised or shouted mime is the same claim — settled
        // at the parse boundary now, rather than by this probe's own
        // normalisation.
        assert!(matches!(
            region(CARD_PNG, Some(&mime("IMAGE/PNG; charset=binary"))),
            ContentRegion::Digest(_)
        ));
        assert_eq!(
            region(b"", Some(&mime("video/mp4"))).stored_value(),
            "unsupported:video/mp4"
        );
        assert!(
            region(b"", None)
                .stored_value()
                .starts_with(UNSUPPORTED_PREFIX)
        );
    }

    #[test]
    fn the_answer_given_without_the_bytes_is_the_one_the_walk_would_have_given() {
        use asterism_core::domain::content_region;

        // The caller decides whether to read the file from the mime
        // alone. Whatever it stores for the files it does not read has
        // to be the same value this probe produces when it is handed
        // no bytes, or one artefact would carry two different markers
        // depending on which side of the size gate it fell.
        for raw in [
            Some("video/mp4"),
            Some("IMAGE/JPEG; charset=binary"),
            Some("text/plain"),
            Some("   "),
            None,
        ] {
            let parsed = raw.map(MimeType::parse);
            let declared = parsed.as_ref();
            assert!(!PngProbe.walks_content(declared), "{raw:?} has no walker");
            assert!(!PngProbe.walks_meta(declared), "{raw:?} has no walker");
            assert_eq!(
                content_region::unsupported_format(declared),
                region(&[], declared),
                "{raw:?}"
            );
            assert_eq!(
                material_meta::unsupported_format(declared),
                meta_of(&[], declared),
                "{raw:?}"
            );
        }

        // …and the one format that does route here, however it is
        // spelled. A caller reading this as "false" would stop
        // fingerprinting the corpus this axis exists for.
        for raw in ["image/png", "IMAGE/PNG; charset=binary", " image/png "] {
            assert!(
                PngProbe.walks_content(Some(&mime(raw))),
                "{raw:?} routes to the walker"
            );
            assert!(
                PngProbe.walks_meta(Some(&mime(raw))),
                "{raw:?} routes to the walker"
            );
        }
        assert!(matches!(
            region(CARD_PNG, Some(&png_mime())),
            ContentRegion::Digest(_)
        ));
    }

    #[test]
    fn a_broken_png_falls_to_a_marker_without_trusting_its_lengths() {
        let intact = parse(CARD_PNG);

        // A length that runs past the end of the file.
        let mut overrun = build(&intact);
        let len_at = pngmeta::SIGNATURE.len();
        overrun[len_at..len_at + 4].copy_from_slice(&0x0010_0000u32.to_be_bytes());
        assert_eq!(
            region(&overrun, Some(&png_mime())),
            ContentRegion::EmptySpan
        );
        assert_eq!(
            meta_of(&overrun, Some(&png_mime())),
            MaterialMeta::EmptySpan
        );

        // Cut in half: chunks parse until the bytes run out. This
        // fixture's text sits *after* its pixels, so a meta walk that
        // returned what it had would have returned nothing and called it
        // an answer anyway.
        let truncated = &CARD_PNG[..CARD_PNG.len() / 2];
        assert_eq!(
            region(truncated, Some(&png_mime())),
            ContentRegion::EmptySpan
        );
        assert_eq!(
            meta_of(truncated, Some(&png_mime())),
            MaterialMeta::EmptySpan
        );

        // A 4 GiB chunk declared inside a 30-byte file. Nothing is
        // sized from that number — if it were, this test would be an
        // allocation of four gigabytes rather than an assertion.
        for declared in [0xffff_fff0u32, u32::MAX, 0x8000_0000] {
            let mut bomb = pngmeta::SIGNATURE.to_vec();
            bomb.extend_from_slice(&declared.to_be_bytes());
            bomb.extend_from_slice(b"IDAT");
            bomb.extend_from_slice(&[0u8; 14]);
            assert_eq!(bomb.len(), 30);
            assert_eq!(
                region(&bomb, Some(&png_mime())),
                ContentRegion::EmptySpan,
                "declared length {declared:#x}"
            );
        }

        // Header alone, then nothing.
        assert_eq!(
            region(&pngmeta::SIGNATURE, Some(&png_mime())),
            ContentRegion::EmptySpan
        );
        assert_eq!(
            region(&pngmeta::SIGNATURE[..7], Some(&png_mime())),
            ContentRegion::Unsupported(UNKNOWN_FORMAT.to_string())
        );

        // Chunks that never reach IEND — with the text already read on
        // the meta side, which is the variant that matters there.
        let unterminated: Vec<Chunk> = intact
            .iter()
            .filter(|(kind, _)| !is(kind, IEND_TAG))
            .cloned()
            .collect();
        assert!(
            unterminated.iter().any(|(kind, _)| is(kind, b"tEXt")),
            "the point of this variant is that the text was reached before the end was not"
        );
        assert_eq!(
            region(&build(&unterminated), Some(&png_mime())),
            ContentRegion::EmptySpan
        );
        assert_eq!(
            meta_of(&build(&unterminated), Some(&png_mime())),
            MaterialMeta::EmptySpan
        );

        // More chunks than the walk will follow.
        let mut swarm: Vec<Chunk> = vec![(IDAT_TAG.to_vec(), vec![0u8; 1])];
        swarm.extend((0..=png::MAX_CHUNKS).map(|_| (b"prIV".to_vec(), Vec::new())));
        swarm.push((IEND_TAG.to_vec(), Vec::new()));
        assert_eq!(
            region(&build(&swarm), Some(&png_mime())),
            ContentRegion::EmptySpan
        );

        let mut text_swarm: Vec<Chunk> = vec![text("workflow", "{}")];
        text_swarm.extend((0..=png::MAX_CHUNKS).map(|_| (b"prIV".to_vec(), Vec::new())));
        text_swarm.push((IEND_TAG.to_vec(), Vec::new()));
        assert_eq!(
            meta_of(&build(&text_swarm), Some(&png_mime())),
            MaterialMeta::EmptySpan
        );
    }

    /// **The most borrowed payloads this probe will hold at once**,
    /// written out rather than imported from
    /// [`png::MAX_CHUNKS`](asterism_media_probe::png::MAX_CHUNKS).
    ///
    /// The number is the same number. Spelling it here is the point: a
    /// test that says `(0..=png::MAX_CHUNKS)` agrees with the constant
    /// by construction and would keep passing if it were raised to a
    /// million, which is exactly the edit worth catching — the counter
    /// lives with the parser, and from there the ceiling looks like it
    /// bounds nothing, because nothing over there holds a chunk. What it
    /// bounds is the `Vec<&[u8]>` in this file.
    const MOST_BORROWED_PAYLOADS: usize = 65_536;

    /// The list of pixel payloads is bounded, and bounded by a number
    /// this side agreed to.
    ///
    /// Two halves, and both are needed. The first is the agreement: the
    /// walk must not admit more chunks than this probe is willing to
    /// borrow payloads for. Both sides are constants, so it is a `const`
    /// block — raising the parser's counter stops this crate compiling
    /// rather than turning a test red, which is the earliest either
    /// number can be told it moved. The second half is that the
    /// agreement describes the code rather than a comment: one payload
    /// past the ceiling and the walk stops, so the accumulation ends
    /// with the file refused instead of running on.
    #[test]
    fn the_pixel_list_this_probe_accumulates_cannot_outgrow_its_ceiling() {
        const {
            assert!(
                png::MAX_CHUNKS <= MOST_BORROWED_PAYLOADS,
                "the walk now admits more chunks than this probe agreed to borrow \
                 payloads for: raising the parser's counter raised an allocation \
                 that lives here, in `PngProbe::content`"
            );
        }

        // A file whose pixel stream alone reaches the ceiling. The walk
        // refuses it rather than borrowing its way to the end, which is
        // the behaviour the number above is a statement about.
        let mut swarm: Vec<Chunk> = vec![(
            b"IHDR".to_vec(),
            vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0],
        )];
        swarm.extend((0..MOST_BORROWED_PAYLOADS).map(|_| (IDAT_TAG.to_vec(), vec![0u8; 1])));
        swarm.push((IEND_TAG.to_vec(), Vec::new()));
        assert!(
            swarm.len() > MOST_BORROWED_PAYLOADS,
            "the fixture has to cross the ceiling for its refusal to mean anything"
        );
        assert_eq!(
            region(&build(&swarm), Some(&png_mime())),
            ContentRegion::EmptySpan
        );
    }

    // ---- the meta axis -------------------------------------------------

    /// The form, read off a real container: sorted keys, no whitespace,
    /// and values exactly as the chunk carried them.
    #[test]
    fn the_canonical_form_is_what_the_chunks_carried() {
        // Inserted in the wrong order on purpose, and one value is a
        // JSON document — the case the "strings, unparsed" rule exists
        // for.
        let workflow = r#"{"seed": 7, "cfg": 1.50}"#;
        let bytes = build(&[
            (
                b"IHDR".to_vec(),
                vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0],
            ),
            text("workflow", workflow),
            text("prompt", "a cat"),
            text("Software", "ComfyUI"),
            (IDAT_TAG.to_vec(), vec![1, 2, 3]),
            (IEND_TAG.to_vec(), Vec::new()),
        ]);

        let (digest, canonical) = digest_and_body(&bytes);
        assert_eq!(
            canonical,
            r#"{"Software":"ComfyUI","prompt":"a cat","workflow":"{\"seed\": 7, \"cfg\": 1.50}"}"#,
            "sorted by key, no whitespace, and the workflow's own spacing kept verbatim"
        );
        // The digest and the body it was taken over travel together, so
        // a caller cannot store one that describes the other.
        assert_eq!(digest, material_meta::digest_of(&canonical));
    }

    /// The axis's own reason for existing: two frames off one workflow
    /// whose only difference is the pixels.
    ///
    /// The control is the other direction — the same pixels with the
    /// metadata edited — because a probe that hashed nothing at all
    /// would satisfy the first assertion alone.
    #[test]
    fn the_pixels_are_not_part_of_the_metadata_and_the_metadata_is() {
        let workflow = r#"{"nodes":[{"class":"KSampler"}]}"#;
        let one = comfy_export_seeded(Some("a prompt"), Some(workflow), 0x1234_5678);
        let two = comfy_export_seeded(Some("a prompt"), Some(workflow), 0x8765_4321);
        assert_ne!(one, two, "the files have to differ, or nothing is proved");

        let (first, body) = digest_and_body(&one);
        let (second, same_body) = digest_and_body(&two);
        assert_eq!(first, second, "different pixels, one workflow");
        assert_eq!(body, same_body);

        // …and the content axis disagrees about exactly this pair,
        // which is what makes the two axes two axes.
        assert_ne!(
            region(&one, Some(&png_mime())).stored_value(),
            region(&two, Some(&png_mime())).stored_value(),
            "the content axis has to see the pixels the meta axis ignores"
        );

        // The control: one character of metadata moves the digest.
        let edited = comfy_export_seeded(
            Some("a prompt"),
            Some(r#"{"nodes":[{"class":"KSampler2"}]}"#),
            0x1234_5678,
        );
        assert_ne!(digest_and_body(&edited).0, first);

        // And an absent chunk is a different metadata set from a
        // present one, rather than the same set with a hole in it.
        assert_ne!(
            digest_and_body(&comfy_export_seeded(None, Some(workflow), 1)).0,
            first
        );
    }

    /// A PNG carrying no text at all takes a marker, not the digest of
    /// `{}`.
    #[test]
    fn a_png_with_no_text_gets_a_marker_not_the_digest_of_an_empty_object() {
        let bare = comfy_export_seeded(None, None, 7);
        let walked = meta_of(&bare, Some(&png_mime()));
        assert_eq!(walked, MaterialMeta::EmptySpan);
        assert_eq!(walked.stored_value(), EMPTY_SPAN);
        assert!(walked.digest().is_none());
        assert!(walked.canonical().is_none());

        // The failure this prevents: a digest over `{}` is real, and
        // every metadata-less PNG in the library would share it.
        assert_ne!(walked.stored_value(), material_meta::digest_of("{}"));
        // A second metadata-less file would otherwise have grouped
        // with the first.
        assert_eq!(
            meta_of(&comfy_export_seeded(None, None, 9), Some(&png_mime())),
            walked
        );
    }

    /// The two readings the form fixes in place, asserted rather than
    /// left to the implementation: a repeated keyword collapses, and a
    /// keyword with no text is a key with an empty value rather than a
    /// dropped entry.
    #[test]
    fn a_repeated_keyword_collapses_and_an_empty_value_is_still_an_entry() {
        let bytes = build(&[
            (
                b"IHDR".to_vec(),
                vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0],
            ),
            text("Comment", "first"),
            text("Comment", "second"),
            text("Empty", ""),
            (IDAT_TAG.to_vec(), vec![1]),
            (IEND_TAG.to_vec(), Vec::new()),
        ]);
        let (_, canonical) = digest_and_body(&bytes);
        assert_eq!(
            canonical, r#"{"Comment":"second","Empty":""}"#,
            "the last occurrence wins, and an empty text is a value"
        );

        // A chunk with no separator at all names nothing and is not an
        // entry — the payload is not a `keyword \0 text` pair.
        let malformed = build(&[
            (
                b"IHDR".to_vec(),
                vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0],
            ),
            (b"tEXt".to_vec(), b"no separator here".to_vec()),
            (IDAT_TAG.to_vec(), vec![1]),
            (IEND_TAG.to_vec(), Vec::new()),
        ]);
        assert_eq!(
            meta_of(&malformed, Some(&png_mime())),
            MaterialMeta::EmptySpan
        );
    }

    // ---- the bytes behind the meta axis ---------------------------------

    fn raw_of(bytes: &[u8], declared: Option<&MimeType>) -> MetaRaw {
        PngProbe.meta_raw(bytes, declared)
    }

    fn raw_bytes(bytes: &[u8]) -> Vec<u8> {
        match raw_of(bytes, Some(&png_mime())) {
            MetaRaw::Captured(kept) => kept,
            other => panic!("expected kept bytes, got {other:?}"),
        }
    }

    /// **The metadata chunks of the character card this repo ships**,
    /// measured — the number [`MAX_META_RAW_BYTES`] is chosen against.
    ///
    /// The largest metadata payload anything here has in reach: a
    /// character card carries a whole persona document, base64 inside a
    /// `tEXt`, which is why this file rather than a ComfyUI export is
    /// the worst case. Frozen as a literal so that the ceiling's
    /// justification is a measurement rather than a memory of one — if a
    /// `pngmeta` bump or a change to the kept set moved it, the number
    /// the doc argues from moves with it and this says so.
    ///
    /// The card is synthetic (`scripts/gen-test-fixtures.py`) and its
    /// lore book is sized *to this number*: the real card measured
    /// 40,339 before it was replaced, and the generator's entry count
    /// is tuned to land just under it. That inverts the usual reading —
    /// the fixture is built to the measurement rather than the
    /// measurement taken off whatever the fixture happened to be — so
    /// the thing to preserve when regenerating is the order of
    /// magnitude, not this exact digit string.
    pub(super) const CARD_PNG_META_RAW_BYTES: usize = 39_587;

    #[test]
    fn the_ceiling_is_measured_against_the_largest_thing_this_corpus_carries() {
        assert_eq!(
            raw_bytes(CARD_PNG).len(),
            CARD_PNG_META_RAW_BYTES,
            "the measured worst case moved; the ceiling's doc argues from this number"
        );
        // A `const` block, like the sibling ceiling's agreement below:
        // both sides are constants, so lowering the ceiling towards the
        // corpus stops this crate compiling rather than turning a test
        // red — the earliest either number can be told it moved.
        const {
            assert!(
                CARD_PNG_META_RAW_BYTES * 25 <= MAX_META_RAW_BYTES,
                "the ceiling is a bound on the pathological case, not a policy about \
                 ordinary files — 25x the worst case measured here"
            );
        }

        // And what that measurement costs the column, which is the
        // other number the ceiling's doc quotes: base64 is 4 bytes per
        // 3, so the row carries half again as much as the file does.
        assert_eq!(
            MetaRaw::Captured(raw_bytes(CARD_PNG))
                .stored_value()
                .map(|value| value.len() - RAW_PREFIX.len()),
            Some(52_784)
        );
    }

    /// **Every chunk of the real file arrives with its frame, so no
    /// digest here can be lost to a frame that could not be produced.**
    ///
    /// `Chunk::frame` is a slice of the input taken from a span the
    /// walk yielded, and the walk hands back a stop when it cannot be
    /// taken — which this probe reads as [`ContentRegion::EmptySpan`].
    /// That is the one path by which the arrival of the raw column
    /// could move a value that already exists: a file holding
    /// [`CARD_PNG_REGION`] would take a marker instead, and a marker
    /// says nobody walked it.
    ///
    /// The property that rules it out is `pngmeta`'s and undocumented
    /// (the walk verifies a chunk's length against what remains before
    /// yielding it), so it is pinned twice: once over synthetic shapes
    /// where the walk is defined
    /// (`asterism_media_probe::png::tests`), and once here, over the
    /// real file this repo ships and the digest it is frozen at.
    #[test]
    fn every_chunk_of_the_real_fixture_carries_a_frame_and_keeps_its_digest() {
        let mut frames = Vec::new();
        for item in png::chunks(CARD_PNG).expect("the fixture is a PNG") {
            let chunk = item.expect("no chunk of a whole file ends the walk");
            assert_eq!(
                chunk.frame.len(),
                12 + chunk.payload.len(),
                "{:?} lost its framing",
                std::str::from_utf8(&chunk.kind)
            );
            frames.extend_from_slice(chunk.frame);
        }
        assert_eq!(
            frames,
            CARD_PNG[pngmeta::SIGNATURE.len()..],
            "the frames in order are the file, so none was clamped or skipped"
        );

        // The consequence, stated rather than left implied: this file
        // holds a digest and not the marker a stopped walk would store.
        assert_eq!(digest_of(CARD_PNG), CARD_PNG_REGION);
        assert_ne!(
            region(CARD_PNG, Some(&png_mime())).stored_value(),
            EMPTY_SPAN
        );
    }

    /// **The round trip, at this probe's level: the kept bytes walk
    /// back to the same fields `meta_of` rendered.**
    ///
    /// The frames are the file's own framing, so a signature in front
    /// and an `IEND` behind produce a sequence **this same walk** reads
    /// — `png::text_fields`, the reader `meta_of` uses. A second reader
    /// written for this test would agree with the implementation by
    /// construction and prove nothing about whether the bytes are
    /// enough.
    ///
    /// Asserted on the real fixture and on an export carrying text on
    /// both sides of its pixels, because a reading that kept only what
    /// it met before the first `IDAT` would pass on one of them.
    #[test]
    fn the_kept_bytes_walk_back_to_the_fields_the_digest_was_taken_over() {
        for (label, bytes) in [
            ("the fixture as it ships", CARD_PNG.to_vec()),
            (
                "text on both sides of the pixels",
                comfy_export(Some("a prompt"), Some(WORKFLOW)),
            ),
        ] {
            let sequence = as_walkable_chunk_sequence(&raw_bytes(&bytes));
            let walked = png::text_fields(&sequence).expect("the kept frames walk");
            assert_eq!(
                material_meta::render(&walked),
                digest_and_body(&bytes).1,
                "{label}: the kept bytes render to the same canonical object"
            );
        }
    }

    /// The kept frames made walkable: signature, the frames, an `IEND`.
    ///
    /// **Not a PNG**, and the name says so because the difference is
    /// what a later consumer of the raw has to know. Only the five
    /// metadata chunks are kept, so there is no `IHDR` and no `IDAT`
    /// here: a chunk walk reads it, and an image decoder — or
    /// `pngmeta::read_text_chunks`, which opens a file — refuses it.
    /// The raw is a chunk sequence and wants a chunk walk.
    ///
    /// The terminator is added rather than kept because `IEND` is
    /// structural — it is in neither axis and it is not metadata, so it
    /// is not one of the five. Its absence from the raw is the reason
    /// this helper exists: `text_fields` refuses a sequence it did not
    /// see end, on purpose.
    fn as_walkable_chunk_sequence(kept: &[u8]) -> Vec<u8> {
        let mut out = pngmeta::SIGNATURE.to_vec();
        out.extend_from_slice(kept);
        out.extend_from_slice(
            &build(&[(IEND_TAG.to_vec(), Vec::new())])[pngmeta::SIGNATURE.len()..],
        );
        out
    }

    /// **The two chunks neither digest is about survive in the bytes.**
    ///
    /// This is what the raw is for. `zTXt` and `iTXt` move neither
    /// digest — the assertion below this one says so, and it is the same
    /// assertion the axes' own test makes — and after this slice they
    /// are recoverable from the row anyway, which is the difference
    /// between a stated gap and a loss.
    #[test]
    fn the_compressed_text_chunks_reach_the_raw_while_staying_out_of_meta_kv() {
        let ztxt: &[u8] = b"workflow\0\0\x78\x9c\x03\x00\x00\x00\x00\x01";
        let itxt: &[u8] = b"Description\0\0\0\0\0a caption";
        let mut widened = parse(CARD_PNG);
        widened.insert(1, (b"zTXt".to_vec(), ztxt.to_vec()));
        widened.insert(2, (b"iTXt".to_vec(), itxt.to_vec()));
        widened.insert(3, (b"tIME".to_vec(), vec![0x07, 0xea, 8, 9, 12, 0, 0]));
        widened.insert(4, (b"eXIf".to_vec(), b"II*\0\x08\0\0\0\0\0".to_vec()));
        let bytes = build(&widened);

        let kept = raw_bytes(&bytes);
        for (label, payload) in [("zTXt", ztxt), ("iTXt", itxt)] {
            assert!(
                kept.windows(payload.len()).any(|w| w == payload),
                "{label}: the payload is in the kept bytes verbatim"
            );
        }
        // …and neither of them is in the rendering, which is the half
        // that must not have changed.
        let (_, canonical) = digest_and_body(&bytes);
        assert!(
            !canonical.contains("Description") && !canonical.contains("workflow"),
            "the digest's input is still tEXt only: {canonical}"
        );
        assert_eq!(
            digest_and_body(&bytes).0,
            digest_and_body(CARD_PNG).0,
            "and the digest itself did not move"
        );

        // The five chunks are kept and nothing else is: the file's IHDR
        // and its pixels are not metadata, and a reading that took the
        // whole file would still satisfy the assertions above.
        assert!(
            kept.len() < bytes.len() / 2,
            "kept {} of {}",
            kept.len(),
            bytes.len()
        );
        assert!(
            !kept.windows(4).any(|w| w == b"IHDR"),
            "the header is not metadata"
        );
    }

    /// A non-UTF-8 `tEXt` value comes back as the byte the rendering
    /// replaced — the loss this column exists to undo.
    #[test]
    fn a_latin1_text_value_the_rendering_lost_is_recoverable_from_the_raw() {
        let mut payload = b"caption".to_vec();
        payload.push(0);
        payload.extend_from_slice(&[0xe9, 0xff]);
        let bytes = build(&[
            (
                b"IHDR".to_vec(),
                vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0],
            ),
            (b"tEXt".to_vec(), payload),
            (IDAT_TAG.to_vec(), vec![1]),
            (IEND_TAG.to_vec(), Vec::new()),
        ]);

        // What the digest was taken over: the bytes are gone, twice
        // over, and no reading of this column gets them back.
        let (_, canonical) = digest_and_body(&bytes);
        assert_eq!(canonical, "{\"caption\":\"\u{fffd}\u{fffd}\"}");

        // What the row keeps.
        let kept = raw_bytes(&bytes);
        assert!(
            kept.windows(2).any(|w| w == [0xe9, 0xff]),
            "the Latin-1 bytes are in the raw: {kept:?}"
        );
    }

    /// Past the ceiling the answer is a marker, and the two digests are
    /// unaffected — the file is fingerprinted normally, only its bytes
    /// are not kept.
    #[test]
    fn metadata_past_the_ceiling_is_a_marker_and_nothing_else_moves() {
        let big = "x".repeat(MAX_META_RAW_BYTES);
        let mut chunks = parse(CARD_PNG);
        chunks.insert(1, text("workflow", &big));
        let bytes = build(&chunks);

        assert_eq!(raw_of(&bytes, Some(&png_mime())), MetaRaw::TooLarge);
        assert_eq!(
            raw_of(&bytes, Some(&png_mime())).stored_value(),
            Some("unsupported:too-large".to_string()),
            "the content axis's word for the same sentence"
        );

        // The control: one byte under the ceiling is kept, so the
        // marker above is the ceiling firing rather than the reading
        // failing on a large file.
        let mut smaller = parse(CARD_PNG);
        smaller.insert(1, text("workflow", &"x".repeat(1024)));
        assert!(matches!(
            raw_of(&build(&smaller), Some(&png_mime())),
            MetaRaw::Captured(_)
        ));

        // And the file still fingerprints: the ceiling is about one
        // column, not about whether the artefact was read.
        assert!(matches!(
            region(&bytes, Some(&png_mime())),
            ContentRegion::Digest(_)
        ));
        assert_eq!(
            digest_of(&bytes),
            CARD_PNG_REGION,
            "metadata is outside the content region however large it is"
        );
        assert!(matches!(
            meta_of(&bytes, Some(&png_mime())),
            MaterialMeta::Digest { .. }
        ));
    }

    /// The cases that keep nothing, and the gate that refuses the
    /// reading outright.
    #[test]
    fn nothing_is_kept_for_a_file_with_no_metadata_a_broken_walk_or_no_claim() {
        // A walkable PNG carrying none of the five.
        assert_eq!(
            raw_of(&comfy_export(None, None), Some(&png_mime())),
            MetaRaw::Absent
        );
        // The text was read before the end was not: a partial walk is
        // not a container's metadata.
        let unterminated: Vec<Chunk> = parse(CARD_PNG)
            .into_iter()
            .filter(|(kind, _)| !is(kind, IEND_TAG))
            .collect();
        assert!(unterminated.iter().any(|(kind, _)| is(kind, b"tEXt")));
        assert_eq!(
            raw_of(&build(&unterminated), Some(&png_mime())),
            MetaRaw::Absent
        );
        // Bytes that are not a PNG's, under a claim that says they are.
        assert_eq!(
            raw_of(b"\xff\xd8\xff\xe0 not a png at all", Some(&png_mime())),
            MetaRaw::Absent
        );
        // And the gate: a row claiming nothing never reaches the
        // reading, whatever its first eight bytes say.
        assert_eq!(raw_of(CARD_PNG, None), MetaRaw::Absent);
        assert_eq!(raw_of(CARD_PNG, Some(&mime("image/jpeg"))), MetaRaw::Absent);
        // The control for all five: the claim restored keeps bytes.
        assert!(matches!(
            raw_of(CARD_PNG, Some(&png_mime())),
            MetaRaw::Captured(_)
        ));
    }

    /// The chunks the meta axis reads are exactly the ones the content
    /// axis drops, and no wider.
    ///
    /// `zTXt` and `iTXt` are excluded from both — a stated gap. This
    /// makes it measurable: adding either to a file moves neither
    /// digest, so a reading that quietly started decoding one would show
    /// up here rather than in a duplicate group.
    #[test]
    fn the_compressed_text_chunks_are_in_neither_axis() {
        let base = parse(CARD_PNG);
        let content_before = digest_of(CARD_PNG);
        let (meta_before, _) = digest_and_body(CARD_PNG);

        let mut widened = base.clone();
        widened.insert(
            1,
            (
                b"zTXt".to_vec(),
                b"workflow\0\0\x78\x9c\x03\x00\x00\x00\x00\x01".to_vec(),
            ),
        );
        widened.insert(
            2,
            (b"iTXt".to_vec(), b"Description\0\0\0\0\0a caption".to_vec()),
        );
        widened.insert(3, (b"tIME".to_vec(), vec![0x07, 0xea, 8, 9, 12, 0, 0]));
        widened.insert(4, (b"eXIf".to_vec(), b"II*\0\x08\0\0\0\0\0".to_vec()));

        let bytes = build(&widened);
        assert_ne!(bytes.as_slice(), CARD_PNG, "the file has to differ");
        assert_eq!(
            digest_of(&bytes),
            content_before,
            "the excluded five are excluded from the content region"
        );
        assert_eq!(
            digest_and_body(&bytes).0,
            meta_before,
            "and only tEXt reaches the meta digest"
        );
    }

    /// Two chunks that both axes would notice, as raw bytes with no
    /// signature in front of them — for appending to a file that already
    /// has one.
    ///
    /// One of each kind on purpose. The `gAMA` is inside the content
    /// region (it is not one of the excluded five) and the `tEXt` is the
    /// one chunk the meta axis reads, so a walk that ran past the end of
    /// the sequence moves whichever digest it belongs to, and the two
    /// cases are told apart rather than lumped into one failure.
    fn afterlife_chunks() -> Vec<Chunk> {
        vec![
            text("afterlife", "past the end of the sequence"),
            (b"gAMA".to_vec(), 100_000u32.to_be_bytes().to_vec()),
        ]
    }

    /// **`IEND` ends the file for both axes, and this is the fixture
    /// that says so.**
    ///
    /// The two walks reach the same conclusion by different code —
    /// `pngmeta::chunk_spans` on the content side, this crate's own
    /// `text_fields` loop on the meta side — and until now every fixture
    /// in the repo ended at its `IEND`, so nothing in the tree
    /// distinguished "stops at `IEND`" from "runs out of input there".
    /// The two answers are the same on a file where the end of the
    /// sequence and the end of the input coincide, and they are only the
    /// same on such a file.
    ///
    /// What that costs is not a lost assertion but a *disagreement*
    /// between the axes: a `pngmeta` bump, or a switch of the meta side
    /// to `pngmeta::read_text_chunks`, that moved one of them past the
    /// end marker would leave the two answering about different files —
    /// one about the picture, one about the picture plus whatever was
    /// appended to it. Nothing else here would notice, because every
    /// other assertion is relative and both sides would still be
    /// internally consistent.
    ///
    /// The control is the same two chunks placed *before* `IEND`, where
    /// each does move its own axis. Without it this test is satisfied by
    /// two walks that ignore the chunks wherever they sit.
    #[test]
    fn nothing_after_iend_reaches_either_axis() {
        let appended = {
            let mut bytes = CARD_PNG.to_vec();
            let tail = build(&afterlife_chunks());
            bytes.extend_from_slice(&tail[pngmeta::SIGNATURE.len()..]);
            bytes
        };
        assert!(
            appended.len() > CARD_PNG.len(),
            "the file has to carry the extra bytes for their absence to prove anything"
        );

        assert_eq!(
            digest_of(&appended),
            CARD_PNG_REGION,
            "the content walk stops at IEND"
        );
        assert_eq!(
            digest_and_body(&appended),
            digest_and_body(CARD_PNG),
            "and so does the text walk, digest and rendering alike"
        );

        // The control: the same two chunks one position earlier.
        let inside = {
            let mut chunks = parse(CARD_PNG);
            let at = chunks.len() - 1;
            for chunk in afterlife_chunks().into_iter().rev() {
                chunks.insert(at, chunk);
            }
            build(&chunks)
        };
        assert_ne!(
            digest_of(&inside),
            CARD_PNG_REGION,
            "gAMA before IEND is inside the content region — if this passes, \
             the assertion above is measuring nothing"
        );
        assert_ne!(
            digest_and_body(&inside).0,
            digest_and_body(CARD_PNG).0,
            "and a tEXt before IEND is metadata"
        );
    }
}
