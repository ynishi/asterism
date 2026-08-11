//! persona-journal SQLite row -> `Footprint::JournalEntry` parser.
//!
//! Each row emitted by the `SqliteScanner` corresponds to one
//! persona-journal `entries` row joined with its current `versions`
//! body. We route each row to a `JournalEntry` whose `JournalKind` is
//! chosen from the `entries.kind` column.
//!
//! Constellation-edge grouping (persona × kind bucket) is carried on
//! `bundle_id` after the session-model refactor — journal is a
//! non-Dialog modality, so Session-related fields stay unused.
//!
//! Legacy note: the original mapping table:
//!
//! - `"states"`   -> `JournalKind::State`
//! - `"emo"`      -> `JournalKind::Emo`
//! - `"non_rem"`  -> `JournalKind::NonRem`
//! - `"memories"` -> `JournalKind::Memory`
//! - anything else round-trips via `JournalKind::Other(kind)`.
//!
//! Locator shape: `{db_path}#{entry_id}` (produced by `SqliteScanner`).
//! Idempotency relies on the server-side
//! `asset.source_kind + source_locator` unique constraint.

use asterism_importer_sdk::{
    Footprint, FootprintSource, JournalEntry, JournalKind, ParseError, RawItem, SourceParser,
};
use chrono::Utc;
use serde_json::{Value, json};

/// Parser for persona-journal rows.
pub struct PersonaJournalParser {
    /// Persona short name (e.g. "rin", "aya") — used as the
    /// `bundle_id` prefix and as an extra label so downstream filters
    /// can pivot on persona without resolving asset owner ids.
    pub persona_name: String,
}

impl SourceParser for PersonaJournalParser {
    fn parse(&self, item: RawItem) -> Result<Vec<Footprint>, ParseError> {
        let extra = match &item.extra {
            Value::Object(_) => item.extra.clone(),
            _ => json!({}),
        };

        let kind_slug = extra
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let seq_in_kind = extra
            .get("seq_in_kind")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let tags_csv = extra
            .get("tags")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_default();

        let body = std::str::from_utf8(&item.payload)
            .map_err(|e| ParseError::Malformed {
                locator: item.locator.clone(),
                message: format!("body not utf-8: {e}"),
            })?
            .to_string();

        if body.trim().is_empty() {
            return Ok(Vec::new());
        }

        let occurred_at = item.occurred_at.unwrap_or_else(Utc::now);

        let kind = match kind_slug.as_str() {
            "states" => JournalKind::State,
            "emo" => JournalKind::Emo,
            "non_rem" => JournalKind::NonRem,
            "memories" => JournalKind::Memory,
            other => JournalKind::Other(other.to_string()),
        };

        let mut labels: Vec<String> = Vec::new();
        labels.push(format!("persona:{}", self.persona_name));
        // Keep the raw persona-journal kind slug as a label so downstream
        // filters can still pivot on it even though modality is now the
        // primary axis.
        labels.push(format!("journal_kind:{}", kind_slug));
        for tag in tags_csv.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            labels.push(tag.to_string());
        }

        // journal is non-Dialog — the persona × kind pair becomes the
        // constellation-edge bundle key, not a Session key.
        let bundle_id = Some(format!(
            "persona-journal/{}/{}",
            self.persona_name, kind_slug
        ));

        let extra_out = json!({
            "kind": kind_slug,
            "seq_in_kind": seq_in_kind,
            "tags": tags_csv,
            "persona_name": self.persona_name,
        });

        let entry = JournalEntry {
            source: FootprintSource {
                kind: "persona-journal".into(),
                locator: item.locator,
                platform: Some("persona-journal".into()),
                external_id: None,
            },
            occurred_at,
            kind,
            body,
            bundle_id,
            labels,
            extra: extra_out,
        };

        Ok(vec![Footprint::JournalEntry(entry)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_importer_sdk::RawItem;
    use serde_json::json;

    fn raw(kind: &str, body: &str) -> RawItem {
        RawItem {
            source_kind: "persona-journal".into(),
            locator: "/tmp/aya.db#42".into(),
            payload: body.as_bytes().to_vec(),
            occurred_at: None,
            extra: json!({
                "kind": kind,
                "seq_in_kind": "5",
                "tags": "morning,soft",
            }),
        }
    }

    #[test]
    fn emits_bundle_id_and_leaves_session_axes_empty() {
        // P3 (session-model): persona-journal is a non-Dialog
        // modality, so the persona/kind grouping key must land on
        // `bundle_id`. Any leakage into `session_id` /
        // `external_session_key` would trip the AssetService
        // Dialog-only guard on the server side.
        let parser = PersonaJournalParser {
            persona_name: "aya".into(),
        };
        let out = parser.parse(raw("states", "gentle morning")).unwrap();
        assert_eq!(out.len(), 1);
        let spec = out.into_iter().next().unwrap().into_asset_spec();
        assert_eq!(spec.modality.as_deref(), Some("state"));
        assert_eq!(
            spec.bundle_id.as_deref(),
            Some("persona-journal/aya/states")
        );
        assert_eq!(spec.session_id, None);
        assert_eq!(spec.external_session_key, None);
    }

    #[test]
    fn different_kind_yields_different_bundle_id() {
        // Idempotency of the bundle key across the (persona, kind)
        // pair — same inputs produce the same key, different kind
        // produces a distinct bucket the edge fabric can group on.
        let parser = PersonaJournalParser {
            persona_name: "aya".into(),
        };
        let a = parser
            .parse(raw("states", "a"))
            .unwrap()
            .pop()
            .unwrap()
            .into_asset_spec()
            .bundle_id;
        let b = parser
            .parse(raw("emo", "b"))
            .unwrap()
            .pop()
            .unwrap()
            .into_asset_spec()
            .bundle_id;
        assert_ne!(a, b);
    }
}
