//! `embedded_text` — the words a container wrote *into* an artefact,
//! recovered for search rather than for identity.
//!
//! # Why this is not [`material_meta`](crate::domain::material_meta)
//!
//! The two read the same chunks off the same bytes and answer different
//! questions, and the difference is the reason both exist.
//!
//! `material_meta` defines a **digest**. Its reading of a file has to be
//! total and frozen: two equal metadata sets must render identically or
//! the axis stops grouping, so that module fixes one decoding, reads
//! `tEXt` and nothing else, and says so — "a different answer is a `m2-`
//! generation rather than an edit". Widening it would silently redefine
//! what every stored `meta_hash` meant.
//!
//! This module defines a **document**. Nothing downstream compares two
//! of its outputs for equality; the output is tokenised and thrown into
//! a haystack. So it can be generous where the digest cannot:
//!
//! - **`zTXt` and `iTXt` are read.** Compressed and international text
//!   were excluded from both stored axes — "a stated gap rather than a
//!   silent one" — and the gap is only defensible for a digest. A
//!   caption a person can see in their image viewer and cannot find by
//!   searching for it is the whole complaint this answers.
//! - **Latin-1 is recovered rather than replaced.** `tEXt` is Latin-1
//!   by the spec and arbitrary bytes in practice, so the digest side
//!   reads it with [`String::from_utf8_lossy`], which turns every byte
//!   above 0x7F that is not part of a valid UTF-8 sequence into
//!   U+FFFD — a total function, which is what a digest needs, and a
//!   shredder for the accented words in `Café` or `Größe`. Here the
//!   bytes are tried as UTF-8 first (what generators actually write,
//!   spec or no spec) and read as Latin-1 when that fails, so no byte
//!   is lost either way.
//! - **A malformed tail keeps what came before it.** The digest side
//!   refuses a chunk sequence that never reaches `IEND`, because what
//!   it collected is part of a file rather than a file. A truncated
//!   file's caption is still a caption, so the walk stops and keeps.
//!
//! # PNG only
//!
//! The same bound its two siblings carry. EXIF, XMP and ID3 all hold
//! words about their artefact and none of them is read here — the
//! recovery is per-container-format work, and this is the format the
//! corpus's text actually travels in. [`walks_format`] is what a caller
//! asks before spending a read.
//!
//! # Where the bytes come from
//!
//! From the caller, as a slice, and only ever from the pass that is
//! already holding them: `fingerprint::hash_artefact` reads an artefact
//! once and answers every axis off that one buffer, and this is a third
//! walk over memory already paid for. Nothing on the indexing path
//! opens a file for this — the recovered text is stored on the material
//! (`material.meta_text`), and a job that re-composes a document reads
//! the column.

use std::collections::BTreeMap;

use crate::domain::material_meta;
use crate::domain::value::{ImageFormat, MimeType};

/// Ceiling on the text one artefact contributes.
///
/// `pngmeta` caps a single decompressed chunk at 64 MiB, which bounds
/// one bomb and not a file full of them. This bounds the walk: a
/// generation prompt is measured in kilobytes, a ComfyUI workflow blob
/// in tens of them, and a megabyte of recovered annotation is already
/// far past the point where more of it improves anyone's search. What
/// is over the line is dropped chunk by chunk, so the words before it
/// still land.
const MAX_RECOVERED_BYTES: usize = 1024 * 1024;

/// Separator between two chunks that share a keyword.
///
/// PNG allows a keyword to repeat and the map form cannot, so the digest
/// side collapses them, last occurrence winning. Dropping a sentence
/// because another chunk happened to be filed under the same word is a
/// loss no search wants, so they are joined instead — a newline, which
/// is what separates two sections everywhere else on this path.
const REPEAT_JOIN: &str = "\n";

/// Whether a declared format has a recovery walk here — the question a
/// caller asks **before** reading anything.
///
/// Its own function rather than a call to
/// [`material_meta::walks_format`], on the same terms that module gives
/// for not calling `content_region`'s: they are separate definitions,
/// and the day one learns a format the other does not, a shared answer
/// would either read a file nothing walks or skip one that does.
pub fn walks_format(declared_mime: Option<&MimeType>) -> bool {
    matches!(declared_mime, Some(MimeType::Image(ImageFormat::Png)))
}

