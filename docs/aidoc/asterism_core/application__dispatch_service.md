# asterism-core::application::dispatch_service

`DispatchService` — the transport-fronted half of the outbound
dispatch lifecycle.

Every verb here is behind a Tauri command, an HTTP route, or both:

- [`create`](DispatchService::create) — start a new dispatch from
  an existing Snapshot, enqueueing an apalis `DispatchRun` job.
- [`run`](DispatchService::run) / [`redispatch`](DispatchService::redispatch)
  — freeze a live source (Group or volatile pick) and start it, or
  re-run a previous freeze unchanged.
- [`get`](DispatchService::get) / [`list`](DispatchService::list)
  — read the persisted state (used by the UI polling endpoint and
  an MCP tool).

Each of the three start verbs takes an
[`AttributionContext`](crate::domain::attribution::AttributionContext)
and stamps it on the job row, because the run outlives its caller:
the exporter is polled by a background job, and the moment the answer
is needed (stamping the reified outputs) is minutes or hours after
the request that supplied it.

What the runner does to a dispatch in flight — `save_state` /
`save_handle` / `reify` — is not here. Those live on
[`DispatchRunnerService`](crate::application_support::DispatchRunnerService),
reachable from the `DispatchRun` job's environment and from no
transport context, so no handler can park a job in `Done` or mint
assets from a wire payload.

## Types

- `DispatchService` — Outbound-dispatch use-case service.

