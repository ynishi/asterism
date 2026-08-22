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
parent_id)` and `UNIQUE (pursuit_id, parent_id)` refuse one, and the
refusal arrives as part of the insert — so the validation is the
write rather than something beside it that could be answered from a
row somebody else has since moved.

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

`contains` is what exactness is guarding against, and the direction
matters: `pursuit_node.pursuit_id` is the second ending and is a *prefix*
of `pursuit_node.pursuit_id, pursuit_node.parent_id`, which is a fork. So a
substring test asked about the ending matches the fork — it reads
"somebody pushed a round first" as "this work has already ended",
and an ending is final where a fork is decided again. So the close
that a second decision would have landed is refused instead, and
the caller is told the work is over when it is only one round
further along. The other direction cannot happen, which is why
naming it would be naming the wrong risk.

## Types

- `SqliteForge` — SQLite adapter for `Lines`, `Pursuits` and `Closings`.

