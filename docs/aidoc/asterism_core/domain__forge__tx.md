# asterism-core::domain::forge::tx

`PursuitTx` — the pursuit's append-only membership ledger (#22,
model on #63): every asset that enters the line of work, every
mid-work removal, and every reversal, one row per gesture.

The ledger is what makes a cull's "out of what" answerable without
being handed in: the candidate set is **what the pursuit
accumulated**, derived here and frozen at close — never a
caller-supplied snapshot. Mid-work tidying feels like free
manipulation on the surface; underneath, every gesture is a ledger
entry, which is the difference between a workspace and a record.

# Shape

- [`PursuitTx`] is one gesture: `in` (with its [`TxOrigin`]),
  `remove`, `unremove` — and `update`, the model's reserved verb
  for the external-edit round-trip, admitted by the vocabulary but
  written by nothing yet.
- **Membership is derived on read** by [`ledger`]: latest tx per
  asset by `(created_at, id)` wins — `in` / `unremove` mean
  present, `remove` means removed, `update` changes nothing. No
  row is ever edited.
- The asset reference is an id, not a foreign key: the ledger is
  history and history outlives the asset (the
  `dispatch_job.output_asset_ids` stance). The candidate *set*
  survives independently in the snapshot the cull freezes.

## Functions

- `ledger` — Derives the ledger state from a pursuit's gestures: latest tx per

## Types

- `Ledger` — The derived state of a pursuit's ledger: every asset that ever
- `MemberState` — One asset's derived position in a pursuit's ledger.
- `PursuitTx` — One recorded membership gesture.
- `PursuitTxKind` — The closed set of ledger gestures. `In` carries its origin because
- `TxOrigin` — Where an `in` gesture brought its asset from. A fact about the
- `TxTarget` — What an `in` declares about a line entry it is aimed at (#63

