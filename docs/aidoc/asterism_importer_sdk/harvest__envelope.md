# asterism-importer-sdk::harvest::envelope

Typed schema for `asterism_agent_harvest` v1 JSON dumps.

Every field is either strictly required or has a serde default so a
minimal well-formed dump only needs `spec`, `spec_version`,
`service`, and at least one `conversations[].messages[]` entry.

## Types

- `HarvestCharacter` — Optional character metadata (persona-side info the harvester
- `HarvestConversation` — One conversation. All messages share the conversation's `id` as
- `HarvestEnvelope` — Top-level envelope for one harvested batch (one file = one dump).
- `HarvestMessage` — One message inside a conversation. Maps to

## Constants

- `HARVEST_SPEC` — Canonical `spec` string every dump must carry. The parser rejects
- `HARVEST_SPEC_VERSION` — Current `spec_version`. Bumped on breaking schema changes.

