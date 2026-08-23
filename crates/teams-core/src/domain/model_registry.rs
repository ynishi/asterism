//! `model_registry` — the instance's carriage of a qualified model's
//! registry entry (#126).
//!
//! The entry (`asterism-model-registry-entry-v1`) is authored by the
//! provider's tooling (`asterism-model-lab registry`) and consumed by
//! the member app's fetch flow. Between the two, the instance is a
//! carrier, not an authority (#126 decision 2): it stores and re-serves
//! the provider's bytes **verbatim**, and this module deliberately
//! types no field of the entry's body — no digests, no URLs, no
//! qualification report. Parsing those here would grow the hosted
//! plane a reading of the model contract that #83 §4's dependency rule
//! keeps out (`teams-*` → `asterism-core` only; the entry's typed home
//! is beside `ModelPackage`, on the app side of the split).
//!
//! What *is* validated is the envelope — the part the instance answers
//! for as a carrier: the bytes are one JSON object, the `schema` field
//! names the one version this instance knows how to carry, and
//! `model_id` is a non-empty string the storage layer can key history
//! by. A carrier that accepted arbitrary bytes would serve members
//! something their fetch flow cannot consume, and could not say which
//! model superseded which.

use serde_json::Value;

use crate::error::DomainError;

/// The one entry schema this plane carries. A future `-v2` is a new
/// constant and an explicit decision, not a silent pass-through.
pub const ENTRY_SCHEMA_V1: &str = "asterism-model-registry-entry-v1";

/// A validated registry entry: the provider's bytes, verbatim, plus
/// the two envelope facts the instance keys carriage by.
///
/// The raw text is the value. Serving anything else — a re-serialized,
/// key-reordered, whitespace-normalized rendering — would mean the
/// bytes a member verifies against are bytes the provider never
/// authored, for no gain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRegistryEntry {
    raw: String,
    model_id: String,
}

impl ModelRegistryEntry {
    /// Validates the envelope and keeps the bytes verbatim.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let value: Value = serde_json::from_str(raw)
            .map_err(|e| DomainError::Validation(format!("a registry entry must be JSON: {e}")))?;
        let Value::Object(fields) = &value else {
            return Err(DomainError::Validation(
                "a registry entry is a JSON object, not an array or scalar".into(),
            ));
        };
        match fields.get("schema").and_then(Value::as_str) {
            Some(ENTRY_SCHEMA_V1) => {}
            Some(other) => {
                return Err(DomainError::Validation(format!(
                    "entry schema {other:?} is not one this instance carries; \
                     expected {ENTRY_SCHEMA_V1:?}"
                )));
            }
            None => {
                return Err(DomainError::Validation(format!(
                    "a registry entry names its schema; \
                     expected a \"schema\" field holding {ENTRY_SCHEMA_V1:?}"
                )));
            }
        }
        let model_id = match fields.get("model_id").and_then(Value::as_str) {
            Some(id) if !id.trim().is_empty() => id.to_string(),
            _ => {
                return Err(DomainError::Validation(
                    "a registry entry carries a non-empty \"model_id\" string; \
                     without one, supersession history has nothing to key by"
                        .into(),
                ));
            }
        };
        Ok(Self {
            raw: raw.to_string(),
            model_id,
        })
    }

    /// The provider's bytes, exactly as authored.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// The model the entry describes — the envelope's history key.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v1_entry() -> String {
        serde_json::json!({
            "schema": ENTRY_SCHEMA_V1,
            "model_id": "siglip2-base-patch16-256",
            "dim": 768,
            "image_model": { "path": "vision_model.onnx", "sha256": "ab", "url": "https://x" },
        })
        .to_string()
    }

    #[test]
    fn a_v1_entry_parses_and_the_bytes_survive_verbatim() {
        // Whitespace and key order the provider chose are part of the
        // carried bytes — parse must not launder them.
        let raw = format!("  {}\n", v1_entry());
        let entry = ModelRegistryEntry::parse(&raw).expect("parse");
        assert_eq!(entry.raw(), raw);
        assert_eq!(entry.model_id(), "siglip2-base-patch16-256");
    }

    #[test]
    fn the_envelope_is_validated_and_nothing_deeper() {
        // A body the app side would refuse (no digests, no files) is
        // still carriable: the instance answers for the envelope only.
        let thin = serde_json::json!({
            "schema": ENTRY_SCHEMA_V1,
            "model_id": "m",
        })
        .to_string();
        assert!(ModelRegistryEntry::parse(&thin).is_ok());
    }

    #[test]
    fn wrong_or_missing_envelopes_are_refused() {
        for (raw, why) in [
            ("not json".to_string(), "not JSON"),
            ("[1, 2]".to_string(), "not an object"),
            (
                serde_json::json!({ "model_id": "m" }).to_string(),
                "no schema field",
            ),
            (
                serde_json::json!({ "schema": "asterism-model-registry-entry-v2", "model_id": "m" })
                    .to_string(),
                "a schema this instance does not carry",
            ),
            (
                serde_json::json!({ "schema": ENTRY_SCHEMA_V1 }).to_string(),
                "no model_id",
            ),
            (
                serde_json::json!({ "schema": ENTRY_SCHEMA_V1, "model_id": "  " }).to_string(),
                "blank model_id",
            ),
        ] {
            assert!(
                matches!(
                    ModelRegistryEntry::parse(&raw),
                    Err(DomainError::Validation(_))
                ),
                "{why}: {raw}"
            );
        }
    }
}
