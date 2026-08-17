# asterism-core::domain::forge::mainline

`Mainline` — a project's canonical set, and the forge identity that
makes "the living one" a derivable fact (#63 decisions 1–3).

Raw asset ids are one-off: a replacement is a different row, and
nothing in the core says the new row *is* the old thing, newer. The
entry is that statement — a minted identity above asset ids — and
the four merge verbs (add / replace / delete / rename) are the only
things that ever move it. Liveness, current name, and current
version all derive on read from the verb sequence, exactly like
`PursuitStanding`: latest event per entry wins, and history is the
sequence itself, not a second record of it.

# Shape

- [`Mainline`] is one named line of a project. v1 mints exactly one
  per project, named [`Mainline::MAIN`] (application-enforced); the
  row exists so "mainline" is a branch rather than a hard-coded
  place, and the schema admits siblings before the code does (the
  V82 admit-ahead stance).
- [`MainlineEntry`] is the identity; it carries no name column —
  the current name is the latest naming verb's, so renames are
  history like everything else.
- [`MainlineEvent`] is one verb applied to one entry.
  [`MainlineVerb`] carries each verb's payload so a caller cannot
  file an add without a name or a delete with an asset (the
  `RestampSubject` stance); storage enforces the same pairing with
  two-way CHECKs.
- [`Merge`] is the record that one satisfied close applied its
  verbs — approval *is* the merge event, so every event names the
  merge it landed under, and who approved derives through the
  close event's attribution rather than being copied here.

# The boundary, restated

The mainline *references* asset ids; it never annotates or mutates
an asset (the PR #62 rule). A dead entry's asset row stays live and
restorable — `delete` is a statement about the canonical set, not
about bytes, the same distance `CullVerdict::Reject` keeps from
trash.

# Invariants (service-enforced, entity-checked where local)

- Verb payload pairing and non-blank names are checked here.
- Living-name uniqueness within a mainline is an application rule
  checked at merge time — dead names are reusable, so it cannot be
  a schema constraint.
- An entry's first event is an `add`; later events land on an
  existing entry. The write path (P3) enforces this; on read the
  derive tolerates a dangling tail by answering `None` on the axes
  the missing `add` would have filled. That tolerance is *weaker*
  than the ledger's, which drops a dangling gesture's asset from
  membership outright (`tx.rs`) — deliberately so: an event row
  names an entry that exists, so deriving its presence states
  nothing false, where a membership would.

## Functions

- `entry_state` — Derives one entry's state from its events: latest by

## Types

- `EntryState` — One entry's derived position: alive or dead, and what it currently
- `Mainline` — One named line of a project. v1 restricts a project to exactly one,
- `MainlineEntry` — The forge identity above raw asset ids (#63 decision 1). Deliberately
- `MainlineEvent` — One verb applied to one entry, filed under the merge that landed it.
- `MainlineVerb` — The closed set of merge verbs, payload included (#63 decision 2).
- `Merge` — The record that one satisfied close applied its verbs (#63 decision

