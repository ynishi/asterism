# asterism-teams-client 0.0.0

# asterism-teams-client — the member's half of a shared line

A team hosts a forge and holds what its members brought (#148
decisions 1 and 2). This is what a member's machine runs to talk to
one: a session, the reads over a shared line, the forge's verbs,
and the one composite act that is the point of the whole issue —
[`promotion`], handing an Asset over so the team converts it into
something of its own.

## Where the boundary is

`asterism-* -> teams-*` is forbidden in any form (#83 §4), so this
crate reaches the wire through two doors and no others:
`asterism-teams-wire`, the leaf both planes depend on and which depends on
neither, and `asterism-contract::forge`, whose DTOs were already
MIT/Apache and already read on both sides. What follows from that
is worth stating plainly: **this crate never links the team plane's
code**, only its vocabulary. Everything it knows about a team it
learned over HTTP.

## The three things that do not cross

- **Ids.** Each plane mints its own surrogates (#148 decision 6). A
  team's id for anything arrives here as a
  [`TeamScopedId`](asterism_core::domain::team_link::TeamScopedId),
  which is a different type from `AssetId` with no conversion in
  either direction, so the forbidden read does not compile.
  Subjects and digests do cross, and they are the two the issue
  says may.
- **Anything re-derivable.** A promotion sends the material and the
  marks a person wrote, and nothing else (#148 decision 4).
  Thumbnails, indexed bodies and `Imported`/`Machine` marks stay
  home because the receiving side can make them again.
- **Anything undeclared.** What may travel in a projection is
  declared at [`mapper`], and a local field nobody has declared
  does not leave (#148 decision 13). A column added to the Asset
  next year starts out staying home.

## What is not here

**The clone** (#148 decision 10) — it is #153's, and it is an
import rather than a forge concept: it mints new ids, writes no
relation row, and says where it came from through `source_kind` the
way every other import does.

**A local copy of a shared line.** Decision 16 serves shared lines
through rather than mirroring them, so every read here is a request
and there is no staleness to reason about. Wanting it locally is
what a clone is for.

## Modules

- [`client`](client.md): Talking to a team server (#148 decisions 16 and 19).
- [`link`](link.md): Verify and reap — what makes the relation *attended* rather than
- [`mapper`](mapper.md): The one mapper — what may travel, and the declaration that decides
- [`promotion`](promotion.md): Handing an Asset over (#148 decisions 3, 4, 5, 7 and 8).

