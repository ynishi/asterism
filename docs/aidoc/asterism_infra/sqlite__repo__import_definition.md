# asterism-infra::sqlite::repo::import_definition

SQLite adapter for the `ImportDefinitionRepository` port.

Two tables behind one port, for the reason the port states: the
question that spans them — is a run of this definition still going —
is the one that stops a second importer starting over a source the
first is about to move.

`args_json` holds the argument vector as a JSON array. A column per
argument is not a shape a command line has, and a single
space-joined string would have to be re-split by something that
guessed at quoting — the one part of a command line it is genuinely
easy to get wrong, and getting it wrong here means an importer run
with arguments nobody wrote.

## Types

- `SqliteImportDefinitionRepository` — SQLite adapter for `ImportDefinitionRepository` (uses a writer isle).

