# asterism-importer-sdk::scanner::fs

`FsScanner` — filesystem source scanner.

Walks a directory tree, optionally filtered by glob-ish extension
set, and emits every matching file as a `RawItem`.

The walk is sorted and resumable: a checkpoint behind each file
carries the path it stopped at, and a later scan handed one takes up
after it. The partition is the root *and* the extension filter,
because both decide what the walk yields. In `Watch` mode
the scanner also stays live and streams filesystem-change events via
`notify` — new / modified files are re-emitted, deletions are
ignored (deletions on the source do not automatically delete the
corresponding asset; that is a policy decision left to the caller).

## Types

- `FsScanner` — Filesystem scanner.