/// Recovers every text annotation an artefact carries, keyed by the
/// keyword the container filed it under.
///
/// `declared_mime` is what the row believes the file is, on the same
/// terms its siblings take it: a guess from the extension, which lies
/// in both directions, so the signature has to agree with it before
/// anything is read.
///
/// `None` means there is nothing to recover — a format with no walk
/// here, bytes that are not a PNG, or a PNG carrying no text at all.
/// Distinct from `Some(empty)`, which cannot be produced: a walk that
/// found nothing answers `None`, and the caller renders that as the
/// empty object so that "read, and it carried nothing" stays tellable
/// from "never read".
pub fn recover(bytes: &[u8], declared_mime: Option<&MimeType>) -> Option<BTreeMap<String, String>> {
    let claimed = declared_mime.filter(|mime| !mime.as_str().is_empty());
    if let Some(mime) = claimed
        && !matches!(mime, MimeType::Image(ImageFormat::Png))
    {
        return None;
    }
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    let mut budget = MAX_RECOVERED_BYTES;

    for item in pngmeta::chunk_spans(bytes).ok()? {
        // A structural defect ends the iterator, and what was already
        // read stays read. See the module doc: this is the axis where
        // a partial answer beats no answer.
        let Ok((span, payload)) = item else { break };
        if !span.kind.is_text() {
            continue;
        }
        let Some((keyword, text)) = decode(span.kind, payload, span.offset) else {
            continue;
        };
        if keyword.is_empty() || text.trim().is_empty() {
            continue;
        }
        if text.len() > budget {
            continue;
        }
        budget -= text.len();
        match fields.entry(keyword) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(text);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                let joined = slot.get_mut();
                joined.push_str(REPEAT_JOIN);
                joined.push_str(&text);
            }
        }
    }

    (!fields.is_empty()).then_some(fields)
}

/// Renders recovered fields the way the column stores them.
///
/// The same canonical JSON `material_meta` renders — sorted keys,
/// compact, values exactly as the container stated them. Reused rather
/// than re-derived because the reader on the other side is one code
/// path over both columns, and two renderings would make which column a
/// value came from something that reader had to know.
///
/// `None` renders as the empty object rather than as SQL `NULL`. That
/// is the whole three-state contract of the column: `NULL` is "nobody
/// has looked", `{}` is "looked, and these bytes carry no words", and a
/// populated object is the words. Without the middle state a PNG that
/// genuinely carries nothing is re-read by every backfill pass forever.
pub fn render(fields: Option<&BTreeMap<String, String>>) -> String {
    match fields {
        Some(fields) => material_meta::render(fields),
        None => material_meta::render(&BTreeMap::new()),
    }
}

/// Decodes one text chunk to `(keyword, text)`, or `None` when its
/// payload does not match the layout its type requires.
///
/// `tEXt` is taken apart here and the other two are handed to
/// `pngmeta`, and the split is not arbitrary. `tEXt` is the chunk this
/// corpus's text actually travels in and the one whose payload is
/// Latin-1, so it is the one worth decoding byte by byte; `pngmeta`
/// would hand back a `String` that had already been through
/// `from_utf8_lossy`, with the bytes gone. `zTXt` and `iTXt` need
/// zlib to reach their payload at all, and the decompressed bytes are
/// not exposed — so those two keep the lossy reading, which costs
/// nothing for `iTXt` (UTF-8 by the spec) and is a carried limitation
/// for a `zTXt` chunk written in Latin-1.
fn decode(kind: pngmeta::ChunkType, payload: &[u8], offset: u64) -> Option<(String, String)> {
    if kind == pngmeta::ChunkType::TEXT {
        let separator = payload.iter().position(|byte| *byte == 0)?;
        return Some((
            text_of(&payload[..separator]),
            text_of(&payload[separator + 1..]),
        ));
    }
    // A corrupt zlib stream, or a compression method PNG does not
    // define, drops its own chunk and no other — the sibling chunk
    // holding the prompt is not the one that is broken.
    let entry = pngmeta::decode_text(kind, payload, offset).ok()??;
    Some((entry.keyword, entry.text))
}

