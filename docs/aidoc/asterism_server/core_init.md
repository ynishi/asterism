# asterism-server::core_init

Shared backend initialisation for both the Tauri UI and the standalone
server.

The two processes assemble the exact same service graph. The only
differences are (1) the progress emitter (`TauriEmitter` in the UI, the
stderr [`LogEmitter`] in the server), (2) whether the Tantivy index is
opened read-write or read-only, and (3) whether a job-worker `Monitor`
is spawned. Those three axes are captured by [`CoreMode`]; everything
else lives here so the ~160 lines of DI wiring are written once.

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
- `CoreMode` — Selects how the shared core is opened for the calling process.
- `LogEmitter` — Default [`ProgressEmitter`] for processes without a UI event bus (the

