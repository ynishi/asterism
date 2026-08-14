# asterism-core::application::modality_service

`ModalityService` — use cases for the Modality master.

The master is the open half of the two-layer model: rows are listed,
created, partially updated, and deleted at runtime. Behaviour is not
touched here — the service only validates identity + presentation
metadata and the `kind` reference into the closed
[`ContentKind`](crate::domain::value::ContentKind).

Delete is guarded: a slug still carried by any asset is rejected
(`409 Conflict`) so the master can never orphan assets. The
operational retirement path is the `hidden` flag.

Every write here takes an [`AttributionContext`] it does not persist:
the master carries no attribution column, and none is being added
(see the [`application`](crate::application) module doc for why the
argument is required anyway).

## Types

- `ModalityService` — Modality master use-case service. Shared as an `Arc` through Tauri

