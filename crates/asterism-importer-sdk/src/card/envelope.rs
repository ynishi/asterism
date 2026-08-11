//! Envelope + context types shared by every character-card parser.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::FootprintSource;

/// Parsed character-card envelope.
///
/// A character card, regardless of container (PNG tEXt chunk,
/// standalone `.json`, `.charx` inner file), decodes to one JSON
/// object with three top-level fields: `spec` (`"chara_card_v2"` /
/// `"chara_card_v3"` / vendor slug), `spec_version` (`"2.0"` /
/// `"3.0"`), and `data{…}` (the actual card content). This struct is
/// the parsed form; the raw envelope is kept in `raw` so parsers that
/// need vendor-specific top-level fields (rare) can still reach them.
#[derive(Debug, Clone)]
pub struct CardEnvelope {
    /// `spec` string. Used by [`crate::card::CardParserRegistry`] to
    /// dispatch to the right parser.
    pub spec: String,
    /// `spec_version` string. Empty when the envelope omits it.
    pub spec_version: String,
    /// The `data{…}` object — where every canonical slot
    /// (`name`, `description`, `first_mes`, `character_book`, …)
    /// lives. Guaranteed to be a JSON object by
    /// [`Self::from_json`].
    pub data: Value,
    /// Full envelope as parsed, retained for parsers that need to
    /// reach top-level fields outside `data{…}` (rare).
    pub raw: Value,
}

impl CardEnvelope {
    /// Parse a JSON value into an envelope.
    ///
    /// Returns `None` when the shape does not match the character-card
    /// envelope contract — missing `spec` string, missing `data`
    /// object, or `data` that is not itself a JSON object. Callers
    /// that need lenient behaviour (accept envelopeless cards) should
    /// build the envelope by hand.
    pub fn from_json(v: Value) -> Option<Self> {
        let obj = v.as_object()?;
        let spec = obj.get("spec")?.as_str()?.to_string();
        let spec_version = obj
            .get("spec_version")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let data = obj.get("data")?.clone();
        if !data.is_object() {
            return None;
        }
        Some(Self {
            spec,
            spec_version,
            data,
            raw: v,
        })
    }

    /// Convenience accessor for the extensions bag
    /// (`data.extensions.<vendor>.*`). Returns `Value::Null` when the
    /// card carries no extensions.
    pub fn extensions(&self) -> Value {
        self.data.get("extensions").cloned().unwrap_or(Value::Null)
    }
}

/// Caller-supplied context every parser needs.
///
/// Kept separate from [`CardEnvelope`] because the same envelope may
/// be re-ingested against a different persona / at a different
/// occurred-at without touching the parsed card.
#[derive(Debug, Clone)]
pub struct CardContext<'a> {
    /// Source kind slug the importer wants written on every emitted
    /// footprint (see [`crate::scanner::RawItem::source_kind`] for
    /// ownership rules).
    pub source_kind: &'a str,
    /// Container locator — the caller's stable id for the *card* as a
    /// whole (typically the PNG or `.json` path). Every per-slot
    /// footprint derives its own locator by appending a suffix
    /// (`#field=<slot>` / `#greeting=<i>` / `#book_entry=<uid>` — see
    /// [`crate::catalogue`] for the taxonomy).
    pub locator: &'a str,
    /// Shared per-card grouping key. Callers usually derive it via
    /// [`crate::bundle::session_id_for`]`(container_locator)`.
    /// Fanout at emit time: the parsers route it into
    /// `ChatMessage.external_session_key` (dialogue greetings)
    /// and `bundle_id` on `Note` / `Doc` / `Image` so the domain
    /// draws a `same-bundle` edge across the whole sibling set. The
    /// field name
    /// stays `session_id` for card-local familiarity; treat it as an
    /// opaque key rather than a Session id.
    pub session_id: &'a str,
    /// Occurred-at time to stamp on every footprint. Cards do not
    /// carry per-record timestamps, so parsers assign the same value
    /// to all outputs; the caller resolves it via the fallback ladder
    /// documented on [`crate::parser::SourceParser`].
    pub occurred_at: DateTime<Utc>,
    /// Optional platform label (`"SillyTavern"`, `"CharacterHub"`,
    /// `"RisuAI"`, …). Flows into every [`FootprintSource::platform`].
    pub platform: Option<&'a str>,
}

impl<'a> CardContext<'a> {
    /// Build a [`FootprintSource`] with `<locator><separator><suffix>`
    /// (the separator is `#` for standard suffixes; callers pass e.g.
    /// `"field=name"` or `"greeting=0"`).
    pub fn footprint_source(&self, suffix: &str) -> FootprintSource {
        FootprintSource {
            kind: self.source_kind.to_string(),
            locator: format!("{}#{suffix}", self.locator),
            platform: self.platform.map(String::from),
            // A card's fields are addressed by suffix, and the card
            // states no id of its own for them to be named by.
            external_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn envelope_parses_minimal_v2() {
        let v = json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": { "name": "Alice" }
        });
        let env = CardEnvelope::from_json(v).expect("minimal V2 parses");
        assert_eq!(env.spec, "chara_card_v2");
        assert_eq!(env.spec_version, "2.0");
        assert_eq!(env.data.get("name").and_then(|v| v.as_str()), Some("Alice"));
    }

    #[test]
    fn envelope_rejects_missing_data() {
        let v = json!({ "spec": "chara_card_v2", "spec_version": "2.0" });
        assert!(CardEnvelope::from_json(v).is_none());
    }

    #[test]
    fn envelope_rejects_missing_spec() {
        let v = json!({ "data": {} });
        assert!(CardEnvelope::from_json(v).is_none());
    }

    #[test]
    fn envelope_rejects_non_object_data() {
        let v = json!({ "spec": "chara_card_v2", "data": "not an object" });
        assert!(CardEnvelope::from_json(v).is_none());
    }

    #[test]
    fn envelope_extensions_defaults_to_null() {
        let v = json!({ "spec": "chara_card_v2", "data": {} });
        let env = CardEnvelope::from_json(v).unwrap();
        assert_eq!(env.extensions(), Value::Null);
    }

    #[test]
    fn context_footprint_source_appends_suffix() {
        let now = Utc::now();
        let ctx = CardContext {
            source_kind: "chara",
            locator: "/tmp/alice.png",
            session_id: "sid-42",
            occurred_at: now,
            platform: Some("SillyTavern"),
        };
        let src = ctx.footprint_source("field=name");
        assert_eq!(src.kind, "chara");
        assert_eq!(src.locator, "/tmp/alice.png#field=name");
        assert_eq!(src.platform.as_deref(), Some("SillyTavern"));
    }
}
