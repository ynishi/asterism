# asterism-infra::sqlite::repo::import_definition

SQLite adapter for the `ImportDefinitionRepository` port.

Two tables behind one port, for the reason the port states: a
definition and its runs are read and written together, so splitting
them would make one act reach through two ports.

It does not answer whether a run is going. That question belongs to
the process holding the children — see the service — and the only
thing left of it here is [`abandon_running`], a sweep over rows a
previous process left open.

[`abandon_running`]: asterism_core::domain::repository::ImportDefinitionRepository::abandon_running

`args_json` holds the argument vector as a JSON array. A column per
argument is not a shape a command line has, and a single
space-joined string would have to be re-split by something that
guessed at quoting — the one part of a command line it is genuinely
easy to get wrong, and getting it wrong here means an importer run
with arguments nobody wrote.

## Types

- `SqliteImportDefinitionRepository` — SQLite adapter for `ImportDefinitionRepository` (uses a writer isle).

