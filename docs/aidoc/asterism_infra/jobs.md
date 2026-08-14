# asterism-infra::jobs

Job engine — apalis with the `apalis-sql` SQLite backend.

## Contents

- [`AsterismJob`] — the queue payload. A single typed queue holds every
  kind of job; the kind is carried in a slug field.
- [`SqliteJobQueue`] — implementation of the `JobQueue` port from
  `asterism-core`.
- [`start`] — runs `SqliteStorage::setup` and spawns the worker
  `Monitor` on a tokio task.

## Coexistence with `rusqlite-isle`

`apalis-sql` reaches SQLite through `sqlx 0.8` (which uses
`libsqlite3-sys 0.30`). The workspace pins `rusqlite-isle` to the
release line built against the same cluster, so both stacks link
against a single copy of `sqlite3`. Job persistence tables (`Jobs`,
and so on) are created by `SqliteStorage::setup` inside the same DB
file as the domain tables — they are owned by apalis, and the domain
never mirrors them.

## Handlers

[`handlers`] holds the per-kind implementations: `cover_gen`
(modality-specific heuristic), `auto_tag` (keyword extraction + tag
materialisation), and `edge_rebuild` (windowed incremental rebuild via
`plan_edges`). `asset_add`, `persona_import`, and `index_rebuild` are
future work.

## Functions

- `jobs_depth` — Reads [`JobsDepth`] off the apalis `Jobs` table.
- `jobs_snapshot` — Returns a compact snapshot of the apalis `Jobs` table. Groups by
- `open_job_pool` — Opens the sqlx pool used by the job engine. Provided as a helper so
- `open_queue` — Opens the job queue: runs `SqliteStorage::setup`, creates the
- `start` — Starts the job engine: opens the queue ([`open_queue`]) and spawns a
- `start_workers` — Spawns the worker `Monitor` for an already-opened queue on a tokio

## Types

- `AsterismJob` — Queue payload — the job-kind slug (see [`JobKind::as_str`]) plus a
- `JobDeps` — Dependencies passed in by the caller (`asterism-ui` /
- `JobEnv` — Execution environment handed to worker handlers via apalis' `Data`
- `JobKindSnapshot` — Per-kind slice of the apalis `Jobs` table.
- `JobsDepth` — Queue depth by status — the poll-cheap half of [`jobs_snapshot`].
- `JobsSnapshot` — Snapshot of the apalis `Jobs` table used by the UI progress banner.
- `SqliteJobQueue` — Implementation of `JobQueue` on top of apalis' `SqliteStorage`.
- `SqlitePool` — Re-export the sqlx-side pool type so downstream crates (server /

