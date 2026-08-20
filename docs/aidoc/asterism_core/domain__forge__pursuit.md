# asterism-core::domain::forge::pursuit

`Pursuit` — the minted unit of work: one line of generation and
curation toward an intent, identified by a minted id that its events
are stamped with (#29, design on #21).

Content ancestry cannot express this unit: round N+1 built from the
same sources with a new prompt shares no derivation edge with round
N's outputs, rejected rounds have no descendants, and abandonment has
no ancestry expression at all. So the identity is minted up front —
the convergent form of every surviving forge (Gerrit's Change-Id,
Jujutsu's change id, Radicle's COB ids) — and everything else about
the pursuit is a projection over the stamped events.

# Shape

- [`Pursuit`] is a thin, immutable row: identity, persona, optional
  filing, optional parent, optional human label. No status column,
  no members.
- [`PursuitEvent`] is a one-way lifecycle fact (close / reopen);
  **standing is derived on read** by [`standing`] — latest event by
  `(created_at, id)` wins, no row means open. A repeat close is a new
  fact, not an error.

# Invariants (service-enforced, entity-checked where local)

- A stamped event's persona equals its pursuit's persona; `parent_id`
  never crosses personas; a parent exists before its child. These are
  cross-aggregate and live in the application service, like the
  persona cascade.
- `project_id` never crosses personas either, and the foreign key
  cannot say so: `project` carries its own `persona_id`, and
  `pursuit.project_id` references only `project(id)`. So filing
  under someone else's project is refused where both rows are
  visible — the same place, and for the same reason, as `parent_id`.
- `snapshot_id` may only accompany `closed_satisfied` (checked here):
  it is the kept set frozen at close. `None` on a `closed_satisfied`
  is a defined state — "concluded with nothing kept" — because an
  empty snapshot is domain-rejected.

## Functions

- `standing` — Derives standing from a pursuit's events: latest by

## Types

- `Pursuit` — The minted unit of work. Thin and immutable: identity plus intent
- `PursuitEvent` — One lifecycle fact about a pursuit.
- `PursuitEventKind` — The closed set of lifecycle facts. One-way: no event edits another,
- `PursuitStanding` — Live standing of a pursuit, derived on read — never stored.

