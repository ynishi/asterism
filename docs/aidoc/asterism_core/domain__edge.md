# asterism-core::domain::edge

`ConstellationEdge` — the backbone of the hover-burst experience.

One edge represents an asset-to-asset relationship that surfaces when the
user hovers a card. Are.na-style "same channel" connections are not stored
here — they are derived from the `asset_tag` table on demand.

Several jobs persist edges and they do not agree about scope, which is a
property of the questions rather than an inconsistency: `edge_rebuild`
works a window around each asset (same session id or ±48h) because "these
arrived together" is a claim about a window, while the visual and
near-duplicate rebuilds scan the whole persona because a copy of a picture
can arrive years after the original. What keeps the second kind affordable
is that it compares stored values rather than re-reading anything, and
what keeps the three apart is [`EdgeKind::is_synth`] and the disjoint
scopes beside it.

## Functions

- `dedupe_incident_pairs` — Collapses symmetric `Outgoing` + `Incoming` pairs sharing the same

## Types

- `ConstellationEdge` — An edge connecting two assets.
- `EdgeDirection` — Which side of a [`ConstellationEdge`] a given asset sits on.
- `EdgeKind` — Axis along which an edge is created.
- `IncidentEdge` — An edge as seen from one endpoint's perspective — the pair

