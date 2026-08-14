# asterism-core::domain::color

`ColorBucket` — the closed set of colours the palette facet filters
on.

Assets already carry a five-entry dominant-colour palette (extracted
from the 128 px thumbnail by the `thumb_gen` job). Those are exact
hex values, which is the wrong shape for a filter: no two photos
share a hex, so "show me the red ones" cannot be answered by
equality, and answering it by distance would mean scanning every
palette on every query and exposing a threshold knob nobody wants to
tune.

So the palette is quantised once, at extraction time, into this
closed set. A bucket is indexable (the sidebar swatch is an equality
predicate), countable (the facet shows how many assets carry each
colour, like the FORMAT section next to it), and stable (the same
hex always lands in the same bucket, so the derived
`asset_color` rows can be rebuilt from `asset.palette` at any time).

Buckets are a *view* of the palette, never the source of truth —
`asset.palette` stays canonical, `asset_color` is a projection.

## Functions

- `bucket_of` — Quantises one palette entry (`#rrggbb`, leading `#` optional, case
- `buckets_of` — Quantises a whole palette, de-duplicated, in [`ColorBucket::ALL`]

## Types

- `ColorBucket` — A quantised palette colour — one sidebar swatch.

