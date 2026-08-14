# asterism-infra::telemetry

Local telemetry — append-only `action_log` access (dogfooding
metrics).

`action_log` is the ActionLog stream of the observability domain: what
the *person* did. Its siblings (`job_log`, `diag_log`, `perf_log`)
answer different questions and are written elsewhere; see
[`asterism_core::domain::observation`].

Operational tier like [`crate::jobs::jobs_snapshot`]: no domain
aggregate, no repository port. The struct wraps the writer isle and
exposes exactly two operations — `record` (append) and `list`
(newest-first read) — consumed by the Tauri commands and the HTTP
API. Rows are local-only by design; nothing ever leaves the
machine.

Contract types are used directly on the boundary (`asterism-infra`
already depends on `asterism-contract` for the dispatch runtime) so
the two transports share one mapping instead of duplicating it.

## Types

- `Telemetry` — Append/read handle for the `action_log` table.

