# asterism-server::core_init

Backend initialisation — the whole service graph, assembled once.

Its caller in the product is `asterism-ui`; its other callers are
the end-to-end tests, which want the same graph over a tempdir. Two
things differ between them: the progress emitter (`TauriEmitter` in
the UI, the stderr [`LogEmitter`] elsewhere) and whether a job-worker
`Monitor` is spawned, which is [`JobWorker`]. Everything else lives
here so the ~160 lines of DI wiring are written once.

Callers wrap the returned [`CoreCtx`] into their own context struct
(`ServerCtx` / `AppState`) and add nothing to it. A service assembled
in one wrapper instead of here would be reachable from that
transport alone, which is how the Asset comment thread ended up with
four Tauri commands and no HTTP route.

## Functions

- `init_core` — Initialises the shared backend and returns the assembled [`CoreCtx`].
- `init_core_with` — [`init_core`] with an explicit override for the Tantivy index dir.

## Types

- `CoreCtx` — Shared service graph assembled by [`init_core`].
- `JobWorker` — Whether this process runs the job worker.
- `LogEmitter` — Default [`ProgressEmitter`] for a core with no UI event bus behind

