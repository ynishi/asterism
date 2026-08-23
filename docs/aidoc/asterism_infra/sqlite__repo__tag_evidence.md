# asterism-infra::sqlite::repo::tag_evidence

SQLite adapters for `TagEvidenceRepository` and
`TagVectorRepository` (#112, P3).

The evidence adapter's one load-bearing statement is the upsert in
[`suggest_under_head`]: the primary key `(asset, tag, model)` plus
the update's `WHERE disposition = 'suggested' AND head <> excluded`
is what keeps a rerun of the suggestion job from touching a
person's ruling — or a same-head rerun from moving a score — while
letting a *new* head replace an unruled suggestion (#132). The
guarantee is structural, not a handler branch.

[`suggest_under_head`]: SqliteTagEvidenceRepository::suggest_under_head

## Types

- `SqliteTagEvidenceRepository` — SQLite adapter for `TagEvidenceRepository` (uses a writer isle).
- `SqliteTagVectorRepository` — SQLite adapter for `TagVectorRepository` (uses a writer isle).

