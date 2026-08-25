# asterism-teams-client::client

Talking to a team server (#148 decisions 16 and 19).

## Served through, never mirrored

Decision 16: reads and writes go to the server, there is no local
copy of a shared line, and therefore no staleness to reason about.
So every method here is a request. There is no cache, no `sync`,
and no "refresh" — and there should not be one, because a mirror
would be the weaker version of a clone with a cache attached, and a
clone is #153's.

## The shape of the surface

Decision 19: the transport is the local forge's verbs mirrored
under `/teams/{team_id}/forge/*`, plus a content verb scoped to a
pursuit, a bulk resolve and a have-check.

**This crate implements a subset of that, and the subset is the
design.** #152 is a promotion and the reads a member needs around
one, so the line and pursuit verbs a promotion walks are here and
the conversation verbs are not — nothing in this issue says
anything in a thread. What every path here does promise is to be
the router's verbatim: a path spelled differently on the two sides
is a bug in one of them, and that is the claim worth checking.

## Ids that come back are handles

Every id a team states arrives as a
[`TeamScopedId`](asterism_core::domain::team_link::TeamScopedId),
which has no conversion to or from a local `AssetId` in either
direction (#148 decision 6). What crosses in the other direction is
a subject and a digest, which is what the decision says may.

## Types

- `TeamsClient` — A client bound to one team server, holding at most one session.
- `TeamsClientError` — What can go wrong between here and a team.

