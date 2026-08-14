# asterism-core::application_support::retention_service

`RetentionService` — the trash retention sweep.

**Driven by the `trash_purge` job handler, and by nothing else.**
No Tauri command and no HTTP route fronts this: purging on a clock
is a scheduled destruction of the user's data, and the only way to
ask for it is to be the worker whose page-at-a-time, self-chaining
contract bounds it. A user-initiated purge is a different verb with
a different guard — [`AssetService::purge`](crate::application::AssetService::purge)
— which refuses anything not already in the trash, and that one is
transport-fronted precisely because it acts on one row the user
named.

The retention period is injected (from the composition root, which
reads `ASTERISM_TRASH_RETENTION_DAYS`) rather than declared here:
the cutoff is policy, and this layer must not carry a policy
number.

## Types

- `RetentionService` — Retention sweep actuator. Held by `CoreCtx`'s support bundle and
- `Sweep` — One page of the retention sweep, as reported by

