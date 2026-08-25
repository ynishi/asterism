# asterism-teams-client::publish

Seeding a team's line from a private one (#148 decision 11).

The other direction from a clone, and it transfers the current state
for the same reason: what a team is being given is something to work
on, not somebody's record of how they got there.

## Two seedings, and only one of them is free

[`Seeding::CurrentState`] is the default and the cheap one. The
team's line gets a genesis and one change point, holding what the
private line holds now.

[`Seeding::Reenactment`] replays the chain — one pursuit and one
close per change point, so the team's line ends with as many change
points as the private one had. **It is chosen at init and nowhere
else**: a line that was seeded with its current state cannot be
given its history afterwards, because the history would have to
arrive underneath change points that already exist.

What it costs is worth saying rather than leaving to be discovered.
A re-enactment sends **every content the line ever named** — the set
[`Line::holds`] describes, which only ever grows and includes
everything an entry was replaced with and everything taken back off
the line. Seeding the current state sends what is on the line now,
which is usually far less.

The cheaper seeding is also the narrower one, in a way that is
nobody's guess: it cannot take a line whose live entries share
content, because a promotion's repeat check is keyed on the asset
and the line, so the second entry would be answered from the first
and the team would receive one where the private line has two.
`seeds` refuses that outright rather than narrowing it silently, and
a re-enactment takes it, because a chain names each entry in its own
right. Both refusals happen before the team's line is opened — see
`seeds` for the ordering and for the one failure it cannot cover.

## Why it is a re-enactment and not a history

The acts are restamped to whoever published. The original actors are
not necessarily members of this team, so there is nobody on the team
plane for the old stamps to name — and inventing one would be the
team's record claiming knowledge it does not have. So the team's
line does not record who did the work upstream, and that is not a
hole: at this boundary the question is who brought this here, and
the restamped act answers exactly that. Who made it before is the
private side's record, which #66 decision 2 says the team never had
a claim on.

The word for that is **re-enactment**, and it is in the type, in
[`Published::reenacted`], and in what the UI says.

## What does not travel at all

Work logs and conversations, and they are not offered at init
either. A pursuit that was abandoned, a round that was pushed and
thought better of, a thread arguing about it — those are the private
deliberation #66 decision 2 protects. Nothing in this module reads
the private line's pursuits: the seeding walks the *line's* chain,
which is what was landed, and the work that produced it is not
reachable from here.

## Functions

- `publish` — Opens a line on the team and seeds it from a private one.

## Types

- `HeldSubject` — One local asset a line names, with what may be said about it.
- `Publication` — A private line, and the team to seed from it.
- `Published` — What a publication left on the team.
- `Seeding` — How much of a private line the team's copy is given.

## Traits

- `Holdings` — What a publication reads out of the local plane.

