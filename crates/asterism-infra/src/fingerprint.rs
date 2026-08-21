//! Reading one artefact off disk and answering every fingerprint axis
//! from that single read.
//!
//! Two callers, and they must not diverge: the `material_hash` job (the
//! ordinary pass over material that has just arrived) and the data
//! migrations that finish a column for material that was already here.
//! If they read a file differently — a different size gate, a different
//! mime source, one of them hashing the buffer while the other streams
//! it — the same file gets a digest through one door and a marker
//! through the other, and downstream that reads as two pictures.
//! Keeping the read in one function is what makes the agreement
//! structural rather than a thing two copies happen to have.

use asterism_core::domain::content_hash::{self, ContentHasher};
use asterism_core::domain::content_region;
use asterism_core::domain::embedded_text;
use asterism_core::domain::material_meta;
use asterism_core::domain::material_meta_raw::MetaRaw;
use asterism_core::domain::measurement::{Measurement, MeasurementStatus};
use asterism_core::domain::repository::MaterialFingerprint;
use asterism_core::domain::value::MimeType;

use crate::probes;

/// Read buffer for the streaming hash. 64 KiB is the usual sweet spot
/// between syscall count and resident memory; the point is that a 4 GB
/// video costs 64 KiB here rather than 4 GB.
const HASH_CHUNK_BYTES: usize = 64 * 1024;

/// Largest artefact this will read into memory whole for the content
/// axis.
///
/// The file axis streams and has no ceiling: 64 KiB of buffer answers a
/// 4 GB video. The content axis cannot, and the reason is in the PNG
/// probe's own doc — the pixel stream is fed to the hash as one
/// concatenation so that the same data split into a different number of
/// chunks hashes the same, which means a streaming reading would have to
/// hold every pixel chunk until the end (the allocation, in a worse
/// form) or read the file a second time. A probe takes a slice instead,
/// and deciding whether to produce that slice was left to "the job that
/// opens the file — it is the one that knows the file's size before
/// reading it". This is that decision.
///
/// **The gate is the job's, not a probe's**, and it stays here for the
/// same reason: "too large to read into memory" is a statement about
/// this process at this moment, made by the code that holds the open
/// handle. A probe is handed bytes or nothing.
///
/// 64 MiB, against a corpus whose PNGs run to a few megabytes [measured:
/// a 4,601-image ComfyUI corpus whose exports are single-digit MB;
/// character cards, the other PNG source here, run to a few hundred KB
/// — the card this repo shipped as a fixture was 551 KB before it was
/// replaced by a synthetic one]. The gate is two orders of
/// magnitude above what it is protecting against, so in practice it
/// fires on nothing that has a probe — video is what reaches gigabytes,
/// and video has no probe yet, so those files never reach the read at
/// all ([`probes::walks_content`] answers before the size is asked).
/// What the number really bounds is the pathological case: a PNG built
/// to be enormous, which would otherwise be a multi-gigabyte allocation
/// in a background job.
///
/// One buffer at a time — every caller is a sequential loop over one
/// page — so this is the peak, not a per-page cost.
pub(crate) const MAX_CONTENT_WALK_BYTES: u64 = 64 * 1024 * 1024;

/// Reads one artefact's embedded text and nothing else, rendered the
/// way the column stores it.
///
/// The recovery walk's reader. `hash_artefact` above would answer this
/// question too — it is one of the four axes it fills — but asking it
/// here would mean hashing every byte of every picture in the library
/// to recover a caption, and then throwing three digests away because
/// the row already carries them. This reads the bytes once and walks
/// them once.
///
/// `Ok(None)` means the file is over the ceiling: nobody has looked, and
/// the row stays a candidate for a later pass under a larger one. A
/// format this cannot read is the caller's question, asked before the
/// file is opened — see
/// [`embedded_text::walks_format`](asterism_core::domain::embedded_text::walks_format).
pub(crate) fn recover_embedded_text(
    path: &str,
    declared_mime: Option<&MimeType>,
    max_walk: u64,
) -> std::io::Result<Option<String>> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.len() > max_walk {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(embedded_text::render(
        embedded_text::recover(&bytes, declared_mime).as_ref(),
    )))
}

