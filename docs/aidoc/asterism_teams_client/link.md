# asterism-teams-client::link

Verify and reap — what makes the relation *attended* rather than
merely advisory (#148 decision 9).

Either end of a promotion can vanish independently and neither may
break the other, so nothing in the schema stops a link row from
outliving what it points at. Decision 9 names the two systems that
live with exactly that and go looking anyway: GitLab's loose
foreign keys, which are a missing constraint plus a worker that
cleans up after one, and `git annex fsck`, which tolerates a
dangling location log and checks for it.

## Two ends, two questions, one of which needs the network

- **The Asset was deleted.** Answerable here, with no session and
  no server: `AssetLinkRepository::dangling_locally` is an
  anti-join against the local `asset` table.
- **The entry is gone from the team.** Not answerable here at all.
  [`verify`] asks the team — two reads per line the relation names,
  for the reason its own doc gives — and a line that no longer
  exists answers `404`, which is what a discard leaves behind.

## What is *not* dangling

A **trashed** Asset. The local plane can still restore it, so the
row still corresponds to something.

An entry that was **removed from the line**. Taking an entry off a
line is a change point saying so, and the fold still lists it —
nothing in a forge truly disappears except through a discard. A row
pointing at a removed entry points at something the team can still
account for, and reaping it would throw away the record of a
promotion that did happen.

An entry named by a round of **work that is still open**. It has
not landed on the line yet and it is not lost either; see
[`verify`] for why that costs a second read.

## Reap

[`reap`] is a thin pass to `AssetLinkRepository::reap`, whose doc
states what a reap may touch. Nothing is added on the way through,
and that is the whole of this module's part in it.

## Functions

- `reap` — Removes the named link rows, and nothing else. Answers how many
- `verify` — Checks every row this machine holds for one team, both ends.

## Types

- `DanglingLink` — One row that points at something that is not there.
- `LinkVerification` — What a verify found.
- `Missing` — Which end of a link went missing.