/// Reads a chunk payload as the text somebody wrote, losing no byte.
///
/// UTF-8 first because that is what generators write into `tEXt`
/// whatever the spec says, and Latin-1 when that fails because that is
/// what the spec says. Both readings are total, so there is no third
/// case and no replacement character: every input maps to some string,
/// and for the two encodings that actually occur it maps to the right
/// one.
fn text_of(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        // Latin-1 is the first 256 code points, in order, so the byte
        // *is* the scalar value. This is the decode, not an
        // approximation of one.
        Err(_) => bytes.iter().map(|byte| *byte as char).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pngmeta::test_util::PngBuilder;

    fn png() -> MimeType {
        MimeType::parse("image/png")
    }

    /// The gap this module exists to close: a prompt stored compressed
    /// is invisible to the digest walk and has to reach the document.
    #[test]
    fn a_compressed_chunk_is_recovered() {
        let bytes = PngBuilder::new()
            .ztxt("parameters", "a lighthouse at dusk, long exposure")
            .build();

        let fields = recover(&bytes, Some(&png())).expect("the chunk carries text");

        assert_eq!(
            fields.get("parameters").map(String::as_str),
            Some("a lighthouse at dusk, long exposure")
        );
        // The digest side still does not see it — the two walks
        // disagree on purpose. That half of the property lives with the
        // walk that owns it now: the PNG probe's meta axis excludes
        // `zTXt`, and its own tests hold the exclusion (the meta walk
        // moved behind `probes` in asterism-infra, out of this crate's
        // reach).
    }

    /// `iTXt` is where a caption written in a language with its own
    /// script lands, and it was in neither stored axis.
    #[test]
    fn international_text_is_recovered_with_its_script_intact() {
        let bytes = PngBuilder::new()
            .itxt("Description", "ja", "説明", "灯台と夕暮れ")
            .build();

        let fields = recover(&bytes, Some(&png())).expect("the chunk carries text");

        assert_eq!(
            fields.get("Description").map(String::as_str),
            Some("灯台と夕暮れ")
        );
    }

    /// The half a digest cannot have: a Latin-1 byte is the character
    /// somebody typed, not a replacement mark.
    ///
    /// The same bytes through the meta axis come back with U+FFFD in
    /// them, which is why the assertion is written as the pair that
    /// disagrees rather than as one call's output.
    #[test]
    fn latin1_bytes_survive_here_and_do_not_survive_the_digest() {
        // `Caf\xE9` — Latin-1, and not valid UTF-8.
        let payload = b"comment\0Caf\xe9 window";
        let bytes = PngBuilder::new()
            .raw_chunk(*b"tEXt", payload.len() as u32, payload)
            .build();

        let fields = recover(&bytes, Some(&png())).expect("the chunk carries text");
        assert_eq!(
            fields.get("comment").map(String::as_str),
            Some("Café window")
        );

        // The digest side replaces the byte with U+FFFD — which is what
        // this module is for. The comparison is the PNG probe's to make
        // now (the meta walk moved behind `probes` in asterism-infra),
        // so this test holds only the recovery half.
    }

    /// UTF-8 written into a `tEXt` chunk is read as UTF-8. Generators
    /// do this regardless of the spec, and reading those bytes as
    /// Latin-1 would produce mojibake out of a string that was already
    /// correct.
    #[test]
    fn utf8_written_into_a_latin1_chunk_is_read_as_utf8() {
        let bytes = PngBuilder::new().text("prompt", "灯台, 夕暮れ").build();
        let fields = recover(&bytes, Some(&png())).expect("the chunk carries text");
        assert_eq!(
            fields.get("prompt").map(String::as_str),
            Some("灯台, 夕暮れ")
        );
    }

    /// A keyword may repeat, and the second occurrence is a second
    /// sentence rather than a correction of the first.
    #[test]
    fn a_repeated_keyword_keeps_both_texts() {
        let bytes = PngBuilder::new()
            .text("Comment", "shot on the north pier")
            .text("Comment", "printed for the hallway")
            .build();

        let fields = recover(&bytes, Some(&png())).expect("the chunks carry text");
        let comment = fields.get("Comment").expect("one entry, both texts");
        assert!(comment.contains("north pier"), "{comment}");
        assert!(comment.contains("hallway"), "{comment}");
    }

    /// One broken chunk costs its own text and no other's — the sibling
    /// holding the prompt is not the one that is corrupt.
    #[test]
    fn a_corrupt_compressed_chunk_does_not_take_its_neighbours_with_it() {
        let bytes = PngBuilder::new()
            .raw_chunk(*b"zTXt", 9, b"kw\0\0\x78\x01\xff\xff\xff")
            .text("prompt", "a lighthouse")
            .build();

        let fields = recover(&bytes, Some(&png())).expect("the readable chunk still reads");
        assert_eq!(
            fields.get("prompt").map(String::as_str),
            Some("a lighthouse")
        );
        assert!(!fields.contains_key("kw"), "{fields:?}");
    }

    /// A file that never reaches `IEND` keeps the words read before the
    /// cut. The digest side refuses the same input, and the divergence
    /// is the point of having two walks.
    #[test]
    fn a_truncated_file_keeps_what_was_read_before_the_cut() {
        let whole = PngBuilder::new().text("prompt", "a lighthouse").build();
        let cut = whole.len() - 12;
        let truncated = &whole[..cut];

        let fields = recover(truncated, Some(&png())).expect("the chunk before the cut read");
        assert_eq!(
            fields.get("prompt").map(String::as_str),
            Some("a lighthouse")
        );
        // A digest over part of a file is not a digest over the file —
        // the meta axis refuses the truncation. That refusal is the PNG
        // probe's, tested where the walk lives (asterism-infra's
        // `probes`); recovery deliberately keeps what was read instead.
    }

    /// Nothing to recover is `None`, and `None` renders as the marker
    /// that stops the next backfill pass from reading the file again.
    #[test]
    fn a_png_carrying_no_words_answers_none_and_renders_as_looked_and_empty() {
        let bytes = PngBuilder::new().build();
        assert_eq!(recover(&bytes, Some(&png())), None);
        assert_eq!(render(None), "{}");
    }

    /// An empty value names nothing and neither does an empty keyword:
    /// a document is not improved by a term that separates no two rows.
    #[test]
    fn blank_keywords_and_blank_values_are_not_entries() {
        let bytes = PngBuilder::new()
            .text("Software", "   ")
            .text("prompt", "a lighthouse")
            .build();

        let fields = recover(&bytes, Some(&png())).expect("one real entry");
        assert_eq!(fields.len(), 1, "{fields:?}");
        assert!(fields.contains_key("prompt"));
    }

    /// Both the claim and the signature have to agree — the same rule
    /// the two stored axes follow, for the same reason: a mime is a
    /// guess from a filename and lies in both directions.
    #[test]
    fn the_format_has_to_be_a_png_by_both_the_claim_and_the_bytes() {
        let bytes = PngBuilder::new().text("prompt", "a lighthouse").build();
        assert_eq!(recover(&bytes, Some(&MimeType::parse("image/jpeg"))), None);
        assert_eq!(recover(b"not a png at all", Some(&png())), None);

        assert!(walks_format(Some(&png())));
        assert!(!walks_format(Some(&MimeType::text_plain())));
        assert!(!walks_format(None));
    }

    /// The ceiling bounds the walk rather than ending it: a chunk over
    /// the line is dropped and the ones that fit still land.
    #[test]
    fn an_oversized_chunk_is_dropped_and_its_neighbours_are_not() {
        let huge = "x".repeat(MAX_RECOVERED_BYTES + 1);
        let bytes = PngBuilder::new()
            .text("workflow", &huge)
            .text("prompt", "a lighthouse")
            .build();

        let fields = recover(&bytes, Some(&png())).expect("the small chunk fits");
        assert_eq!(
            fields.get("prompt").map(String::as_str),
            Some("a lighthouse")
        );
        assert!(!fields.contains_key("workflow"), "{:?}", fields.keys());
    }

    /// The rendering is the column's, and it round-trips through the
    /// reader on the other side — the same shape `material.meta_kv`
    /// holds, so one code path reads both.
    #[test]
    fn the_rendering_is_the_canonical_object_both_columns_hold() {
        let bytes = PngBuilder::new()
            .ztxt("parameters", "a lighthouse")
            .text("Software", "a generator")
            .build();
        let rendered = render(recover(&bytes, Some(&png())).as_ref());

        let parsed: BTreeMap<String, String> =
            serde_json::from_str(&rendered).expect("the column parses as the object it is");
        assert_eq!(
            parsed.get("parameters").map(String::as_str),
            Some("a lighthouse")
        );
        assert_eq!(
            parsed.get("Software").map(String::as_str),
            Some("a generator")
        );
        // Sorted and compact, so two equal sets render identically.
        assert!(rendered.starts_with(r#"{"Software":"#), "{rendered}");
    }
}
