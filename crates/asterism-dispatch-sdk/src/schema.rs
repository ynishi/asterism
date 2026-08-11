//! Schema artifact registry — the SDK-owned half of the "SDK =
//! external contract" story.
//!
//! Every entry here is a `(name, example JSON)` pair. The example
//! JSON is the canonical wire shape an external backend author
//! consumes — LLMs work fine off an example, and the schema files
//! live in `schema/` next to this crate for `cat`-friendly access.
//!
//! Per-exporter parameter schemas are *not* listed here; each
//! exporter crate ships its own `params_example_json()` fn and
//! `SCHEMA_NAME` const, and `asterism-server` walks the exporter
//! registry to enumerate them alongside the SDK-owned ones.
//!
//! Naming convention: SDK-owned schemas use bare snake_case names
//! (`public_asset`, `dispatch_context`, `derived`). Exporter-owned
//! schemas use the `exporter:<slug>:params` form so the two
//! namespaces cannot collide.

/// Canonical example JSON for [`asterism_contract::dto::AssetCardDto`] —
/// the per-asset shape [`crate::DispatchContext::inputs`] carries,
/// what exporter-file emits as sidecar metadata, and what the
/// harvest-style `asset_card.example.json` LLM prompt hands to a
/// backend author.
pub fn asset_card_example_json() -> &'static str {
    include_str!("../schema/asset_card.example.json")
}

/// Canonical example JSON for the dispatch envelope (what
/// exporter-file writes in `instruction` mode and what an
/// HTTP-backed sibling would POST).
pub fn dispatch_context_example_json() -> &'static str {
    include_str!("../schema/dispatch_context.example.json")
}

/// Canonical example JSON for [`crate::Derived`] — the shape
/// exporters return to the core for reification into new Assets.
pub fn derived_example_json() -> &'static str {
    include_str!("../schema/derived.example.json")
}

/// One [`SDK_SCHEMAS`] row: the public schema name paired with the
/// accessor that yields its canonical example JSON.
pub type SdkSchemaEntry = (&'static str, fn() -> &'static str);

/// Every schema this crate publishes, keyed by public name.
///
/// Kept as a `const` slice so consumers can `iter()` at boot time
/// without paying an allocation and without a runtime registry
/// step. The `asset_card` name matches the contract DTO
/// [`asterism_contract::dto::AssetCardDto`] to keep external
/// consumers on the same word the Tauri UI already uses.
pub const SDK_SCHEMAS: &[SdkSchemaEntry] = &[
    ("asset_card", asset_card_example_json),
    ("dispatch_context", dispatch_context_example_json),
    ("derived", derived_example_json),
];

/// One-shot lookup by name against [`SDK_SCHEMAS`].
pub fn find_sdk_schema(name: &str) -> Option<&'static str> {
    SDK_SCHEMAS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, f)| f())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sdk_schema_parses_as_json() {
        for (name, get) in SDK_SCHEMAS {
            let raw = get();
            serde_json::from_str::<serde_json::Value>(raw)
                .unwrap_or_else(|e| panic!("schema {name} is not valid JSON: {e}"));
        }
    }

    #[test]
    fn find_sdk_schema_hits_and_misses() {
        assert!(find_sdk_schema("asset_card").is_some());
        assert!(find_sdk_schema("derived").is_some());
        assert!(find_sdk_schema("no_such_schema").is_none());
    }
}
