# asterism-importer-sdk::harvest

# Agent-harvest intake subsystem

Canonical JSON schema (`asterism_agent_harvest` v1) for
ingesting conversations from closed cloud services that ship no
official export — Character.AI, Kindroid, Replika, Grok, and the
like. Instead of writing a Rust parser per service, the intake
flow is:

```text
(browser scrape / DevTools dump / GDPR export / …)
        │  raw service-specific JSON
        ▼
   Claude Code / Codex agent, prompted with the schema below
        │  writes a one-off Python converter
        ▼
   canonical AgentHarvest JSON file dropped in a landing dir
        │
        ▼
   asterism-importer-harvest binary picks it up and calls
   HarvestSourceParser → Footprint::ChatMessage × N
```

The SDK only sees the canonical schema. Per-service converters are
throwaway Python (or shell, or jq) scripts an LLM writes on demand,
kept alongside the raw dumps by the user.

## Schema (v1)

A prompt-ready example is emitted by
[`schema_example_json`] — the `asterism-importer-harvest
--print-schema` CLI flag pipes it to stdout so a user can paste it
into Claude Code / Codex with "convert my dump to this shape":

```json
{
  "spec": "asterism_agent_harvest",
  "spec_version": "1.0",
  "service": "character.ai",
  "service_user_id": "user-abc123",
  "harvested_at": "2026-07-18T12:34:56Z",
  "characters": [
    {
      "id": "seraphina-xyz",
      "name": "Seraphina",
      "description": "A gentle healer in a forest glade.",
      "avatar_uri": null,
      "extra": {}
    }
  ],
  "conversations": [
    {
      "id": "conv-abc",
      "character_id": "seraphina-xyz",
      "title": "First encounter",
      "started_at": "2026-01-01T00:00:00Z",
      "messages": [
        {
          "id": "m1",
          "role": "user",
          "body": "hello",
          "timestamp": "2026-01-01T00:00:01Z",
          "parent_id": null,
          "extra": {}
        },
        {
          "id": "m2",
          "role": "assistant",
          "body": "welcome, traveler",
          "timestamp": "2026-01-01T00:00:05Z",
          "parent_id": "m1",
          "extra": {}
        }
      ],
      "extra": {}
    }
  ],
  "extra": {}
}
```

## Mapping to Footprints

- `conversations[].messages[]` → [`crate::ChatMessage`] × N
  (`session_id = conversation.id`, `role` via
  [`crate::ChatRole`], `thread_position` = message index,
  `parent_message_id = message.parent_id`, `body = message.body`,
  `occurred_at = message.timestamp || conversation.started_at ||
  envelope.harvested_at || RawItem.occurred_at || Utc::now()`).
- `conversations[].title` / `.started_at` → not emitted directly;
  they seed the shared `session_id` grouping so `edge_rebuild`
  links siblings.
- `characters[]` → not emitted in v1 (character metadata is left
  to a dedicated character-card importer such as
  [`crate::card::CharaSourceParser`] because that route already
  knows about avatars, personalities, and lorebooks). Extend v2
  when a service exposes character metadata in a way no other
  importer covers.
- `service` / `service_user_id` / `spec_version` / envelope
  `extra` → merged verbatim into every emitted footprint's `extra`
  so downstream queries can filter by `service = "character.ai"`
  etc.

See [`crate::catalogue`] section 16 for the catalogue-level entry.

## Functions

- `schema_example_json` — Return the canonical schema example as a `&'static str` (pretty JSON).

