# teams-infra::sqlite::forge

The team's forge — adapters behind `asterism-core`'s forge ports,
over the team's own database (#148 decision 20).

One type for `Lines`, `Pursuits`, `Closings`, `Threads`, `Actors`
and `Store`. The close is why the first four are one object: it
writes a change point, its rows and an ending together, and two
adapters sharing one transaction is a shape that only reads as
sharing when they are the same object.

`Actors` and `Store` join them for a smaller reason: they need the
same three things this type holds — the isle, the team, and nothing
else — and every question they answer is scoped by the team the way
every other read here is. Neither appends to the stream. `Store` is
a pure read, and minting a handle deliberately records nothing (see
[`TeamForge::handle`]), so what they share with the write ports is
the scope rather than the transaction.

`Strategies` needs no adapter — `Builtin` is in `asterism-core`, and
a collision rule is not storage.

# The team is here and in no signature

[`TeamForge`] holds a `team_id`, every statement carries it, and no
port method mentions it. That is the seat `Lines::list` reserves
when it says that scoping a listing belongs to whoever knows what a
person is: the forge does not, this plane does, and the answer lands
here rather than in a trait the local plane also implements.

Reads are scoped as tightly as writes. A `LineId` from another team
reads back as nothing rather than as somebody else's line, so a
caller holding an id it should not have learns nothing from it.

# Every write is one transaction, and the ledger is in it

Decision 17: a forge write and its ledger event commit together or
neither lands. So every write-port method here is one isle call
opening one transaction, and inside it the rows change *and*
[`append_event_in_tx`] runs — the same allocation of `seq`, the same
registry check and the same subject index rows the repository's own
gestures go through. A half-written pair is not a state this store
can be in, which is what the e2e asks it for.

# Two records, two fields

Revision 6. The **event** records the capacity — `LedgerActor`,
member or admin, with the display name stamped at write time. The
**forge node** records who — an `ActorId`, which is a handle in
`forge_actor` and carries no capacity at all. Neither is derived
from the other, and an event carries the handle in its payload so
the two can be read against each other without a join.

Where the handle is one this team's `forge_actor` already holds,
the event also carries it as a
[`SubjectRef::ForgeIdentity`](teams_core::domain::ledger::SubjectRef::ForgeIdentity),
which is what lets a trace query cross from a person to their forge
writes without reading payloads. A handle with no row is not an
error — the model mints `ActorId`s freely and a caller may carry one
this store never saw — so the subject is added when it can be
resolved and the payload's `by` answers either way.

# What a payload does not carry

Anything somebody wrote. Names of lines and titles of threads are
here, because a name is how a team refers to a line and an event
about a rename that did not say the names would not read on its
own. Message bodies are not, and neither is content: the ledger is
append-only in the schema and there is no path that rewrites a row,
so a body copied into it is a second copy nothing can ever act on
when somebody asks to be erased.

# The head is never read to be compared

Nothing here selects a head and checks it against what a caller
decided: `UNIQUE (line_id, parent_id)` and `UNIQUE (pursuit_id,
parent_id)` refuse a fork as part of the insert. What that costs is
telling one constraint violation from another, so the exact column
list SQLite reports is what is matched — see `is_unique_violation`.

## Types

- `HeldAsset` — One `team_asset` as a caller reading it back sees it — what the
- `TeamForge` — The forge's ports over one team's rows in the teams database.

