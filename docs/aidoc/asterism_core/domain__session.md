# asterism-core::domain::session

`Session` — the Dialog-modality 1st-class aggregate root.

Session was previously "an `asset.session_id` GROUP BY projection"
(the removed `SessionSummary` shape,
[`AssetRepository::list_sessions`](crate::domain::repository::AssetRepository::list_sessions))
with two failure modes. First, importers on non-dialogue modalities
(tape / journal / image) also wrote `session_id`, so the
SessionsView tile grid was flooded with per-file tape rows and
per-kind journal buckets that had nothing to do with a Dialog
session. Second, the projection carried no per-Session metadata,
so the user could not attach a title / note / cover to a run they
cared about.

P1a (this subtask) makes Session a **1st-class entity** stored in
the `session` table with user-editable metadata, scoped to Dialog
modality via a DB CHECK (`asset.session_id IS NULL OR modality =
'dialogue'`, added in V27). The non-dialogue `session_id` values
are moved to a separate `asset.bundle_id` column (V25) that keeps
the constellation-edge grouping intact under its own name — see
[`BundleId`](crate::domain::value::BundleId).

# Metadata

`title` / `note` / `cover_hint` are all `Option<String>` and share
the "user-edited free-form text, default absent" contract Group and
PersonaProfile use. They are grouped into [`SessionMetadata`] so
P1b's HTTP `PATCH …/metadata` handler can carry them as one
payload without leaking through the Session's identity fields.

# Derived aggregates

`started_at_ms` / `ended_at_ms` / `message_count` are aggregates
over the participating `asset` rows and stay on the Session row so
the SessionsView listing can render without a per-row `GROUP BY`.
The `SessionRebuild` job in P1b keeps them in sync; P1a only
initialises them at find-or-create time.

## Types

- `Session` — A Dialog-modality session — the aggregate root that owns a run of
- `SessionMetadata` — Per-Session user-editable annotations. Every field is `None` by
- `SessionMetadataPatch` — Partial-update payload for

