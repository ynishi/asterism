# asterism-importer-sdk::scanner::sqlite

`SqliteScanner` — SQLite source scanner.

Opens an arbitrary SQLite database read-only, runs a user-supplied
`SELECT` statement, and emits every row as a `RawItem`. Column
mapping is spelled out by the caller so the same scanner can drive
importers over totally unrelated schemas (chat exports, message DBs,
bespoke tools' scratch tables, and so on).

**Resumable only on the caller's word.** The query is the caller's,
so only the caller knows whether it has an order to take up inside;
[`SqliteScanner::ordered_by_id`] is where they say so. Without it
this scanner emits no checkpoints and refuses to resume, rather than
resuming inside an order nobody promised.

Async is faked at the edge: `rusqlite` is blocking, so the scan
actually runs on a dedicated `spawn_blocking` task and pushes rows
into a bounded mpsc.

This scanner leaves
[`payload_is_whole_artefact`](super::SourceScanner::payload_is_whole_artefact)
at its `false` default, and the reason is worth stating rather than
inheriting: a row's `body` column is a value out of a database, and
the `<db>#<id>` address it is given has no bytes of its own for
anybody to read back. A digest declared from here could never be
checked, which is precisely what the server refuses.

## Types

- `ColumnMap` — Column-to-`RawItem` mapping supplied by the importer.
- `SqliteScanner` — SQLite scanner.

