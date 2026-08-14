# asterism-core::application::asset_service

`AssetService` — asset lifecycle, grid reads, and detail views.

- Write path: [`add`](AssetService::add) validates and persists the
  asset, then enqueues the follow-up pipeline jobs (`cover_gen`,
  `auto_tag`, `edge_rebuild`).
- Read hot path: [`list`](AssetService::list) and
  [`search`](AssetService::search) pass the `AssetCard` projection
  straight through without materialising the full entity.
- Detail path: visibility is enforced here (through
  [`Visibility::visible_to`]) — the list path enforces it via SQL.

There is no separate `SearchService` in v1: [`search`](AssetService::search)
only orchestrates the `AssetRetriever` port (Tantivy) against the
repository's filter surface, which is small enough to sit here. When
semantic search lands, this decision can be revisited.

## Types

- `AssetService` — Asset use-case service. Shared as an `Arc` through Tauri state and
- `OriginalFileRef` — Where one asset's original artefact is, and what shape its bytes

## Constants

- `INBOX_LABEL` — Well-known label slug the Inbox / Review flow keys off. Assets

