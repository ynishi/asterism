# asterism-server::state

Backend context for the HTTP surface. A thin selection over the
shared [`crate::core_init::init_core`]: assembles a [`ServerCtx`]
from the returned `CoreCtx`.

Its callers are `asterism-ui`, which serves this router in its own
process, and the end-to-end tests, which build one over a tempdir
core. There was a third — the `asterism-server serve` subcommand —
and #300 removed it along with the `init` helper that existed only
for it.

The server shares the SQLite file with the Tauri UI process under
WAL; the `busy_timeout = 5000` pragma is applied by `sqlite::open`.
Progress updates go to stderr via `LogEmitter` — there is no UI event
bus in this process.

## Functions

- `default_db_path` — Default DB path: active local data profile (override via

## Types

- `ServerCtx` — Bundle of services that HTTP handlers share via `axum` state.

