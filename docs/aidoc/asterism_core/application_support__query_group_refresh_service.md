# asterism-core::application_support::query_group_refresh_service

`QueryGroupRefreshService` — bulk Query Group re-evaluation.

**Driven by the `query_group_refresh` job handler and by process
startup, and by nothing else.** No Tauri command and no HTTP route
fronts either sweep: a user never asks for "re-evaluate everything"
— the writes that could invalidate a rule enqueue
[`JobKind::QueryGroupRefresh`](crate::domain::job::JobKind) through
the invalidator (W4), and the
startup pass exists to close the window the V19 migration could not
(no async isle, no Tantivy at raw-connection migrate time). The
transport-fronted verbs are per-group and live on
[`QueryGroupService`]: `create_query_group` and `update_query`, each
of which evaluates the one group it just wrote.

This service is therefore a thin driver over
[`QueryGroupService::evaluate_and_materialize`] rather than a home
for it — that method has three callers on the transport side (both
commands above, plus `DispatchService::run`'s pre-freeze
refresh), so it stays in `application` where they can reach it.
Sweeping *every* group is the part nothing on the wire asks for.

Both sweeps are fail-loud and keep-going: one corrupt rule must not
stop the rest, and every failure comes back in the outcome for the
caller to surface.

## Types

- `QueryGroupRefreshService` — Bulk Query Group refresh driver. Held by `CoreCtx`'s support bundle
- `RefreshAllOutcome` — Result of a [`QueryGroupRefreshService::refresh_all`] or

