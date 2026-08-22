# asterism-dispatch-sdk::schema

Schema artifact registry — the SDK-owned half of the "SDK =
external contract" story.

Every entry here is a `(name, example JSON)` pair. The example
JSON is the canonical wire shape an external backend author
consumes — LLMs work fine off an example, and the schema files
live in `schema/` next to this crate for `cat`-friendly access.

Per-exporter parameter schemas are *not* listed here; each
exporter crate ships its own `params_example_json()` fn and
`SCHEMA_NAME` const, and `asterism-server` walks the exporter
registry to enumerate them alongside the SDK-owned ones.

Naming convention: SDK-owned schemas use bare snake_case names
(`public_asset`, `dispatch_context`, `derived`). Exporter-owned
schemas use the `exporter:<slug>:params` form so the two
namespaces cannot collide.

## Functions

- `asset_card_example_json` — Canonical example JSON for [`asterism_contract::dto::AssetCardDto`] —
- `derived_example_json` — Canonical example JSON for [`crate::Derived`] — the shape
- `dispatch_context_example_json` — Canonical example JSON for the dispatch envelope (what
- `find_sdk_schema` — One-shot lookup by name against [`SDK_SCHEMAS`].

## Types

- `SdkSchemaEntry` — One [`SDK_SCHEMAS`] row: the public schema name paired with the

## Constants

- `SDK_SCHEMAS` — Every schema this crate publishes, keyed by public name.

