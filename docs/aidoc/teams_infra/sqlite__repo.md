# teams-infra::sqlite::repo

The teams repository — state tables and the per-team ledger behind
one write rule.

## The same-tx rule is the only write API shape (#83 §2)

Every public state-changing method here opens one transaction,
applies the state change **and** appends the corresponding ledger
event, and commits or rolls back the two together. There is no
public method that writes state without appending, and none that
appends without a state change. The documented exceptions are the
writes outside the ledger's scope, which #83 §2 fixes at the team
boundary: [`SqliteTeamsRepository::record_locator`] (locators are
private-space, which is also why the v0 kind registry has no
locator kind) and
[`SqliteTeamsRepository::publish_head_entry`] (the head registry
is instance-scope — #132 — and carries its history in its own
superseded rows).

## Where the domain runs

Invariants are evaluated by the domain, on current state, *inside*
the transaction that would record the change: the last-owner rule
is [`TeamRoster`]'s answer over the membership rows as they read
under the write lock, and role TEXT goes through [`Role::parse`] in
both directions. This is the deliberate exception to the
read-side convention (promotion outside the closure —
[`map`](crate::sqlite::map)): a check made outside the transaction
would be a check against state the transaction no longer holds.

Domain refusals inside a closure travel as the inner `Result` of a
nested pair — the outer error is SQLite's, the inner is the
domain's, and the transaction rolls back on either.

## What `seq` means here

Storage assigns it: `MAX(seq) + 1` per team, computed inside the
write transaction. The primary key makes it monotonic; the
single-writer deployment shape (#83 §4 — one server, one
connection, `BEGIN IMMEDIATE`) makes it gapless, because a
rolled-back transaction never leaves a hole behind — the next
append recomputes over what actually committed. The domain's
[`EventSeq`] validates what storage hands back and mints nothing.

## Types

- `SqliteTeamsRepository` — SQLite repository for the teams plane — teams, memberships, blob

