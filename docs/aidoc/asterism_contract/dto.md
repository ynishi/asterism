# asterism-contract::dto

Response DTOs — outputs shared between the application services, the
TypeScript bindings, and any future MCP tool responses.

Conversion from domain types happens in `asterism-core`
(`application::mapping`); this crate is a leaf and knows nothing about
the domain types.

## Types

- `AssetCardDto` — Lightweight card representation used on the grid (wire form of
- `AssetCommentDto` — One comment attached to an Asset.
- `AssetCountEntryDto` — One row of a sidebar count aggregation — `(key, asset_count)`.
- `AssetDetailDto` — Composite response for the detail view (asset + tags + edges).
- `AssetDto` — Full asset payload used on the detail view.
- `AssetIndexEntryDto` — Index-only wire form for 6-figure grids.
- `AssetIndexPageDto` — Paginated index page (sibling of `AssetPageDto`).
- `AssetPageDto` — Paginated grid page.
- `AssetTextDto` — Full source text of one asset, resolved from the original
- `ChapterMarkDto` — One named section of a material — an entry in a chapter list.
- `ConstellationItemDto` — One hover-burst item — an edge paired with the card it lands on.
- `DerivedDto` — One thing an exporter produced, ready for the core to reify
- `DiagDto` — One persisted diagnostic (`GET /asterism/diag`).
- `DirDto` — A sidebar organisation folder. Dirs contain dirs and groups —
- `DispatchDto` — One exporter invocation against a frozen [`SnapshotDto`].
- `DuplicateAxis` — Which fingerprint a duplicate finding is about.
- `DuplicateConflictDto` — One unanswered "are these two the same thing?" question.
- `DuplicateGroupDto` — One set of live assets that share a fingerprint on one axis.
- `DuplicateReportDto` — The duplicate report (`GET /asterism/duplicates`).
- `DuplicateResolutionDto` — What one answered question ended up saying.
- `EdgeDto` — A single constellation edge (payload for the hover-burst).
- `EventDto` — One telemetry event row (wire form of the local `event_log`
- `GroupDto` — A user-curated Group (bucket) — the hand-picked twin of a Tag.
- `GroupLinkDto` — One Group-in-Group connection (Are.na channel-in-channel). The
- `GroupSummaryDto` — A group paired with the number of distinct assets attached, used
- `JobDto` — Job status payload.
- `JobKindSnapshotDto` — Per-kind slice of the background-jobs table (wire form of the
- `JobLogDto` — One job run (`GET /asterism/jobs/log`).
- `JobsSnapshotDto` — Snapshot of the background-jobs table used by the UI progress
- `LineDto` — One named line of a project (#63 decisions 1–2) — the branch of the
- `LineageEdgeDto` — One `derived_from` link in a lineage walk. Direction is
- `LineageNodeDto` — One asset in a multi-hop lineage walk.
- `LineageViewDto` — Multi-hop `derived_from` lineage around one asset.
- `MaterialLayerDto` — One band of marks over an Asset's material — which reading of the
- `MaterialLayerViewDto` — One band together with the chapters in it — what an asset-level read
- `MaterialMarkDto` — One mark placed into an Asset's material — the coordinate space its
- `MergeAssetsDto` — What the manual merge verb saw and did — the answer to
- `MergeRefusalDto` — One row a fold could not touch, and why.
- `MergeTotalsDto` — Row-count totals of a merge, a field-by-field port of
- `MergeWarningDto` — One rule that would have declined an *automatic* fold of a pair, if
- `MessageDto` — One appended entry in a Thread.
- `MessageRefDto` — One reference chip embedded in a `MessageDto` body.
- `ModalityDefDto` — One row of the Modality master (`GET /asterism/modalities`).
- `ObservationDto` — The envelope every observation carries, whatever stream it is in.
- `PerfDto` — One persisted timing (`GET /asterism/perf`).
- `PersonaDto` — Persona payload.
- `PersonaProfileDto` — Per-persona identity signal — the avatar / short bio / role
- `PersonaThemeDto` — Per-persona visual chrome — currently holds the wallpaper asset
- `ProjectDto` — The repo of the forge's git analogy (#63 decisions 1–2): the shared
- `ProvenanceViewDto` — Composite response for `GET /asterism/assets/{id}/provenance` —
- `PursuitDto` — The minted unit of work (#29): one line of generation and curation
- `PursuitEventDto` — One lifecycle fact about a pursuit (#29): a close or a reopen,
- `PursuitTxDto` — One gesture in a pursuit's append-only membership ledger (#22):
- `PursuitViewDto` — One pursuit, opened up (#29): the thin row plus everything the
- `RetrievedIdsDto` — A retrieval reduced to **order**: the ranked ids and nothing else.
- `RetrievedPageDto` — One page of a **retrieval** — the ranked shortlist, narrowed by the
- `SampledPageDto` — A random handful drawn from the set a filter describes — the answer to
- `SavedQueryDto` — A named `(filter, sort)` snapshot pinned in the sidebar next to
- `SeriesStrategyDto` — One registered series Strategy — a rule for reading "made the same
- `SessionDto` — One `session` row — the Dialog-modality 1st-class entity.
- `SessionPageDto` — Paginated response for the Sessions view. Items are
- `SettingDto` — One application setting, resolved through the whole layer stack
- `SettingLayerDto` — One layer contributing a value to a setting.
- `SnapshotDto` — An immutable, content-addressed freeze of an ordered asset set —
- `TagCountDto` — A tag paired with the number of assets currently attached to it,
- `TagDto` — A single channel tag.
- `ThreadAnchorDto` — Thread anchor — what a `ThreadDto` hangs off of.
- `ThreadDto` — Thread container payload.
- `VideoPreviewDto` — Where a video's preview rendition stands

## Constants

- `DERIVED_COVER_MAX_CHARS` — Maximum cover hint length in Unicode scalar values (matches
- `DERIVED_REGISTER_MAX_CHARS` — Maximum register-note preview length (matches

