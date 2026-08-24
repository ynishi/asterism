//! `head_registry` — the instance's carriage of a trained tag head
//! (#132 phase 3).
//!
//! The artifact (`asterism-tag-head-v1`) is what a member's training
//! run writes locally: the per-tag rows, the encoder identity they
//! were trained against, and the held-out eval. It is kilobytes of
//! JSON, which is why it rides this registry row whole — no blob
//! store involved. The instance is a carrier, not an authority, the
//! same stance the route took when it carried the model entry
//! (#127): it stores and re-serves the publisher's bytes **verbatim**
//! and deliberately types nothing of the body — no rows, no eval, no
//! floors. Verification belongs to the member app, which re-runs the
//! same checks its startup bind runs before a pulled head may score.
//!
//! What *is* validated is the envelope — the part the instance
//! answers for as a carrier: the bytes are one JSON object, the
//! `schema` field names the one artifact version this instance
//! carries, `head` is the non-empty label supersession history is
//! keyed by, and the encoder identity fields are present — a pull
//! must be able to refuse a head trained against another encoder
//! before parsing anything deeper.
//!
//! The schema string is the app-side artifact's
//! (`asterism-infra`'s head store writes it); it is re-spelled here
//! rather than imported because `teams-*` depends on `asterism-core`
//! only (#83 §4) — the same one-notation-two-spellings shape as the
//! digest grammar note in [`crate::domain::store`].

use serde_json::Value;

use crate::error::DomainError;

/// The one artifact schema this plane carries. A future `-v2` is a
/// new constant and an explicit decision, not a silent pass-through.
pub const HEAD_ENTRY_SCHEMA_V1: &str = "asterism-tag-head-v1";

/// A validated head entry: the publisher's bytes, verbatim, plus the
/// envelope facts the instance keys carriage by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagHeadEntry {
    raw: String,
    label: String,
}

impl TagHeadEntry {
    /// Validates the envelope and keeps the bytes verbatim.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let value: Value = serde_json::from_str(raw)
            .map_err(|e| DomainError::Validation(format!("a head entry must be JSON: {e}")))?;
        let Value::Object(fields) = &value else {
            return Err(DomainError::Validation(
                "a head entry is a JSON object, not an array or scalar".into(),
            ));
        };
        match fields.get("schema").and_then(Value::as_str) {
            Some(HEAD_ENTRY_SCHEMA_V1) => {}
            Some(other) => {
                return Err(DomainError::Validation(format!(
                    "entry schema {other:?} is not one this instance carries; \
                     expected {HEAD_ENTRY_SCHEMA_V1:?}"
                )));
            }
            None => {
                return Err(DomainError::Validation(format!(
                    "a head entry names its schema; \
                     expected a \"schema\" field holding {HEAD_ENTRY_SCHEMA_V1:?}"
                )));
            }
        }
        let label = match fields.get("head").and_then(Value::as_str) {
            Some(label) if !label.trim().is_empty() => label.to_string(),
            _ => {
                return Err(DomainError::Validation(
                    "a head entry carries a non-empty \"head\" label; \
                     without one, supersession history has nothing to key by"
                        .into(),
                ));
            }
        };
        // The encoder identity must be sayable from the envelope: a
        // member refuses a head trained against another encoder, and
        // that refusal must not require trusting anything deeper.
        if fields
            .get("model_id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
            || fields.get("dim").and_then(Value::as_u64).is_none()
            || fields
                .get("preprocess_ver")
                .and_then(Value::as_u64)
                .is_none()
        {
            return Err(DomainError::Validation(
                "a head entry names the encoder it was trained against \
                 (model_id, dim, preprocess_ver)"
                    .into(),
            ));
        }
        Ok(Self {
            raw: raw.to_string(),
            label,
        })
    }

    /// The publisher's bytes, exactly as authored.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// The head's label — the envelope's history key.
    pub fn label(&self) -> &str {
        &self.label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v1_entry() -> String {
        serde_json::json!({
            "schema": HEAD_ENTRY_SCHEMA_V1,
            "head": "head-v3",
            "model_id": "siglip2-base-patch16-256-q4v",
            "dim": 768,
            "preprocess_ver": 1,
            "rows": { "00000000-0000-0000-0000-000000000001": { "weights": [0.1], "bias": 0.0 } },
        })
        .to_string()
    }

    #[test]
    fn a_v1_entry_parses_and_the_bytes_survive_verbatim() {
        let raw = format!("  {}\n", v1_entry());
        let entry = TagHeadEntry::parse(&raw).expect("parse");
        assert_eq!(entry.raw(), raw);
        assert_eq!(entry.label(), "head-v3");
    }

    #[test]
    fn the_envelope_is_validated_and_nothing_deeper() {
        // A body the app side would still have to verify (no rows at
        // all) is carriable: the instance answers for the envelope
        // only.
        let thin = serde_json::json!({
            "schema": HEAD_ENTRY_SCHEMA_V1,
            "head": "h",
            "model_id": "m",
            "dim": 4,
            "preprocess_ver": 1,
        })
        .to_string();
        assert!(TagHeadEntry::parse(&thin).is_ok());
    }

    #[test]
    fn wrong_or_missing_envelopes_are_refused() {
        let base = |patch: fn(&mut serde_json::Map<String, serde_json::Value>)| {
            let mut value: serde_json::Value = serde_json::from_str(&v1_entry()).unwrap();
            patch(value.as_object_mut().unwrap());
            value.to_string()
        };
        for (raw, why) in [
            ("not json".to_string(), "not JSON"),
            ("[1, 2]".to_string(), "not an object"),
            (
                base(|f| {
                    f.remove("schema");
                }),
                "no schema field",
            ),
            (
                base(|f| {
                    f.insert("schema".into(), "asterism-tag-head-v2".into());
                }),
                "a schema this instance does not carry",
            ),
            (
                base(|f| {
                    f.remove("head");
                }),
                "no label",
            ),
            (
                base(|f| {
                    f.insert("head".into(), "  ".into());
                }),
                "blank label",
            ),
            (
                base(|f| {
                    f.remove("model_id");
                }),
                "no encoder identity",
            ),
            (
                base(|f| {
                    f.remove("dim");
                }),
                "no dim",
            ),
        ] {
            assert!(
                matches!(TagHeadEntry::parse(&raw), Err(DomainError::Validation(_))),
                "{why}: {raw}"
            );
        }
    }
}
