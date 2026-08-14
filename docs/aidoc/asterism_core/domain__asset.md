# asterism-core::domain::asset

`Asset` — an aggregate root for a single footprint, plus the read
projection used on hot paths.

The write path uses [`Asset`] (rich entity with invariants). The read hot
path — grid listing and search — goes through [`AssetCard`] instead, so
the physical row layout can evolve (for example switching to a columnar
representation) without changing the port signature.

## Types

- `Asset` — A single Asterism item (aggregate root; used for writes and detail views).
- `AssetCard` — Lightweight projection used on the read hot path (grid / search).
- `AssetIndex` — Index-only projection for 6-figure grids.
- `AssetQuery` — Filter and pagination parameters for listing and searching assets.
- `ContentFlags` — Coarse content-type hints — mirrors render-session's `HasTable` /
- `TrashFilter` — Which side of the trash a query wants to see.

## Constants

- `UNCLASSIFIED_MODALITY` — Reserved facet key for rows carrying no modality.