/// Reads one artefact **once** and answers every axis.
///
/// `declared_mime` is the material's `mime` — a guess from the
/// extension, which is what routes the file to a probe. `max_walk`
/// is the ceiling above which the walking axes are declined
/// ([`MAX_CONTENT_WALK_BYTES`] in production; a parameter so a test can
/// put a real file on the far side of the gate without writing 64 MiB).
///
/// Blocking. The job caller runs it inside `spawn_blocking`; the
/// migration caller is already on a blocking thread.
///
/// # One read, three answers
///
/// The axes want the bytes in different shapes — a stream for the file
/// digest, a slice for the probes — and the naive way to serve them is
/// to read the file once per axis. That multiplies the I/O of the pass
/// that dominates this work, over a corpus measured in gigabytes, to
/// avoid an allocation a probe needs anyway. So the file is opened
/// once and read once, in whichever of the two shapes the format and
/// the size call for:
///
/// - **A format some probe handles, inside the gate**: read whole,
///   hash the buffer, walk the same buffer once per reading. One
///   `read_to_end`, several passes over memory already paid for. The
///   first two readings select opposite halves of one container — the
///   bytes that decode, and the metadata that does not — so the second
///   walk is the cheap one, and the third (the metadata bytes kept as
///   they are, `material_meta_raw`) walks the same cheap half again.
/// - **Anything else**: stream in [`HASH_CHUNK_BYTES`] pieces, and
///   answer the walking axes with markers. Nothing is held.
///
/// **Either axis claiming the format is enough to read the file.**
/// The two gates are asked separately rather than through one shared
/// answer: they are two definitions, and the day one of them learns a
/// format the other does not, a single gate would either skip a walk
/// that would have worked or read a file nothing walks. The registry
/// keeps that shape across every probe it holds — see
/// [`probes`](crate::probes).
///
/// The size is taken from the open handle's metadata rather than from
/// `material.file_size_bytes`: the row records what an importer saw,
/// and the gate is a statement about this process's memory at this
/// moment. A file that grew since the import would otherwise walk
/// straight past it.
pub(crate) fn hash_artefact(
    path: &str,
    declared_mime: Option<&MimeType>,
    max_walk: u64,
) -> std::io::Result<MaterialFingerprint> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();

    // Asked before the read, because the answer decides what the read
    // costs. A format no probe handles never becomes a buffer, however
    // large it is — which is why the gate below is not the thing
    // protecting the process from a 4 GB video.
    if !probes::walks_content(declared_mime)
        && !probes::walks_meta(declared_mime)
        && !embedded_text::walks_format(declared_mime)
    {
        return Ok(MaterialFingerprint {
            file: Measurement::computed(stream_digest(&mut file)?),
            content: content_region::unsupported_format(declared_mime).record(),
            meta: material_meta::unsupported_format(declared_mime).record(),
            meta_kv: None,
            // NULL rather than a marker, which is what
            // `MetaRaw::Absent` renders to: this column holds a
            // container's bytes, and nothing here reads this
            // container's. The two markers beside it already say who
            // declined and why.
            meta_raw: MetaRaw::Absent.stored_value(),
            // No walk here has a reading of this format, so nobody has
            // looked and `NULL` is the true answer. Writing `{}` would
            // claim the bytes were read and carried no words, which
            // would retire the row from a pass that learns the format
            // later.
            meta_text: None,
        });
    }
    if size > max_walk {
        return Ok(MaterialFingerprint {
            file: Measurement::computed(stream_digest(&mut file)?),
            content: Measurement::bare(MeasurementStatus::TooLarge),
            // The same statement on this axis and for the same reason:
            // the metadata is there and could be read, and nothing
            // about the file is wrong — the policy declined to spend
            // the memory. A status that said "no walker" would send a
            // reader off to write one that exists.
            meta: Measurement::bare(MeasurementStatus::TooLarge),
            meta_kv: None,
            // And on the bytes, where the sentence is the same one
            // again — this time about a ceiling two orders of magnitude
            // above the probe's own. One word for both, because what a
            // reader has to know is identical: bytes exist and this
            // build chose not to hold them.
            meta_raw: MetaRaw::TooLarge.stored_value(),
            // Same again on the text side, and here the distinction
            // has teeth: the file was not read, so the row stays a
            // candidate for a later pass under a larger ceiling.
            meta_text: None,
        });
    }

    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    file.read_to_end(&mut bytes)?;
    let meta = probes::meta(&bytes, declared_mime);
    Ok(MaterialFingerprint {
        file: Measurement::computed(content_hash::of_bytes(&bytes)),
        content: probes::content(&bytes, declared_mime).record(),
        meta_kv: meta.canonical().map(str::to_string),
        meta: meta.record(),
        // A third walk over the same buffer, and the cheapest of the
        // three: it copies the metadata chunks and reads nothing else.
        // Taken here rather than out of the reading above because the
        // two answer different questions — what the container says, and
        // what it says it in — and a `MaterialMeta` that carried its own
        // input would put a megabyte behind every value the meta axis
        // passes around.
        meta_raw: probes::meta_raw(&bytes, declared_mime).stored_value(),
        // The walk whose output is a document rather than a digest.
        // Written once the bytes are in hand — `{}` when they carry no
        // words — because "read and empty" is what retires the row from
        // the recovery walk, and a `NULL` there would leave every
        // text-free picture in the library permanently pending.
        //
        // Gated on the format for the other half of that contract.
        // Reaching this line does not mean *this* walk was the one that
        // asked for the bytes: a JPEG is here because the content-region
        // walker reads it, and answering `{}` for it would say a text
        // recovery had looked at those bytes when none exists for that
        // format. `NULL` keeps the row waiting for the reader that
        // eventually handles it, which is the same rule the early
        // returns above follow.
        meta_text: embedded_text::walks_format(declared_mime)
            .then(|| embedded_text::render(embedded_text::recover(&bytes, declared_mime).as_ref())),
    })
}

