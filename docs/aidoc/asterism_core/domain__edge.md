# asterism-core::domain::edge

`ConstellationEdge` — the backbone of the hover-burst experience.

One edge represents an asset-to-asset relationship that surfaces when the
user hovers a card. The `edge_rebuild` job persists edges incrementally,
scoped to a window around each asset (same session id or ±48h) so we
avoid an O(n²) full scan. Are.na-style "same channel" connections are
not stored here — they are derived from the `asset_tag` table on demand.

## Functions

- `dedupe_incident_pairs` — Collapses symmetric `Outgoing` + `Incoming` pairs sharing the same

## Types

- `ConstellationEdge` — An edge connecting two assets.
- `EdgeDirection` — Which side of a [`ConstellationEdge`] a given asset sits on.
- `EdgeKind` — Axis along which an edge is created.
- `IncidentEdge` — An edge as seen from one endpoint's perspective — the pair

