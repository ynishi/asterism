# asterism-infra::sqlite::repo::forge

SQLite adapter for the forge's ports.

One type for `Lines`, `Pursuits`, `Closings` and `Threads`, where
the rest of this directory is one per port. The close is why: it
writes a change point, its rows and an ending together, and two
adapters sharing one transaction is a shape that only reads as
sharing when they are the same object.

# Where the work is, and where it is not

Taking a domain value apart and putting one back lives in
[`crate::forge::rows`], which the in-memory store uses too. What is
here is SQL and nothing else: the same nine shapes, written as
columns.

# The head is never read to be compared

Nothing here selects a head and checks it against what a caller
decided: `UNIQUE (line_id, parent_id)` and `UNIQUE (pursuit_id,
parent_id)` refuse a fork as part of the insert. The rule is
[`Closings`]' — "on the parent nothing has taken" — and what the
index adds is that the check is the write rather than something
beside it that could be answered from a row somebody else has
since moved.

What that costs is telling one constraint violation from another.
SQLite names the columns rather than the index — `UNIQUE constraint
failed: change_point.line_id, change_point.parent_id` — so that
column list is what is matched, and matched exactly.

# And the one place a log is read to decide something

A close that loses its parent is decided again in here, from a line
and a pursuit read inside the transaction that lost. That read is
not a comparison: nothing is checked against what the caller
decided, and the answer comes from the model rather than from this
adapter. What the transaction contributes is that the logs cannot
move between the read and the write, which is why the second
attempt is the last one.

Which column list is which, and what a substring test would read
out of the wrong one, is on `is_unique_violation`.

## Types

- `SqliteForge` — SQLite adapter for `Lines`, `Pursuits`, `Closings` and `Threads`.

