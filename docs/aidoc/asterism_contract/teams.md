# asterism-contract::teams

The team plane's shapes, as they cross this app's own boundary.

A topic module for the same reason [`forge`](crate::forge) is one:
the team plane answers to a model of its own, and a reader of
[`TeamLedgerPageDto`] needs the types beside it rather than the
asset DTOs two hundred lines up.

# Why these exist here as well as on the wire

Two boundaries, not one. `asterism-teams-wire` carries what a
member's client and a team server say to each other over HTTP; this
carries what the desktop app says to its own frontend. They meet at
one place — the Tauri command — and the shapes look alike there
because the same facts cross both.

Alike is not the same as shared. This crate imports no other
Asterism crate, which is what keeps it a leaf and what stops a
dependency cycle; and the frontend has one vocabulary rather than
two, which is what `bindings.ts` being a projection of *this* crate
means. A command that returned a wire type would hand the second
vocabulary to every screen that called it.

So the duplication is the boundary, and the mapping lives in the
command that crosses it.

# What the ledger's shape carries

An append-only record of what a team did, in what capacity. The
read is paged over `seq` rather than whole, because the forge
writes a row per push and a table that grew by the occasional
membership gesture does not any more.

Two properties of it decide how a screen may read it, and both are
stated on the fields they belong to: a null cursor is not an end,
and an actor's display name is a snapshot rather than a lookup.

## Types

- `TeamLedgerEventDto` — One act, as the ledger recorded it.
- `TeamLedgerPageDto` — One page of a team's ledger, oldest first.
- `TeamSubjectRefDto` — One typed reference an act makes.

