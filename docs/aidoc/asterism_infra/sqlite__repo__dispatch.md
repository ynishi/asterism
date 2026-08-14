# asterism-infra::sqlite::repo::dispatch

SQLite adapter for the `DispatchRepository` port.

One row per dispatch job — the enum-carrying `DispatchState` is
stored as a slug column plus three payload columns
(`state_message`, `progress_current`, `progress_total`) that
together round-trip the variants without a JSON-tagged shape (the
wire DTO does the same split).

## Types

- `SqliteDispatchRepository` — SQLite adapter for `DispatchRepository`.

