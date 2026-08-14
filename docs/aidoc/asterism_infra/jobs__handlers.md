# asterism-infra::jobs::handlers

Pipeline job handlers: `cover_gen`, `auto_tag`, `edge_rebuild`.

Every handler is idempotent and can be re-derived from persisted
state — if an enqueue is lost the missing signal (for example a null
cover) can trigger another run. v1 uses simple heuristics for cover
text and keywords; higher-quality generation via an LLM is planned
for later modalities.

## Functions

- `asset_dims` — Measures `asset.width_px` / `height_px`.
- `asset_fold` — Folds one asset into another — the structural half of resolving a
- `auto_tag` — Extracts keywords, materialises channel tags, and links them to the
- `chapter_scan` — Reads a container's own chapter list into its imported structure
- `cover_gen` — Auto-generates the card cover. Idempotent — if the cover column is
- `duplicate_scan` — Re-derives duplicate conflicts from fingerprints already on the rows.
- `edge_rebuild` — Incrementally rebuilds constellation edges for the target asset.
- `index_rebuild` — Rebuilds the Tantivy full-text index for one asset (single-doc
- `material_hash` — Fingerprints an original's bytes into `material.content_hash`, then
- `observation_sweep` — Expires observations past their stream's declared retention.
- `preview_gen` — Transcodes a webview-unplayable video into its preview rendition
- `query_group_refresh` — Re-evaluates every Query Group under one persona. Payload:
- `series_derive` — Derives `material_series` keys — applies every registered
- `session_rebuild` — Session reconciliation stub — the precomputed rkyv snapshot was
- `thumb_gen` — Generates a resized JPEG thumbnail for a visual asset at a
- `trash_purge` — Retention sweep — purges assets and Groups whose trash stamp has

