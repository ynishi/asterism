# asterism-infra::sqlite::repo::session

SQLite adapter for the [`SessionRepository`] port.

session-model v2 (shape updated by asset-model v4): a Session is a
**composite Asset** (`asset` row with `role = 'collection'`,
`modality` NULL), not a row in the legacy `session`
table. Its members are the assets pointing at it via
`asset.container_id`. This adapter therefore reads/writes the
`asset` table and projects composite rows into the [`Session`]
domain shape, deriving the aggregates
(`message_count` / `started_at_ms` / `ended_at_ms`) at query time
from the members — so there is no cached-count drift (the class of
bug the old stored `session.message_count` column produced).

Metadata mapping composite Asset ↔ Session: `title` ↔ `title`,
`register_note` ↔ `note`, `cover` ↔ `cover_hint`, `external_key` ↔
`external_key`. The composite's own `occurred_at` seeds the time
window when it has no members yet.

The delete guard (`delete_if_empty`) is implemented server-side
(SQLite `COUNT` over `container_id` inside the same isle closure
that issues the composite DELETE) so a race between an
`AssetRepository::save` writing `container_id = <id>` and a caller
trying to delete the composite cannot slip an orphan through — both
writes hit the same writer isle serially.

`create` is single-valued by the same mechanism. It is a
find-or-create — the lookup on `(persona_id, external_key)` and the
composite `INSERT` run inside **one** isle closure, so no second
writer can interleave between them. Until V62 the atomicity was
borrowed from a UNIQUE index instead: `create` inserted blindly, read
the violation back out of the error text, and the caller re-queried.
That index is gone — `external_key` is a Prop, an external record
legitimately arrives twice, and ids from two platforms collide — so
the serialisation point had to become explicit. It always was the thing
doing the work; the constraint was only where it happened to live.

## Types

- `SqliteSessionRepository` — SQLite adapter for [`SessionRepository`] (uses a writer isle).

