# asterism-server::state

Backend context for the standalone server. Thin wrapper over the
shared [`crate::core_init::init_core`] (`ReadOnly` mode): assembles a
[`ServerCtx`] from the returned `CoreCtx`.

The server shares the SQLite file with the Tauri UI process under
WAL; the `busy_timeout = 5000` pragma is applied by `sqlite::open`.
Progress updates go to stderr via `LogEmitter` — there is no UI event
bus in this process.

## Functions

- `default_db_path` — Default DB path: active local data profile (override via
- `init` — Initialises the backend in read-only mode and returns the shared

## Types

- `ServerCtx` — Bundle of services that HTTP handlers share via `axum` state.

