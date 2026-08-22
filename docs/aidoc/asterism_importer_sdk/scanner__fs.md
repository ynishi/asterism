# asterism-importer-sdk::scanner::fs

`FsScanner` — filesystem source scanner.

Walks a directory tree, optionally filtered by glob-ish extension
set, and emits every matching file as a `RawItem`. In `Watch` mode
the scanner also stays live and streams filesystem-change events via
`notify` — new / modified files are re-emitted, deletions are
ignored (deletions on the source do not automatically delete the
corresponding asset; that is a policy decision left to the caller).

## Types

- `FsScanner` — Filesystem scanner.

