# asterism-core::application::thread_service

`ThreadService` — application-layer surface for `Thread` and its
`Message` children.

Verbs:
- [`list`](ThreadService::list) — Threads under a given anchor.
- [`create`](ThreadService::create) — new Thread with a title +
  anchor.
- [`archive`](ThreadService::archive) — toggle the `archived`
  flag (soft hide).
- [`delete`](ThreadService::delete) — hard delete (cascades to
  messages).
- [`list_messages`](ThreadService::list_messages) — chronological
  read, optional `since` cursor for polling.
- [`append_message`](ThreadService::append_message) — append a
  Message. Same verb for UI (`author = human`) and HTTP
  (`author = claude_code` / `agent` / `persona`). Idempotency
  key deduplicates safely.
- [`delete_message`](ThreadService::delete_message) — remove one
  Message (misfire correction; there is no edit verb).

Anchor validation: `snapshot` and `query_group` anchors are
id-only in P1 — the service parses the id and lets the adapter
surface a foreign-key error on write. `Card` (asset) anchors are
reserved for a later phase (P3); the service still
accepts them today because the anchor parser does.

Every write here takes an [`AttributionContext`] it does not persist.
A Message already names its own writer through
[`thread::Author`](crate::domain::thread::Author), which is a
different factorisation of the same question (attribution splits it
into author × operator; `thread::Author` folds human and agent into
one enum). Unifying the two is deliberately left to a later
wave — this argument neither feeds nor overrides
`author_kind`.

## Types

- `ThreadService` — Application-layer surface for the Thread primitive.

