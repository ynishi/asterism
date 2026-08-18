# asterism-dispatch-sdk::exporter

`Exporter` — the trait every outbound adapter implements.

An exporter is stateless from the SDK's point of view: the core
passes it a [`DispatchContext`] on every method and stores anything
the exporter wants to remember about the in-flight job as an opaque
[`Handle`]. The three methods split the lifecycle so backends with
very different rhythms (single-shot HTTP, long-poll, watchdog on a
filesystem drop dir) all fit the same shape:

1. [`Exporter::dispatch`] — send the job to the backend. Returns a
   `Handle` the core persists so subsequent calls survive restart.
2. [`Exporter::poll`] — check the backend for progress. Called by
   the apalis `DispatchRun` job on a re-enqueue loop until the
   state is terminal.
3. [`Exporter::harvest`] — once `poll` returns `Done`, ask the
   exporter to collect the produced artefacts and describe them as
   [`Derived`]s. The core reifies each Derived into a new `Asset`
   with `parent_ids` = the Selection's inputs, `session_id` =
   `DispatchJob.id`, and `source_kind` = `format!("dispatch:{}",
   exporter_slug)`.

All three take the same [`DispatchContext`], and it carries one
outbound channel beside the read-only slices:
[`attempt`](DispatchContext::attempt), where the exporter records
what a call sent and what came back. That record survives an error
return, which is what makes a refused submit as readable as an
accepted one.

## Types

- `DispatchContext` — The Selection-plus-context bundle the core hands the exporter on
- `ExporterError` — Errors returned by an exporter. Everything not covered by a

## Traits

- `Exporter` — The single trait every outbound adapter implements.

