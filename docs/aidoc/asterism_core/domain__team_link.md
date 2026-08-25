# asterism-core::domain::team_link

The relation between a local Asset and what a team made of it
(#148 decisions 6, 8 and 9).

## It lives here and nowhere else

A promotion hands an Asset over and the team converts it into
something of its own. Neither side holds a reference to the other's
object: the server holds no reference to a local Asset, in either
direction, and this module is the whole of what the member's
machine keeps about the correspondence.

Its key is `(team_id, line_id, entry_id)` — all three fixed by the
client rather than learned back from the server, which is the shape
offline-first sync converges on. Within one member and one team the
relation is 1:1 by construction; across teams one Asset has as many
rows as teams; across members one team entry has a row on each
machine that promoted it. There is no global 1:1 and nothing needs
one — each row is a weak reference to its own team, and reading the
row for the team you are looking at is the whole of it.

## Advisory, and attended

Either end can vanish independently and neither may break the
other (#148 decision 9). So there is no foreign key under any of
the four ids, and the pair of verbs that make "advisory" different
from "unattended" are
[`AssetLinkRepository::dangling_locally`](crate::domain::repository::AssetLinkRepository::dangling_locally)
and
[`AssetLinkRepository::reap`](crate::domain::repository::AssetLinkRepository::reap).
Why no key rather than one of the two SQLite offers is argued from
the storage side, in the V104 migration.

## A clone writes no row

Only a promotion does. A cloned Asset is a detached copy and says
where it came from through `source_kind` / `source_locator` the way
every other import does (#148 decision 10) — which is also why a
row here means "I put this there" and never merely "I have seen
this".

## Types

- `AssetLink` — One promotion, recorded at home.
- `AssetLinkKey` — What identifies one promotion: the team, the line, and the entry on
- `TeamScopedId` — An id another plane minted, held here as an opaque handle.

