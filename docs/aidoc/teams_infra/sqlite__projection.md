# teams-infra::sqlite::projection

Storage for captured projections (#148 decisions 12 and 14).

**Its own module, beside the forge rather than inside it.** Decision
12 puts the projection outside the forge, and where the code lives
is part of how that stays true: nothing here goes through
[`TeamForge`](crate::sqlite::forge::TeamForge), touches a forge
table, or appends to the ledger. What connects the two is that the
push handler calls both, in that order, which is the whole of the
coupling.

## Why the write is not in the forge's transaction

The forge's writes and their ledger events share one transaction
because #83 §2 makes the event the receipt for the write — two
independently writable truths is the one forbidden arrangement, and
same-tx is what forecloses it. A projection is not in that
relationship with anything: decision 12 makes it losable, which is
what makes a separate write correct here rather than merely
tolerable.

[`capture`](SqliteProjectionStore::capture) therefore opens its own
transaction and never appends to the ledger. When it runs relative
to the push is not this file's rule and is argued where it can be
broken — `teams_server::forge::push_round`.

## Nothing here reads the body

It arrives as a
[`ProjectionBody`](teams_core::domain::projection::ProjectionBody),
goes into a `TEXT` column, and comes back out. There is no parse,
no index and no column lifted out of it — decision 14's check
applied to the one file that would be tempted to break it.

## Types

- `SqliteProjectionStore` — The `asset_projection` table, and nothing else.

