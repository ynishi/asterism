# asterism-core::domain::duplicate_conflict

`DuplicateConflict` — one open question of the form "these two rows
hold the same bytes; are they the same thing?".

A fingerprint landing on bytes another asset already holds is a
fact, and [`EdgeKind::IdenticalTo`](crate::domain::edge::EdgeKind::IdenticalTo)
is where that fact is recorded. This is the other half: the *question*
the fact raises, parked where a person can answer it later
The two are deliberately separate —
the edge outlives every answer, the question stops being asked once
it has one.

# Why a table and not the edge

The edge cannot carry the queue. It is written on all three
strategies, including the two that ask nothing (`fold` acts
immediately, `separate` records and moves on), so "there is an
`identical_to` edge" is not the same statement as "somebody still has
to look at this". Deriving the queue from edges would also make a
resolution unrecordable: closing a question by deleting the edge
would destroy the byte-level fact, which is the one thing a `keep`
ruling explicitly preserves.

# The pair, not the event

A row here is keyed by the **unordered pair** ([`pair_key`]), even
though the fields remember which side arrived last. Detection is an
event and has a direction; the question is about the pair and does
not. Which row happens to be fingerprinted first depends on whether
the bytes arrived through an import or through the backfill walk, and
keying on that would put the same pair on the queue twice — once from
each end — for a user to answer twice.

[`pair_key`]: DuplicateConflict::pair_key

## Types

- `ConflictResolution` — How an open question was answered.
- `DuplicateAxis` — Which fingerprint the two rows agreed on.
- `DuplicateConflict` — One raised — and possibly answered — duplicate question.
- `FoldExclusion` — Why a pair that a lane asked to fold was put on the queue instead.

