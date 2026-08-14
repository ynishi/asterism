# asterism-core::application_support::dispatch_runner_service

`DispatchRunnerService` — the runner-side half of the outbound
dispatch lifecycle.

**Driven by the `DispatchRun` job (`asterism_infra::dispatch::runtime`),
and by nothing else.** No Tauri command and no HTTP route fronts
these three verbs, and none should: they are the state machine's
own transitions. A handler that could call
[`save_state`](DispatchRunnerService::save_state) could park a
dispatch in `Done` without an exporter ever having run, and one
that could call [`reify`](DispatchRunnerService::reify) could mint
assets from a payload the wire supplied rather than from what the
exporter actually produced. What the transports *do* front lives on
[`DispatchService`](crate::application::DispatchService): create /
run / redispatch (start one) and get / list (read one).

- [`save_state`](DispatchRunnerService::save_state) /
  [`save_handle`](DispatchRunnerService::save_handle) — runner-side
  updates during the `dispatch → poll` loop. The handle is
  persisted immediately after `Exporter::dispatch` returns so a
  restart mid-poll rehydrates the exact same reference.
- [`reify`](DispatchRunnerService::reify) — turn the
  `Vec<Derived>` the exporter produced into new `Asset` rows whose
  `parent_ids` point at the Snapshot's members via
  `ConstellationEdge { kind: DerivedFrom }`, and ask for each new
  row's bytes to be fingerprinted.

`reify` is the boundary where the SDK's `Derived` shape crosses
back into the domain — the only place in the workspace that speaks
both dialects.

It is also the one write path that takes no `AttributionContext`
from its caller (the *restore* class). The caller is the
job runtime in `asterism-infra`, which could only ever assert one;
the honest answer was recorded on the dispatch row when the request
arrived, so this service reads it back
([`DispatchJob::persisted_attribution`]) and carries it onto the
assets it mints.

## Types

- `DispatchRunnerService` — Runner-side dispatch service. Held by `CoreCtx`'s support bundle

