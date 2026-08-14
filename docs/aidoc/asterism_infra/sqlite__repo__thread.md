# asterism-infra::sqlite::repo::thread

SQLite adapter for the `ThreadRepository` port.

Storage shape mirrors the V20 migration in
[`crate::sqlite::migrations`]. Projection columns on `thread`
(`message_count`, `last_message_at`,
`updated_at`) are trigger-maintained; `save` therefore preserves
them across an `ON CONFLICT DO UPDATE` (title / archive edits only
touch the caller-owned columns). `append_message` is idempotent by
`(thread_id, idempotency_key)` — a repeat write returns the
pre-existing row instead of a constraint error.

## Types

- `SqliteThreadRepository` — SQLite adapter for `ThreadRepository`.

