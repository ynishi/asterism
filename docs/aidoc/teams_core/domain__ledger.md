# teams-core::domain::ledger

`ledger` — the actor-stamped, append-only event envelope (#83 §2).

Each team has one stream. The substrate knows the envelope and
nothing inside it: `payload` is a versioned body *per kind* and
stays opaque here, `subjects` is the typed index trace queries walk
instead of parsing payloads, and `kind` is a namespaced + versioned
string so `forge.*` kinds can register after #63 with the envelope
unchanged.

Two things this module deliberately does **not** do:

- **Generate `seq`.** Monotonicity within a team is a storage
  guarantee (one SQLite tx, single writer by deployment shape), so
  [`EventSeq`] is a newtype the domain validates and carries but
  never mints — a domain-side counter would be a second writable
  truth, the one forbidden shape.
- **Source state from events.** State tables are authoritative and
  every state change appends its event in the same tx (audit-log
  pattern, not event sourcing — #83 §2 SoT note). Nothing here
  replays.

## Functions

- `is_v0_kind` — Whether `kind` is one this build of the plane writes. Shape and

## Types

- `EventKind` — A namespaced + versioned event kind — `"teams.membership.added/1"`.
- `EventSeq` — Storage-assigned position of an event within its team's stream —
- `LedgerEvent` — One entry in a team's stream — the envelope, with the payload
- `SubjectRef` — A typed reference an event makes — the index trace queries walk, so

## Constants

- `BLOB_COPY_COMPLETED` — A promotion's blob copy completed — declared digest verified,
- `MEMBERSHIP_ADDED` — A user became a member.
- `MEMBERSHIP_REMOVED` — A member left or was removed.
- `ROLE_CHANGED` — A member's role changed — the payload carries **both** the old and
- `TEAM_CREATED` — A team came into existence.
- `TEAM_DELETED` — A team was deleted (owner, or the operator — ledger-stamped).
- `V0_KINDS` — The v0 kind registry: team lifecycle, membership changes, role

