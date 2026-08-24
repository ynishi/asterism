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

## Erasing a person answers in three places

A request to erase somebody is not one deletion, and the reason is
that a person is answered for in three records, each holding a
different part of the answer — a name in one, an association in the
next, a handle in the third. All three have to answer, and an
answer that covers two of them has erased nothing:

1. **The [`ActorStamp`](crate::domain::identity::ActorStamp) on
   every ledger event they wrote.** The name is captured at write
   time precisely so a later rename does not rewrite history — the
   property that makes the ledger readable is the property that
   makes it hold the name.
2. **Their rows in the subject index.** A `user` subject is a uuid
   rather than a name, so what it exposes is the association: which
   events touched this person, which is the question the index
   exists to answer quickly.
3. **`forge_actor` on the local plane.** The handle a member's
   writes resolve to is minted on their own instance, outside this
   plane's storage entirely, and the display snapshot it captures
   is the same captured-not-referenced value as the stamp above.

Three mechanisms can answer, and which one is chosen is a decision
rather than a default: **masking at write** (the stamp records an
id and no name, which costs every reader the ability to say who
without a join, and costs history the name the account had then),
**retention under a documented exemption** (the record is kept and
the basis for keeping it is written down, which is what an audit
log is usually held under), and **crypto-shredding** (the name is
stored encrypted under a per-subject key and erasure destroys the
key, which turns a rewrite into a delete somewhere the append-only
rule does not reach).

**The order matters, and this end of it comes first.** Today a row
could be rewritten under a migration if the decision demanded it —
expensive and against the schema's triggers, but possible. A
tamper-evidence chain removes that: once each entry commits to its
predecessor, rewriting one invalidates every entry after it, and
erasure-by-rewriting is gone permanently rather than merely
discouraged. So the mechanism is settled before a chain starts, not
after — a chain built first would decide this question by making it
unanswerable.

## Functions

- `is_forge_kind` — Whether `kind` is one the hosted forge writes.
- `is_registered_kind` — Whether this build writes `kind` at all — the writer's question,
- `is_v0_kind` — Whether `kind` is one of the substrate's own v0 gestures.

## Types

- `EventKind` — A namespaced + versioned event kind — `"teams.membership.added/1"`.
- `EventSeq` — Storage-assigned position of an event within its team's stream —
- `ForgeIdentityRef` — A forge handle, as a ledger subject: what it stands for, and whom
- `ForgeStandsFor` — What a forge handle stands for — the four kinds #102 fixed for the
- `LedgerEvent` — One entry in a team's stream — the envelope, with the payload
- `SubjectRef` — A typed reference an event makes — the index trace queries walk, so

## Constants

- `BLOB_COPY_COMPLETED` — A promotion's blob copy completed — declared digest verified,
- `BLOB_LINK_PURGE_MARKED` — A team's blob link was marked for purge (#83 §3 lifecycle, the #95
- `BLOB_LINK_PURGE_UNMARKED` — A purge mark was lifted during the grace window — the link is
- `BLOB_LINK_RECLAIMED` — Marked links whose grace window elapsed were reclaimed — the second
- `FORGE_KINDS` — The kinds the hosted forge's verbs write (#148 decisions 17 and
- `FORGE_LINE_DISCARDED` — A line was dropped, with everything on it.
- `FORGE_LINE_OPENED` — A line was opened on the team's forge.
- `FORGE_LINE_RENAMED` — A line was renamed — the payload carries the old name and the new,
- `FORGE_LINE_STANDING_SET` — A line was archived or taken back out of the archive.
- `FORGE_LINE_STRATEGY_SET` — A line's collision rule was changed, old and new both in the
- `FORGE_PURSUIT_CLOSED` — Work ended, and — when it was satisfied — the line moved with it.
- `FORGE_PURSUIT_OPENED` — Work was opened against a line.
- `FORGE_ROUND_PUSHED` — A round was written onto open work.
- `FORGE_THREAD_AMENDED` — Something said was corrected. The correction is a new record and
- `FORGE_THREAD_OPENED` — A conversation was opened about work, a round, an entry as a round
- `FORGE_THREAD_RENAMED` — A conversation was given a title, or had the one it had taken off.
- `FORGE_THREAD_SAID` — Something was said in a conversation.
- `MEMBERSHIP_ADDED` — A user became a member.
- `MEMBERSHIP_REMOVED` — A member left or was removed.
- `ROLE_CHANGED` — A member's role changed — the payload carries **both** the old and
- `TEAM_CREATED` — A team came into existence.
- `TEAM_DELETED` — A team was deleted (owner, or an admin — ledger-stamped).
- `V0_KINDS` — The v0 kind registry: team lifecycle, membership changes, role

