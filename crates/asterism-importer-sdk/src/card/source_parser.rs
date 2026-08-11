//! [`SourceParser`] adapter that turns any character-card [`RawItem`]
//! (PNG tEXt or standalone JSON) into per-slot [`Footprint`]s.
//!
//! Composes the pipeline:
//! [`RawItem`] → [`envelope_from_png`] / [`CardEnvelope::from_json`] →
//! [`CardParserRegistry::dispatch`] → `Vec<Footprint>`.
//!
//! Importer binaries plug this straight into an [`FsScanner`](crate::FsScanner)
//! so they only need to configure the source_kind slug and the batch
//! posting loop.

use chrono::Utc;
use serde_json::Value;

use crate::Footprint;
use crate::bundle::session_id_for;
use crate::parser::{ParseError, SourceParser};
use crate::scanner::RawItem;

use super::envelope::{CardContext, CardEnvelope};
use super::png_chunk::{envelope_from_png, is_png};
use super::registry::CardParserRegistry;

/// [`SourceParser`] that decodes character cards from PNG tEXt chunks
/// or standalone JSON and dispatches through a
/// [`CardParserRegistry`].
///
/// Configure the `platform` label (`"SillyTavern"`, `"CharacterHub"`,
/// …) via [`Self::with_platform`] and, when custom vendor parsers are
/// needed, hand in a pre-built registry via [`Self::with_registry`].
pub struct CharaSourceParser {
    registry: CardParserRegistry,
    platform: Option<String>,
}

impl CharaSourceParser {
    /// New parser with defaults ([`CardParserRegistry::with_defaults`],
    /// no platform label).
    pub fn new() -> Self {
        Self {
            registry: CardParserRegistry::with_defaults(),
            platform: None,
        }
    }

    /// Attach a platform string that flows into every emitted
    /// [`FootprintSource::platform`](crate::FootprintSource::platform).
    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }

    /// Swap in a custom registry (for adapter packs that register
    /// vendor-specific `CharacterCardParser`s beyond V2 / V3).
    pub fn with_registry(mut self, registry: CardParserRegistry) -> Self {
        self.registry = registry;
        self
    }

    /// Try to decode a payload as either PNG (chara / ccv3 chunk) or
    /// standalone JSON envelope. Returns `None` when the payload is
    /// neither a recognised PNG card nor a valid card envelope.
    pub fn envelope_from_payload(payload: &[u8]) -> Option<CardEnvelope> {
        if is_png(payload) {
            envelope_from_png(payload)
        } else {
            let s = std::str::from_utf8(payload).ok()?;
            let v: Value = serde_json::from_str(s).ok()?;
            CardEnvelope::from_json(v)
        }
    }
}

impl Default for CharaSourceParser {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceParser for CharaSourceParser {
    fn parse(&self, item: RawItem) -> Result<Vec<Footprint>, ParseError> {
        let Some(env) = Self::envelope_from_payload(&item.payload) else {
            // Unrecognised shape (non-card PNG, non-envelope JSON) —
            // skip silently. The scanner's extension filter usually
            // keeps us from getting here, but a `.png` avatar without
            // an embedded card is a legitimate case.
            return Ok(Vec::new());
        };
        let session_id = session_id_for(&item.locator);
        let occurred_at = item.occurred_at.unwrap_or_else(Utc::now);
        let ctx = CardContext {
            source_kind: &item.source_kind,
            locator: &item.locator,
            session_id: &session_id,
            occurred_at,
            platform: self.platform.as_deref(),
        };
        // `dispatch` returns None when the envelope's spec has no
        // registered parser; treat that the same as an unrecognised
        // shape (skip, do not fail the whole batch).
        Ok(self.registry.dispatch(&env, &ctx).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde_json::json;

    fn v2_card_bytes() -> Vec<u8> {
        let card = json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": { "name": "Alice", "description": "curious", "first_mes": "hi" }
        });
        serde_json::to_vec(&card).unwrap()
    }

    fn v2_png_bytes() -> Vec<u8> {
        let card = json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": { "name": "Bob", "first_mes": "yo" }
        });
        let b64 = STANDARD.encode(serde_json::to_string(&card).unwrap());
        build_png_with_text_chunk("chara", &b64)
    }

    /// Build a minimal PNG carrying a single `tEXt` chunk. The card
    /// parser reads `tEXt` payloads and never validates CRCs or looks at
    /// IHDR pixel bytes, so the fixture rides on `PngBuilder`'s default
    /// 1×1 grayscale IHDR — the same shape every other pngmeta consumer
    /// uses.
    fn build_png_with_text_chunk(keyword: &str, text: &str) -> Vec<u8> {
        pngmeta::test_util::PngBuilder::new()
            .text(keyword, text)
            .build()
    }

    #[test]
    fn parses_standalone_json_card() {
        let parser = CharaSourceParser::new();
        let item = RawItem {
            source_kind: "chara".into(),
            locator: "/tmp/alice.json".into(),
            payload: v2_card_bytes(),
            occurred_at: None,
            extra: json!({}),
        };
        let out = parser.parse(item).unwrap();
        assert!(!out.is_empty(), "V2 JSON payload produces footprints");
        // 2 Notes (name, description) + 1 ChatMessage (first_mes) = 3.
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn parses_png_chara_chunk() {
        let parser = CharaSourceParser::new();
        let item = RawItem {
            source_kind: "chara".into(),
            locator: "/tmp/bob.png".into(),
            payload: v2_png_bytes(),
            occurred_at: None,
            extra: json!({}),
        };
        let out = parser.parse(item).unwrap();
        // 1 Note (name) + 1 ChatMessage (first_mes) = 2.
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn returns_empty_vec_for_unrecognised_payload() {
        let parser = CharaSourceParser::new();
        let item = RawItem {
            source_kind: "chara".into(),
            locator: "/tmp/random.png".into(),
            payload: b"\x89PNG\r\n\x1a\ngarbage".to_vec(),
            occurred_at: None,
            extra: json!({}),
        };
        let out = parser.parse(item).unwrap();
        assert!(
            out.is_empty(),
            "malformed PNG yields no footprints, no panic"
        );
    }

    #[test]
    fn platform_flows_into_footprint_source() {
        let parser = CharaSourceParser::new().with_platform("SillyTavern");
        let item = RawItem {
            source_kind: "chara".into(),
            locator: "/tmp/alice.json".into(),
            payload: v2_card_bytes(),
            occurred_at: None,
            extra: json!({}),
        };
        let out = parser.parse(item).unwrap();
        for f in &out {
            let plat = match f {
                Footprint::Note(n) => n.source.platform.as_deref(),
                Footprint::ChatMessage(m) => m.source.platform.as_deref(),
                Footprint::Doc(d) => d.source.platform.as_deref(),
                Footprint::Image(i) => i.source.platform.as_deref(),
                Footprint::JournalEntry(j) => j.source.platform.as_deref(),
                Footprint::Video(v) => v.source.platform.as_deref(),
                Footprint::Audio(a) => a.source.platform.as_deref(),
                Footprint::Tape(t) => t.source.platform.as_deref(),
            };
            assert_eq!(plat, Some("SillyTavern"));
        }
    }
}
