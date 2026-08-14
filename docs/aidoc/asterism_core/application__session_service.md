# asterism-core::application::session_service

`SessionService` — use cases for the Session 1st-class entity.

P1b + P2 scope: `get` (single-Session lookup for the detail path),
`find_or_create_by_external_key` (the importer's idempotent
re-entry point), plus the P2 CRUD trio `rename` /
`patch_metadata` / `delete_if_empty` (backing the HTTP CRUD
surface). The write methods return the resolved
[`SessionDto`] so the caller can echo the persisted state back
on the wire without a follow-up `get`.

`list_by_persona` was removed: the SessionsView list path goes
through `AssetService::list_sessions`, so the service method had no
caller on either transport. The repository port of the same name
survives it (`SessionRepository::list_by_persona`).

Shared as an `Arc` through Tauri state and server contexts, same
shape as [`ModalityService`](crate::application::ModalityService).

Every write here takes an [`AttributionContext`] it does not persist:
the `session` table carries no attribution column, and none is being
added (see the [`application`](crate::application) module doc for why
the argument is required anyway).

## Types

- `SessionService` — Session use-case service. Shared as an `Arc` through Tauri

