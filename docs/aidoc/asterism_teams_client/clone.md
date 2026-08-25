# asterism-teams-client::clone

Taking a copy of what a team holds (#148 decision 10).

Working on a shared line needs no copy — open a pursuit against it,
and decision 16 serves the reads through. Cloning is for when the
copy *is* the point: leaving a team, or starting from what a team
has and going a different way, which the model will not let you do
in place because it refuses a fork outright.

## A clone is an import, not a forge concept

Three things follow from that word, and each is visible in the code
below rather than left to a caller's discipline.

It **mints new ids**, because the local plane mints its own
([`Imports::record`] hands back a local [`AssetId`], and nothing
here reads a [`TeamScopedId`] as one).

It **writes no relation row**. There is no [`AssetLinkRepository`]
in this module's signature at all, which is the strongest way to say
it: a link row means "I put this there", and a copy did not put
anything anywhere.

It **says where it came from through `source_kind` and
`source_locator`**, the way every other import does — and that is
the part with a design decision in it, so [`cloned_locator`] carries
the argument.

## What is not taken

The projection is read and handed back whole, but only its title
lands on the copy, through the one field an ingest has for saying
what something is called. Marks do not: the ingest command has no
slot for one, and [`ReadMark`](crate::ReadMark) is deliberately not
[`PromotedMark`](crate::PromotedMark) — a mark read off the wire has
no layer under it and no verified origin, so it is not a thing this
machine may go on to promote as its own. A caller wanting more than
the title has [`Cloned::projection`] and the whole view in it.

[`AssetLinkRepository`]: asterism_core::domain::repository::AssetLinkRepository

## Functions

- `clone_entry` — Copies one entry of a team's line onto this machine.
- `cloned_locator` — Where a clone of one entry lives, and therefore what it is recorded

## Types

- `Arrival` — What a clone hands the local plane, once the bytes are on disk.
- `CloneRequest` — One entry of one line of one team, to be copied.
- `Cloned` — What a clone left behind, on this machine.

## Traits

- `Imports` — Where a clone puts what it took.

