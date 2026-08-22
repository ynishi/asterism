# asterism-infra::sqlite::repo::tag_evidence

SQLite adapters for `TagEvidenceRepository` and
`TagVectorRepository` (#112, P3).

The evidence adapter's one load-bearing statement is the
`INSERT OR IGNORE` in [`suggest_if_absent`]: the primary key
`(asset, tag, model)` is what keeps a rerun of the suggestion job
from touching a person's ruling or an earlier score — the guarantee
is structural, not a handler branch.

[`suggest_if_absent`]: SqliteTagEvidenceRepository::suggest_if_absent

## Types

- `SqliteTagEvidenceRepository` — SQLite adapter for `TagEvidenceRepository` (uses a writer isle).
- `SqliteTagVectorRepository` — SQLite adapter for `TagVectorRepository` (uses a writer isle).

