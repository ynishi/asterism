# asterism-core::application::query_group_invalidation

Query Group invalidation — the W4 hook that translates a
user-facing `AssetService` write into a coarse per-persona refresh.

# Shape

[`AssetService`] holds one [`QueryGroupInvalidator`]. Every write
method that changes a field a Query Group rule could read
(asset add / delete, tag attach / detach, manual group mutation)
calls [`QueryGroupInvalidator::notify_persona`] with the persona
id it touched. The invalidator debounces a burst — many writes to
the same persona in a short window collapse into a single job —
and then enqueues one [`JobKind::QueryGroupRefresh`] whose handler
reruns every Query Group in that persona.

# Why debounce

Bulk operations (`attach_tag_batch`, `add_memo`, big
`bulk_move_modality` rounds) fire dozens of writes back-to-back. A
naïve "enqueue per write" would flood apalis with duplicate jobs
and re-run the same evaluation over and over. The debounce is
per-persona: writes to persona P schedule a refresh for `now +
DEBOUNCE_MS`; a second write to P within the window rearms the
timer instead of enqueuing again. `now + DEBOUNCE_MS` chosen to be
long enough to swallow a bulk loop but short enough that a manual
edit's refresh feels immediate (200 ms).

# Job-loop safety

The refresh handler's own write
(`SqliteQueryGroupRepository::replace_membership`) never re-enters
[`AssetService`], so the "refresh must not re-fire itself"
invariant "job-derived writes are excluded from the hook"
holds structurally at this hook site — no per-call
opt-out is needed.

# Job-chain rule inputs (W4-a)

Background jobs also write fields Query Group rules read —
`handlers::auto_tag` links tags (the `tag_ids` filter dimension)
and `index_rebuild` refreshes Tantivy (the `search_text`
dimension). W4-a wires the same invalidator into those handlers
through the late-bound `JobDeps::query_group_invalidator` cell, so
their writes refresh memberships without waiting for the next
user-facing write. Deliberately out: `cover_gen` (its
`content_flags` / `cover` writes have no filter dimension in the
query contract — a refresh would be a no-op re-evaluation) and
`session_rebuild` (persona-less payload; the session projection
is not a rule input).

## Types

- `QueryGroupInvalidator` — Per-persona debouncing enqueuer for
- `QueryGroupRefreshPayload` — Wire payload for [`JobKind::QueryGroupRefresh`].

## Constants

- `DEBOUNCE_MS` — Coalescing window: writes to the same persona within this window

