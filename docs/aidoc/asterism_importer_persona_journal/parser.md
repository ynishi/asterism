# asterism-importer-persona-journal::parser

persona-journal SQLite row -> `Footprint::JournalEntry` parser.

Each row emitted by the `SqliteScanner` corresponds to one
persona-journal `entries` row joined with its current `versions`
body. We route each row to a `JournalEntry` whose `JournalKind` is
chosen from the `entries.kind` column.

Constellation-edge grouping (persona × kind bucket) is carried on
`bundle_id` after the session-model refactor — journal is a
non-Dialog modality, so Session-related fields stay unused.

Legacy note: the original mapping table:

- `"states"`   -> `JournalKind::State`
- `"emo"`      -> `JournalKind::Emo`
- `"non_rem"`  -> `JournalKind::NonRem`
- `"memories"` -> `JournalKind::Memory`
- anything else round-trips via `JournalKind::Other(kind)`.

Locator shape: `{db_path}#{entry_id}` (produced by `SqliteScanner`).
Idempotency relies on the server-side
`asset.source_kind + source_locator` unique constraint.

## Types

- `PersonaJournalParser` — Parser for persona-journal rows.

