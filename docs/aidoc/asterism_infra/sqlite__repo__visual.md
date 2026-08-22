# asterism-infra::sqlite::repo::visual

SQLite adapter for the `VisualFeatureRepository` port (#112).

Vectors are stored as little-endian `f32` blobs beside their full
derivation identity; a row exists only once extraction has answered
(`computed` with a vector, `failed` with a reason), so the walk's
`NOT EXISTS` predicate offers each image material exactly once per
model. `preprocess_ver` sits outside the primary key on purpose: a
recipe bump *overwrites* the same model's row rather than growing a
second generation, and reads filter on it so a stale-recipe vector
is never served as current.

## Types

- `SqliteVisualFeatureRepository` — SQLite adapter for `VisualFeatureRepository` (uses a writer isle).

