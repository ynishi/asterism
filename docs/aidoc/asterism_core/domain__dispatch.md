# asterism-core::domain::dispatch

`DispatchJob` — one exporter invocation against a Snapshot.

Aggregate root for a single outbound job's whole lifecycle
(Pending → Running → Done/Failed/Cancelled). Everything the apalis
`DispatchRun` runner needs to resume after restart is on this row:

- `snapshot_id` — points back at the input Snapshot.
- `exporter_slug` + `action` + `params_json` — how to reach the
  backend.
- `handle_json` — opaque bytes the exporter's `Handle` serialised
  into; the runner rehydrates this to call `poll` / `harvest`
  again.
- `state_slug` — the lifecycle state (mirrors
  `asterism_dispatch_sdk::DispatchState::slug()`).
- `output_asset_ids` — populated during `harvest` so callers can
  go "show me what this dispatch produced" without a follow-up
  query.

# Invariants

1. `persona_id` is the persona the dispatch belongs to; the
   Snapshot carries the same persona (application service
   enforces).
2. `state` transitions are one-way: Pending → Running →
   (Done | Failed | Cancelled). Backwards transitions are rejected
   at the service layer.
3. `output_asset_ids` is empty until state is `Done`; setting it
   is atomic with the transition to `Done` (the reify path writes
   both in one save).

## Types

- `DispatchJob` — One dispatch invocation.
- `DispatchState` — Lifecycle state persisted with the dispatch job.

