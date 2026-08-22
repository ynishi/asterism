# asterism-infra::sqlite::repo::forge_boundary

What the forge asks of everything outside it, answered by SQLite.

Two contracts, and neither is a repository: they are the forge's
side of questions it cannot answer itself.

- [`SqliteStore`] answers whether a persona holds an asset. One
  question, because the forge has one.
- [`SqliteActors`] answers what a handle stands for, and mints one
  when it has not seen the subject before.

They live beside the repositories rather than among them because
what they implement is the boundary, not storage — the forge names
`boundary::Store`, not `AssetRepository`, and that is the whole
arrangement. Putting them in one file keeps the two halves of "what
the forge asks" readable together.

## Types

- `SqliteActors` — Keeps what a forge handle stands for.
- `SqliteStore` — Answers the forge's one question about the layer below.

