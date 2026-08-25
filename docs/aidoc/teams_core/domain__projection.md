# teams-core::domain::projection

The captured projection — descriptive metadata, keyed by entry and
opaque all the way down (#148 decisions 12, 13 and 14).

## Outside the forge, on purpose

The forge has three axes and #102 forbids a column that answers
what the history already answers, so a description is not a fourth
axis and does not go on a change point. It lives beside the forge
on this plane, keyed `(line, entry)`. The consequence the design
actually wants from that: **a projection can be lost without the
line lying.** Everything else here follows from it — including why
the write does not share the forge's transaction, and why nothing
appends to the ledger when one is captured.

## Captured, not owned

It is what the promoter said at the time, on the same discipline as
an `ActorStamp` capturing a display name at write time. The team
does not edit it. Only a forge op replaces one — which is why the
write rides on the round push (#148 decision 19) rather than
getting a verb of its own, so no second editing surface grows
beside the verbs.

## Opaque, and this module is where that is kept honest

Decision 14 gives the test: *if a port signature, a column, or a
DTO ever names something inside the body, this decision has been
broken.* [`ProjectionBody`] exists so that the test is easy to
apply here — it is a newtype over a string that nothing on this
plane parses, whose one accessor hands back the whole of it, and
which has no `serde_json` anywhere near it. This plane stores it,
hands it back, and never learns what is in it.

## Types

- `EntryProjection` — One entry's captured projection.
- `ProjectionBody` — A description, as bytes this plane does not read.

