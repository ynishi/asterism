# asterism-core::domain::forge::cull

`Cull` — the record of one close's narrowing (#22, model on #63):
who decided to keep or drop what, out of which frozen candidate
set, in which line of work.

The cull is a **close-time record**, not a mid-work gate. Mid-work
tidying moves through the ledger
([`tx`](super::tx)); the cull converts the final state into
verdicts at the one moment they become statements — just before
the close that lands them. One cull per close event; a repeat
close is a new event and may carry a new cull.

# Verdict rules (resolved by [`resolve_verdicts`])

- A verdict names a **candidate** — an asset the ledger admitted.
  Judging what never entered is refused.
- A **newly entered** member (`generated` / `imported`) takes
  `keep` or `reject`.
- An **existing** member takes `reject` only: keeping what the
  library already holds is the untouched default, not a statement.
  The one exception is salvage — a `keep` on a *removed* existing
  member cancels the removal's default and is recorded.
- A member **removed** in the ledger and not spoken for culls as
  `reject` — the default is materialised as a row, because the
  cull is the record and a reader must not have to re-derive it
  from the ledger.
- An **untouched** member without a verdict gets no row: "this act
  said nothing about it" is the absence, deliberately (#63 — no
  forced verdict; the unprocessed remainder just stays).

## Functions

- `resolve_verdicts` — Resolves a caller's verdicts against the ledger into the rows the

## Types

- `Cull` — One act of narrowing, bound to the close event it happened at.
- `CullMember` — One member's verdict within a cull.
- `CullVerdict` — The closed set of member verdicts. Two values, no third —
- `RequestedVerdict` — A caller's requested verdict, before resolution against the
- `ResolvedVerdict` — A resolved verdict, ready to become a `cull_member` row.

