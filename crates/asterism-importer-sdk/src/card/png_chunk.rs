//! Character-card PNG `tEXt` chunk decoders.
//!
//! Two chunk keywords are canonical:
//! [`CHARA_KEYWORD`] (V2, base64 UTF-8 JSON) and [`CCV3_KEYWORD`] (V3,
//! same encoding). On read prefer `ccv3` when both are present; the
//! PNG importer or a character-card CLI uses
//! [`envelope_from_chunk`] to lift each chunk value into a
//! [`CardEnvelope`].
//!
//! Chunk framing comes from `pngmeta`: where one chunk ends and the
//! next begins has a single right answer, and a card reader is the
//! wrong place to re-derive it. What stays here is the card-specific
//! half — which keyword wins, and how a chunk value becomes an
//! envelope.
//!
//! A card is the one thing a PNG's `tEXt` chunks are read for here.
//! **Not** metadata in general: an ordinary image's chunks are that
//! image's own metadata and are hashed off its bytes server-side on the
//! `Meta` axis (`asterism-core::domain::material_meta`), with no reader
//! on this side at all. A character card is a different claim — the
//! chunk carries an envelope whose slots are separately addressable
//! records — and [`envelope_from_chunk`] is where that claim is made.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use pngmeta::{ChunkReader, TextEntry, decode_text};
use serde_json::Value;

use super::envelope::CardEnvelope;

/// PNG `tEXt` chunk keyword carrying a V2 character card.
pub const CHARA_KEYWORD: &str = "chara";
/// PNG `tEXt` chunk keyword carrying a V3 character card.
pub const CCV3_KEYWORD: &str = "ccv3";

/// Byte-level PNG detection, re-exported rather than wrapped so that
/// the cheap gate the character-card
/// [`SourceParser`](crate::SourceParser) impl uses to choose between
/// the PNG path and the JSON path gives exactly the answer the walker
/// that reads the bytes next would give.
pub use pngmeta::is_png;

/// Every text chunk in `bytes`, in file order, duplicates kept.
///
/// # Why not `pngmeta::read_text_entries`
///
/// That function fails the whole read on any structural defect, which
/// is right for a metadata reader and wrong here: a card's entire value
/// sits in one chunk, and losing a decodable card because the file's
/// tail was cut off would be a worse answer than the one the old
/// hand-rolled walker gave. So the walk stops at the first defect and
/// keeps what it already has. The importer side makes the opposite
/// call — see `asterism-importer-image`'s `png_text`, where the image
/// has already landed and the notes are the extra.
///
/// Chunk-level malformation (a `tEXt` with no NUL separator, a corrupt
/// zlib stream) skips that one chunk and the walk continues, because a
/// defect in one annotation should not hide the others.
///
/// `pngmeta` splits at the first NUL and `from_utf8_lossy`s both
/// halves — byte for byte what this module used to do by hand. tEXt
/// payload is Latin-1 per PNG spec; character cards keep their base64
/// in the ASCII subset, so that round trip is lossless for the payloads
/// this SDK cares about.
fn text_entries(bytes: &[u8]) -> Vec<TextEntry> {
    let Ok(mut reader) = ChunkReader::from_bytes(bytes) else {
        // Not a PNG at all — the JSON envelope path handles it.
        return Vec::new();
    };
    let mut out = Vec::new();
    // `Err` and `Ok(None)` both end the loop: the first is the defect
    // we tolerate, the second is `IEND`.
    while let Ok(Some(span)) = reader.next_span() {
        if !span.kind.is_text() {
            continue;
        }
        let Ok(data) = reader.read_data() else { break };
        if let Ok(Some(entry)) = decode_text(span.kind, data, span.offset) {
            out.push(entry);
        }
    }
    out
}

/// Walk a PNG and lift the character card out of it.
///
/// `ccv3` (V3) outranks `chara` (V2) per the spec-mandated read
/// priority: every `ccv3` chunk is tried before any `chara` chunk.
///
/// # Duplicate keywords
///
/// PNG permits repeated keywords and editors produce them, because
/// appending a chunk is cheaper than rewriting one. Within a keyword
/// this walks the chunks in file order and takes **the first one that
/// decodes** — not the first, not the last.
///
/// The failure this guards against is not "which copy is newer" but
/// "one copy is unusable": a half-written or garbage `chara` left
/// behind by a broken writer would, under a strict first-wins rule,
/// mask a perfectly good second copy and lose the card entirely, and
/// last-wins loses it the other way round. Trying each candidate is the
/// only rule that never discards a readable card. When two copies both
/// decode the earlier one wins, since file order is the only order the
/// container gives us and a rule that depends on which editor wrote
/// last is not a rule.
///
/// With exactly one chunk per keyword — every card in the wild — this
/// is indistinguishable from the map lookup it replaces.
pub fn envelope_from_png(bytes: &[u8]) -> Option<CardEnvelope> {
    let entries = text_entries(bytes);
    [CCV3_KEYWORD, CHARA_KEYWORD].into_iter().find_map(|kw| {
        entries
            .iter()
            .filter(|e| e.keyword == kw)
            .find_map(|e| envelope_from_chunk(&e.text))
    })
}

