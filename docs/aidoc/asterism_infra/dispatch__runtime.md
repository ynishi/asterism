# asterism-infra::dispatch::runtime

`DispatchRun` handler + `ExporterRegistry`.

The handler is intentionally boring: fetch the job row → look up
the matching Exporter → advance one step of the state machine →
persist → optionally re-enqueue. All Exporter interaction goes
through the SDK trait so this file has no knowledge of any
specific backend.

## Functions

- `run_dispatch_run` — Executes one step of the dispatch state machine.

## Types

- `DispatchRunEnv` — Bundle of dependencies the runner needs on every tick.
- `ExporterRegistry` — Registry of exporters keyed by their `Exporter::slug()`, plus any
- `QueueReEnqueue` — [`ReEnqueue`] impl that pushes another `DispatchRun` job through

## Traits

- `ReEnqueue` — Small port around "put this dispatch id back on the queue for

