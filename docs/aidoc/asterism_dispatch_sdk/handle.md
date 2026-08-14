# asterism-dispatch-sdk::handle

`Handle` — the exporter's backend-side reference to an in-flight job.

The exporter returns one of these from [`crate::Exporter::dispatch`]
and receives it back on every subsequent [`crate::Exporter::poll`]
and [`crate::Exporter::harvest`] call. The core persists the raw
bytes verbatim (opaque JSON payload); the exporter is free to
interpret them however it needs to reach the same backend job on
process restart.

The `kind` slug lets the exporter fast-fail with a clear error
("this handle was issued by ComfyExporter, not GeminiExporter")
when the caller feeds the wrong handle to the wrong adapter — that
only happens on programmer error, but the failure mode is worth
naming.

## Types

- `Handle` — Opaque reference to one in-flight backend job.