/// The file axis on its own, in [`HASH_CHUNK_BYTES`] pieces — the path
/// for artefacts whose bytes are never held whole.
fn stream_digest(file: &mut std::fs::File) -> std::io::Result<String> {
    use std::io::Read;
    let mut hasher = ContentHasher::new();
    let mut buffer = vec![0u8; HASH_CHUNK_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The character-card PNG this repo already ships (IHDR / tEXt×2 /
    /// IDAT / IEND) — written by `scripts/gen-test-fixtures.py`, whose
    /// chunk writer shares no code with the walker under test, so the
    /// values below are not a fixture builder agreeing with itself.
    const CARD_PNG: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../asterism-importer-sdk/tests/fixtures/character-card-lyra.png"
    ));

    type Chunk = (&'static [u8; 4], Vec<u8>);

    /// PNG's CRC-32 (the standard reflected polynomial), so a built
    /// fixture is a real file rather than a shape only these walkers
    /// would accept.
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

    fn build(chunks: &[Chunk]) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, payload) in chunks {
            let length = u32::try_from(payload.len()).expect("fixture chunk fits in a PNG length");
            out.extend_from_slice(&length.to_be_bytes());
            out.extend_from_slice(*kind);
            out.extend_from_slice(payload);
            let mut crc_input = kind.to_vec();
            crc_input.extend_from_slice(payload);
            out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        }
        out
    }

    fn keyed(keyword: &str, rest: &[u8]) -> Vec<u8> {
        let mut payload = keyword.as_bytes().to_vec();
        payload.push(0);
        payload.extend_from_slice(rest);
        payload
    }

    /// A PNG carrying **every chunk the region definition excludes** —
    /// `tEXt`, `zTXt`, `iTXt`, `tIME`, `eXIf` — with a deterministic
    /// pixel stream cut into several `IDAT` chunks.
    ///
    /// One fixture rather than three because the two axes disagree about
    /// it in a way no other file here does: all five are outside the
    /// content region, and only the `tEXt` pair reaches the meta digest.
    /// A move that quietly widened either selection changes exactly one
    /// of the two literals frozen for this file.
    fn every_excluded_chunk() -> Vec<u8> {
        let mut state = 0x1234_5678u32;
        let pixels: Vec<u8> = (0..140 * 1024)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect();

        let mut chunks: Vec<Chunk> = vec![
            (b"IHDR", vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]),
            (b"tEXt", keyed("prompt", b"1girl, purple eyes")),
            // `keyword \0 method \0 <zlib>` — never decompressed by
            // either walker, so the payload only has to be bytes.
            (
                b"zTXt",
                keyed("workflow", b"\0\x78\x9c\x03\x00\x00\x00\x00\x01"),
            ),
            // `keyword \0 flag \0 method \0 lang \0 translated \0 <utf-8>`.
            (
                b"iTXt",
                keyed("Description", b"\0\0\0\0an international caption"),
            ),
            (b"tIME", vec![0x07, 0xea, 8, 9, 12, 0, 0]),
            (b"eXIf", b"II*\0\x08\0\0\0\0\0".to_vec()),
        ];
        for piece in pixels.chunks(64 * 1024) {
            chunks.push((b"IDAT", piece.to_vec()));
        }
        chunks.push((b"tEXt", keyed("Software", "ComfyUI".as_bytes())));
        chunks.push((b"IEND", Vec::new()));
        build(&chunks)
    }

    /// The smallest thing that is honestly a JPEG: SOI, an APP0/JFIF
    /// header, EOI.
    ///
    /// It carries no scan and no EXIF, which is the whole of what it
    /// says now that something walks JPEGs on both axes: `probes::jpeg`
    /// reads a region out of the entropy-coded data an `SOS`
    /// introduces, and reads its metadata out of an `APP1` this file
    /// does not have. So it is the marker case still, twice over, by two
    /// different routes and under one word. See the frozen row below for
    /// what moved and why.
    fn jpeg() -> Vec<u8> {
        let mut out = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
        out.extend_from_slice(b"JFIF\0");
        out.extend_from_slice(&[0x01, 0x02, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
        out.extend_from_slice(&[0xff, 0xd9]);
        out
    }

    /// One row's four stored columns, with `meta_kv` folded to the
    /// digest of itself.
    ///
    /// The fold is not a weakening. `meta` *is* the digest of the
    /// canonical rendering, so re-deriving it from the `meta_kv` column
    /// and comparing the two frozen strings pins that column to one
    /// exact byte sequence — and it additionally catches the failure a
    /// literal could not: a pass that stored a digest taken over one
    /// rendering beside a different rendering. The alternative was to
    /// paste a real character card's 30 KB of base64 into this file.
    type Row = (
        &'static str,
        Measurement,
        Measurement,
        Measurement,
        Option<String>,
    );

    /// **The stored fingerprint of these fixtures, frozen as literals.**
    ///
    /// Every other assertion about the fingerprint axes in this
    /// workspace is *relative* — these two agree, that one moves — and a
    /// pass that stopped hashing anything would satisfy a great many of
    /// them at once. This is the absolute anchor, and its subject is not
    /// the code: it is the set of strings already sitting in a Dogfood
    /// database's `material.content_hash` / `content_region_hash` /
    /// `meta_hash` / `meta_kv` columns. A build that produces different
    /// ones cannot be compared against the rows it has already written,
    /// so the failure is silent data corruption rather than a red test.
    ///
    /// **These values were measured before the walkers moved anywhere**
    /// — computed by running this test against the implementation that
    /// lived in `asterism_core::domain::{content_region, material_meta}`,
    /// then pasted in and watched to pass. That ordering is the whole
    /// content of the guarantee: literals written down after a move are
    /// a comparison of the new code with itself.
    ///
    /// A change to one of these is a `cr2-` / `m2-` decision — a new
    /// prefix and a re-walk of every stored row — never a refactor. A
    /// diff that edits a literal here to make a test pass has inverted
    /// the reason they exist.
    ///
    /// **The two `CARD_PNG` rows moved once, in 2026-08, and this is
    /// what moved them.** The card fixture was SillyTavern's
    /// `default_Seraphina.png` — AGPL-3.0, committed here in breach of
    /// both its licence and the test file's own claim that it was *not*
    /// committed — and it was replaced by a synthetic card written by
    /// `scripts/gen-test-fixtures.py`. Different bytes, therefore a
    /// different digest.
    ///
    /// What tells that apart from the inversion warned about above is
    /// the other two rows: `every_excluded_chunk` and `jpeg` are built
    /// from bytes defined inside this file, they were re-measured in
    /// the same run, and neither moved a character. A walker that had
    /// drifted would have moved all four. This edit is a change of
    /// subject, not of measurement — and the new literals were, like
    /// the originals, pasted from a measurement rather than from the
    /// hope that a test would go green.
    ///
    /// **The rows changed *shape* once more when V92 split status and
    /// digest apart** (issue #17): the marker strings the walking axes
    /// used to store became statuses beside a NULL digest, so the
    /// frozen form here is the status/digest/reason triple the columns
    /// hold now. No digest literal moved a character in that change —
    /// which is the property being protected, since V92 rewrites the
    /// representation and must not rewrite a measurement.
    #[test]
    fn these_fixtures_fingerprint_to_exactly_these_stored_values() {
        let dir = tempfile::tempdir().expect("tempdir");
        let excluded = every_excluded_chunk();
        let jpeg_bytes = jpeg();

        // (name, bytes, declared mime) — the mime is what routes the
        // file to a walker, so it is part of the fixture rather than a
        // detail of the call.
        let cases: [(&'static str, &[u8], Option<&str>); 4] = [
            ("a real PNG with tEXt", CARD_PNG, Some("image/png")),
            (
                "a PNG carrying every excluded chunk",
                &excluded,
                Some("image/png"),
            ),
            (
                "a JPEG: walked on both axes, and neither found anything",
                &jpeg_bytes,
                Some("image/jpeg"),
            ),
            ("a PNG whose row claims nothing", CARD_PNG, None),
        ];

        let measured: Vec<Row> = cases
            .iter()
            .enumerate()
            .map(|(index, (label, bytes, declared))| {
                let path = dir.path().join(format!("fixture-{index}.bin"));
                std::fs::write(&path, bytes).expect("fixture written");
                let parsed = declared.map(MimeType::parse);
                let got = hash_artefact(
                    path.to_str().expect("utf-8 path"),
                    parsed.as_ref(),
                    MAX_CONTENT_WALK_BYTES,
                )
                .expect("the fixture is readable");
                let rendered = got.meta_kv.as_deref().map(material_meta::digest_of);
                if index == 1 {
                    // One fixture's canonical rendering is short enough
                    // to read, so it is frozen as itself as well: the
                    // digests above say "this did not move", and this
                    // says what it is — sorted keys, no whitespace,
                    // values as the chunk carried them, and only the
                    // `tEXt` pair out of the five excluded chunks.
                    assert_eq!(
                        got.meta_kv.as_deref(),
                        Some(r#"{"Software":"ComfyUI","prompt":"1girl, purple eyes"}"#),
                    );
                }
                (*label, got.file, got.content, got.meta, rendered)
            })
            .collect();

        let frozen: Vec<Row> = vec![
            (
                "a real PNG with tEXt",
                Measurement::computed(
                    "sha256:7ac7081cf5c60dc198a557300c0bdf666e5a798da32af01359824ec813238e31"
                        .into(),
                ),
                Measurement::computed(
                    "cr1-sha256:10225b4d3a3709c47a985ecbf8ac9db0c4e3654cfbc0608032c8252a0205b7c9"
                        .into(),
                ),
                Measurement::computed(
                    "m1-sha256:eddd8329dd9e9f668395daaf7328db94c8db41190ee1f0f5a50b5907aa5eb7bd"
                        .into(),
                ),
                Some(
                    "m1-sha256:eddd8329dd9e9f668395daaf7328db94c8db41190ee1f0f5a50b5907aa5eb7bd"
                        .into(),
                ),
            ),
            (
                // Its content digest is the one the region tests already
                // freeze for a ComfyUI export built from the same header
                // and the same pixel stream with no `zTXt` / `iTXt` /
                // `tIME` / `eXIf` in it. The five excluded chunks
                // contributed nothing, and that is visible as one
                // literal appearing in two places rather than as a
                // claim.
                "a PNG carrying every excluded chunk",
                Measurement::computed(
                    "sha256:e79fded6bb0b1b48ee4f079314d70cc5a7927cc6fd44f1e332937e90ee8b5f7a"
                        .into(),
                ),
                Measurement::computed(
                    "cr1-sha256:b60d7dc769d32fee9a8b417381612225545f815d52447588b46a4ae6af799988"
                        .into(),
                ),
                Measurement::computed(
                    "m1-sha256:47557e7fde82911bec6a4a03759dc7a82fd71788e7a2c565f266810f76ee5034"
                        .into(),
                ),
                Some(
                    "m1-sha256:47557e7fde82911bec6a4a03759dc7a82fd71788e7a2c565f266810f76ee5034"
                        .into(),
                ),
            ),
            (
                // **The one literal in this test that has ever moved,
                // and what moved it.**
                //
                // Until `probes::jpeg` landed, nothing walked a JPEG on
                // either axis, and both columns held
                // `unsupported:image/jpeg` — "no probe answers for this
                // format". A probe now answers for it on the content
                // axis, so the content column stopped being that
                // sentence about the build and became a statement about
                // these bytes: they walk to a complete image with no
                // entropy-coded data in it, and a region over what is
                // left would be a digest every such stub shares. Hence
                // `unsupported:empty-span`, which is what the PNG walk
                // has always stored for a file with no `IDAT`.
                //
                // **And then the meta column moved, one slice later, for
                // the same reason.** The probe declared `meta: false`
                // while nothing could express a narrow enough reading of
                // EXIF; the series axis made one expressible, the claim
                // flipped, and this row stopped saying "nobody looked"
                // and started saying what these bytes are — an
                // APP0/JFIF header with no EXIF segment in it, which is
                // a reading that ran and found nothing. So the two
                // columns now carry the same word by two different
                // routes: no scan, and no metadata.
                //
                // The value is the load-bearing part rather than the
                // move. `unsupported:image/jpeg` is a **final** answer
                // to "has anybody looked", so a row holding it is one
                // the ordinary pass never returns to; `empty-span` is a
                // reading, and a later one can improve on it. The rows
                // that were already in a library keep the final answer
                // until a migration writes NULL over it — V72 did that
                // for the content column and V76 for this one.
                //
                // One thing this row still does not say: it is not a
                // change of either region definition. No value that was
                // a digest became a different digest — the two rows
                // that later did (the `CARD_PNG` pair, when the fixture
                // was replaced) moved for a reason on the other side of
                // the walker, described in this test's doc comment.
                "a JPEG: walked on both axes, and neither found anything",
                Measurement::computed(
                    "sha256:a638f3c452ed26e104b099dccbaff8dfeaf0d72e5c95709fb9d207a1713f511d"
                        .into(),
                ),
                Measurement::bare(MeasurementStatus::EmptySpan),
                Measurement::bare(MeasurementStatus::EmptySpan),
                None,
            ),
            (
                // The same bytes as the first case with the claim taken
                // away: the file axis is unchanged and the walking axes
                // carry the unsupported status, because nothing routes
                // an unnamed format to a walker.
                "a PNG whose row claims nothing",
                Measurement::computed(
                    "sha256:7ac7081cf5c60dc198a557300c0bdf666e5a798da32af01359824ec813238e31"
                        .into(),
                ),
                Measurement::unsupported("unknown".into()),
                Measurement::unsupported("unknown".into()),
                None,
            ),
        ];

        assert_eq!(measured, frozen);
    }

    /// Reads a file the way the job does, with the production ceiling.
    fn measure(bytes: &[u8], declared: Option<&str>) -> MaterialFingerprint {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("artefact.bin");
        std::fs::write(&path, bytes).expect("fixture written");
        let parsed = declared.map(MimeType::parse);
        hash_artefact(
            path.to_str().expect("utf-8 path"),
            parsed.as_ref(),
            MAX_CONTENT_WALK_BYTES,
        )
        .expect("the fixture is readable")
    }

    /// **The round trip — and the whole reason the column exists.**
    ///
    /// Take the row the pass wrote, decode `meta_raw`, read the chunks
    /// back out of it, and render them: the result is the `meta_kv` the
    /// same pass wrote. Without this the raw is bytes nobody can use.
    ///
    /// Every step goes through the surface a later consumer would use —
    /// `material_meta_raw::bytes_of` for the decode, the media probe's
    /// own `text_fields` for the walk, `material_meta::render` for the
    /// form — rather than through a reader written here to agree with
    /// the writer. The one thing this test supplies is the `IEND`, which
    /// is structural and therefore not metadata and therefore not in the
    /// raw: the kept bytes are the file's own framing, so a signature in
    /// front and a terminator behind produce a sequence **this same
    /// chunk walk** reads.
    ///
    /// Not a picture, and the difference matters to whoever consumes
    /// the raw next. Only the five metadata chunks are kept, so what is
    /// reassembled here has no `IHDR` and no `IDAT`: a walk over it
    /// yields the text chunks, and an image decoder — or
    /// `pngmeta::read_text_chunks`, which wants a file — will refuse
    /// it. The raw is a chunk sequence, and the way to read it is a
    /// chunk walk.
    ///
    /// The second fixture is the one that matters most. It carries all
    /// five excluded chunks and its text on both sides of its pixel
    /// data, so a raw that stopped at the first `IDAT`, or that kept
    /// payloads without their framing, walks back to a different map.
    ///
    /// **Teeth**: `expect` on the column, not `if let` — a pass that
    /// stopped writing the raw fails here rather than passing over an
    /// empty round trip [measured: making `PngProbe::meta_raw_of` return
    /// `MetaRaw::Absent` fails this test on `the fixture as it ships`
    /// with "the pass wrote no raw for a PNG carrying text", and leaves
    /// `these_fixtures_fingerprint_to_exactly_these_stored_values`
    /// green].
    #[test]
    fn the_raw_column_walks_back_to_the_meta_kv_column() {
        use asterism_core::domain::material_meta_raw;

        let excluded = every_excluded_chunk();
        for (label, bytes) in [
            ("the fixture as it ships", CARD_PNG),
            ("every excluded chunk, text on both sides", &excluded),
        ] {
            let got = measure(bytes, Some("image/png"));
            let stored = got
                .meta_raw
                .as_deref()
                .expect("the pass wrote no raw for a PNG carrying text");
            let raw = material_meta_raw::bytes_of(stored).expect("the column decodes");

            // A walkable chunk sequence, not a picture: signature,
            // the kept frames, a terminator. No IHDR and no IDAT are
            // in the raw, so this is what a chunk walk reads and not
            // what a decoder opens.
            let mut walkable = b"\x89PNG\r\n\x1a\n".to_vec();
            walkable.extend_from_slice(&raw);
            walkable.extend_from_slice(&build(&[(b"IEND", Vec::new())])[8..]);

            let fields = asterism_media_probe::png::text_fields(&walkable)
                .expect("the kept frames walk as a chunk sequence");
            assert_eq!(
                material_meta::render(&fields),
                got.meta_kv.expect("a PNG carrying text has an object"),
                "{label}: the raw renders to the same object the pass stored"
            );

            // …and it is a subset of the file rather than the file: a
            // reading that kept everything would satisfy the round trip
            // and put the pixels in the row.
            assert!(raw.len() < bytes.len() / 2, "{label}: {} bytes", raw.len());
        }
    }

    /// Past the raw ceiling the column takes a marker and the artefact
    /// is fingerprinted exactly as it would have been.
    ///
    /// The ceiling belongs to one column. A build that let it reach the
    /// others would answer three markers for a file whose picture and
    /// metadata are perfectly readable — so the assertion is that the
    /// other three columns are the ones the same bytes take without the
    /// oversized chunk.
    #[test]
    fn metadata_past_the_raw_ceiling_marks_one_column_and_no_other() {
        use asterism_core::domain::content_region::TOO_LARGE;

        // The probe's ceiling, spelled out rather than imported: it is
        // `probes::png`'s to state, and a test that read the constant
        // would keep passing whatever it was moved to.
        let over: Vec<Chunk> = vec![
            (b"IHDR", vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]),
            (b"tEXt", keyed("workflow", &vec![b'x'; 1024 * 1024])),
            (b"IDAT", vec![7; 64]),
            (b"IEND", Vec::new()),
        ];
        let under: Vec<Chunk> = vec![
            (b"IHDR", vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]),
            (b"tEXt", keyed("workflow", b"small enough")),
            (b"IDAT", vec![7; 64]),
            (b"IEND", Vec::new()),
        ];

        // Compared as an equality and reported as a summary: the value
        // this would hold if the ceiling stopped firing is a megabyte of
        // base64, and a failure message is not the place for it.
        let big = measure(&build(&over), Some("image/png"));
        let kept = big.meta_raw.as_deref().unwrap_or("<null>");
        assert!(
            kept == TOO_LARGE,
            "the policy declined to keep them, in the content axis's own word — \
             got {} bytes starting {:.32}",
            kept.len(),
            kept
        );
        assert!(
            big.content.digest().is_some_and(|d| d.starts_with("cr1-"))
                && big.meta.digest().is_some_and(|d| d.starts_with("m1-")),
            "the file is walked normally: {:?} / {:?}",
            big.content,
            big.meta
        );

        // The control: the same shape under the ceiling keeps its bytes,
        // so the marker above is the ceiling firing rather than the
        // reading failing on a large file.
        let small = measure(&build(&under), Some("image/png"));
        assert!(
            small
                .meta_raw
                .as_deref()
                .is_some_and(|value| value.starts_with("undefined:")),
            "{:?}",
            small.meta_raw
        );

        // And the two files' other three columns differ only where the
        // chunk itself differs — the content axis drops metadata, so
        // that column is the *same* on both.
        assert_eq!(small.content, big.content, "metadata is not the picture");
        assert_ne!(small.meta, big.meta, "and it is the metadata");
    }

    /// A format nothing reads the metadata of stores NULL rather than a
    /// marker — the column holds a container's bytes, and a statement
    /// about this build in it would be read as one.
    #[test]
    fn a_format_that_keeps_no_bytes_leaves_the_column_null() {
        // A claimed format whose file carries none: the reading ran, the
        // container states no EXIF, and there are no bytes to keep. This
        // case used to be the other one — JPEG declined the meta axis
        // entirely, and the row held `unsupported:image/jpeg` — and the
        // move is the point: a marker naming the format says nobody
        // looked, which is a **final** answer, while `empty-span` says a
        // reading found nothing and leaves the row where a later one can
        // improve on it.
        let jpeg = measure(&jpeg(), Some("image/jpeg"));
        assert_eq!(jpeg.meta_raw, None);
        assert_eq!(jpeg.meta, Measurement::bare(MeasurementStatus::EmptySpan));
        assert_eq!(jpeg.meta_kv, None);

        // No probe, and no claim at all.
        assert_eq!(measure(b"not a picture", Some("video/mp4")).meta_raw, None);
        assert_eq!(measure(CARD_PNG, None).meta_raw, None);

        // A PNG with nothing to keep: read, walked, and carrying no
        // metadata chunk — NULL again, since there are no bytes rather
        // than a refusal to hold them.
        let bare = build(&[
            (b"IHDR", vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]),
            (b"IDAT", vec![7; 64]),
            (b"IEND", Vec::new()),
        ]);
        let walked = measure(&bare, Some("image/png"));
        assert_eq!(walked.meta_raw, None);
        assert_eq!(walked.meta, Measurement::bare(MeasurementStatus::EmptySpan));
        assert!(
            walked
                .content
                .digest()
                .is_some_and(|d| d.starts_with("cr1-"))
        );
    }

    /// Past the **file** gate nothing is read, and every walking column
    /// says so with one word.
    #[test]
    fn a_file_past_the_size_gate_keeps_no_bytes_either() {
        // The same gate the frozen test uses, lowered so a real file can
        // sit on the far side of it without writing 64 MiB.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("large.png");
        std::fs::write(&path, CARD_PNG).expect("fixture written");
        let mime = MimeType::parse("image/png");
        let got = hash_artefact(path.to_str().expect("utf-8 path"), Some(&mime), 1024)
            .expect("the fixture is readable");

        assert_eq!(got.content, Measurement::bare(MeasurementStatus::TooLarge));
        assert_eq!(got.meta, Measurement::bare(MeasurementStatus::TooLarge));
        assert_eq!(
            got.meta_raw.as_deref(),
            Some("unsupported:too-large"),
            "one sentence on every column that would have needed the bytes"
        );
        assert_eq!(got.meta_kv, None);
        // The file axis still answers, which is what makes the three
        // statuses a statement about a policy rather than about a
        // failure to read.
        assert!(got.file.digest().is_some_and(|d| d.starts_with("sha256:")));
    }
}