/// Decode a `chara` or `ccv3` chunk value into a [`CardEnvelope`].
///
/// The chunk value is expected to be base64-encoded UTF-8 JSON per the
/// V2 / V3 spec. Returns `None` when base64 decoding, UTF-8 decoding,
/// or JSON parsing fails, or when the resulting JSON does not match
/// the envelope shape ([`CardEnvelope::from_json`]).
///
/// Whitespace around the base64 payload is trimmed before decoding
/// because some editors round-trip a trailing newline into the chunk.
pub fn envelope_from_chunk(raw_value: &str) -> Option<CardEnvelope> {
    let bytes = STANDARD.decode(raw_value.trim()).ok()?;
    let json_str = std::str::from_utf8(&bytes).ok()?;
    let value: Value = serde_json::from_str(json_str).ok()?;
    CardEnvelope::from_json(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pngmeta::test_util::PngBuilder;
    use serde_json::json;

    fn encode(v: &Value) -> String {
        STANDARD.encode(serde_json::to_string(v).unwrap())
    }

    /// A V2 card whose `data.name` is `name`, base64-encoded the way a
    /// chunk carries it.
    fn card_chunk(name: &str) -> String {
        encode(&json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": { "name": name }
        }))
    }

    fn name_of(env: &CardEnvelope) -> Option<&str> {
        env.data.get("name").and_then(|v| v.as_str())
    }

    #[test]
    fn roundtrips_minimal_v2() {
        let card = json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": { "name": "Alice", "description": "an example" }
        });
        let chunk = encode(&card);
        let env = envelope_from_chunk(&chunk).expect("decoded");
        assert_eq!(env.spec, "chara_card_v2");
        assert_eq!(env.data.get("name").and_then(|v| v.as_str()), Some("Alice"));
    }

    #[test]
    fn tolerates_trailing_whitespace() {
        let card = json!({ "spec": "chara_card_v2", "data": {} });
        let mut chunk = encode(&card);
        chunk.push('\n');
        assert!(envelope_from_chunk(&chunk).is_some());
    }

    #[test]
    fn rejects_non_base64() {
        assert!(envelope_from_chunk("not base64 !@#").is_none());
    }

    #[test]
    fn rejects_base64_of_non_json() {
        let chunk = STANDARD.encode("hello world");
        assert!(envelope_from_chunk(&chunk).is_none());
    }

    #[test]
    fn rejects_json_missing_envelope_shape() {
        // Decodes cleanly but has no `spec` — envelope layer rejects it.
        let chunk = STANDARD.encode(r#"{"name":"Alice"}"#);
        assert!(envelope_from_chunk(&chunk).is_none());
    }

    #[test]
    fn ccv3_outranks_chara_regardless_of_chunk_order() {
        // The V3 chunk sits *after* the V2 one here, so a rule that
        // read in file order rather than keyword priority would answer
        // "Two".
        let png = PngBuilder::new()
            .text(CHARA_KEYWORD, &card_chunk("Two"))
            .text(CCV3_KEYWORD, &card_chunk("Three"))
            .build();
        let env = envelope_from_png(&png).expect("card decoded");
        assert_eq!(name_of(&env), Some("Three"));
    }

    #[test]
    fn duplicate_keyword_takes_the_earlier_chunk_when_both_decode() {
        // Two `chara` chunks, both valid. File order decides. A map
        // keyed by keyword would answer "Later" — the collapse this
        // walk exists to stop.
        let png = PngBuilder::new()
            .text(CHARA_KEYWORD, &card_chunk("Earlier"))
            .text(CHARA_KEYWORD, &card_chunk("Later"))
            .build();
        let env = envelope_from_png(&png).expect("card decoded");
        assert_eq!(name_of(&env), Some("Earlier"));
    }

    #[test]
    fn duplicate_keyword_skips_the_copy_that_does_not_decode() {
        // A half-written first copy must not mask a readable second
        // one; strict first-wins would answer None here.
        let png = PngBuilder::new()
            .text(CHARA_KEYWORD, "not base64 !@#")
            .text(CHARA_KEYWORD, &card_chunk("Readable"))
            .build();
        let env = envelope_from_png(&png).expect("card decoded");
        assert_eq!(name_of(&env), Some("Readable"));
    }

    #[test]
    fn a_card_stored_in_a_compressed_chunk_is_read() {
        // `zTXt` is legal and no card in the wild uses it, but with the
        // `inflate` feature off this file would report
        // `CompressionUnsupported` and the card would be lost to a
        // build flag.
        let png = PngBuilder::new()
            .ztxt(CHARA_KEYWORD, &card_chunk("Compressed"))
            .build();
        let env = envelope_from_png(&png).expect("card decoded");
        assert_eq!(name_of(&env), Some("Compressed"));
    }

    #[test]
    fn a_cut_off_tail_does_not_cost_the_card_in_front_of_it() {
        // Everything up to the defect is kept: the card chunk is
        // already behind us when the file runs out.
        let png = PngBuilder::new()
            .text(CHARA_KEYWORD, &card_chunk("Survivor"))
            .text("workflow", "trailing metadata")
            .build_truncated(6);
        let env = envelope_from_png(&png).expect("card decoded");
        assert_eq!(name_of(&env), Some("Survivor"));
    }

    #[test]
    fn a_png_without_a_card_chunk_yields_nothing() {
        let png = PngBuilder::new().text("prompt", "1girl, solo").build();
        assert!(envelope_from_png(&png).is_none());
    }

    #[test]
    fn non_png_input_yields_nothing() {
        assert!(envelope_from_png(b"\xff\xd8\xff\xe0 jpeg").is_none());
        assert!(envelope_from_png(b"").is_none());
    }
}
