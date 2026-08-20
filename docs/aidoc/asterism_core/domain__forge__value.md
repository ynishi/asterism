# asterism-core::domain::forge::value

The forge's surrogate ids.

Split from [`domain::value`](crate::domain::value) so that the raw
layer's id vocabulary contains no forge type. Every id declared here
is named nowhere outside `domain::forge` and `application::forge` —
nothing on the raw side holds one, because a raw export carries no
filing.

## Types

- `LineEntryId` — Surrogate id for a `LineEntry` — the name-like forge identity
- `LineEventId` — Surrogate id for a `LineEvent` — one merge verb applied to an
- `LineId` — Surrogate id for a `Line` — one named line of a project, the
- `MergeId` — Surrogate id for a `Merge` — the record that one satisfied
- `ProjectId` — Surrogate id for a `Project` — the repo of the forge's git
- `PursuitEventId` — Surrogate id for a `PursuitEvent` — one one-way lifecycle fact
- `PursuitId` — Surrogate id for a `Pursuit` — the minted unit of work that
- `PursuitTxId` — Surrogate id for a `PursuitTx` — one entry in a pursuit's

