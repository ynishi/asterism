# asterism-infra::sqlite::repo::forge

SQLite adapter for the forge's ports.

One type for `Lines`, `Pursuits` and `Closings`, where the rest of
this directory is one per port. The close is why: it writes a change
point, its rows and an ending together, and two adapters sharing one
transaction is a shape that only reads as sharing when they are the
same object.

# Where the work is, and where it is not

Taking a domain value apart and putting one back lives in
[`crate::forge::rows`], which the in-memory store uses too. What is
here is SQL and nothing else: the same six shapes, written as
columns.

# The head is never read to be compared

Nothing here selects a head and checks it against what a caller
decided. Two nodes on one parent is a fork, `UNIQUE (line_id,
parent_id)` and `UNIQUE (work_id, parent_id)` refuse one, and the
refusal arrives as part of the insert — so the validation is the
write rather than something beside it that could be answered from a
row somebody else has since moved.

What that costs is telling one constraint violation from another.
SQLite names the columns rather than the index — `UNIQUE constraint
failed: change_point.line_id, change_point.parent_id` — so that
column list is what is matched, exactly rather than by substring:
`work_node.work_id` is the second ending, and it is a prefix of
`work_node.work_id, work_node.parent_id`, which is a fork. Reading
one as the other would tell a caller to re-decide something that
cannot be re-decided.

## Types

- `SqliteForge` — SQLite adapter for `Lines`, `Pursuits` and `Closings`.

