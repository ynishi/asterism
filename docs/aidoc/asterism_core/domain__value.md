# asterism-core::domain::value

Value objects: id / slug / text newtypes plus `Visibility`, `SourceRef`,
`Progress`, and `Page<T>`.

Design notes:

- **Surrogate ids are UUID v7.** Natural keys (for example `pack_id` from
  an external persona pack) live in separate fields with unique
  constraints.
- **`Modality` and `SourceKind` are open slugs.** Asterism is a
  general-purpose grid product; adding a new consumer must be a data
  change, not a breaking enum change. Well-known slugs are exposed as
  associated constants.
- **`Visibility` powers the enforcement of visibility filters.** The
  decision function (`visible_to`) lives here; SQL translation is the
  adapter's job in `asterism-infra`.

## Functions

- `dedup_labels` — Drops repeats from a label list, keeping the **first** occurrence of

## Types

- `AssetCommentId` — Surrogate id for an `AssetComment` — one entry in an Asset's
- `AssetId` — Surrogate id for `Asset`.
- `AssetRole` — Structural role of an asset: a curatable item carrying its own
- `AudioFormat` — The `audio/*` formats, plus anything else that arrived as `audio/*`.
- `BundleId` — Open grouping key used by the constellation-edge builder for
- `ChannelAxis` — Classification axis for a `Tag` (channel). Optional in v1 — tags may be
- `ChapterMarkId` — Surrogate id for a `ChapterMark` — one entry in a structure
- `CoverTemplate` — The template `cover_gen` applies to derive card cover text. A
- `CoverText` — Text shown on the grid card for an asset (produced by the CoverGen
- `CullId` — Surrogate id for a `Cull` — the record of one close's
- `DirId` — Surrogate id for `Dir` (sidebar organisation folder).
- `DispatchId` — Surrogate id for a `DispatchJob` — one exporter invocation
- `DuplicateConflictId` — Surrogate id for a `DuplicateConflict` — one raised "are these
- `EdgeId` — Surrogate id for `ConstellationEdge`.
- `ExternalSessionKey` — The raw session identifier an importer hands in — for example
- `FoldPolicy` — Whether an asset may be folded into another, or has been ruled a
- `GroupId` — Surrogate id for `Group` (user-curated set of assets).
- `ImageFormat` — The `image/*` formats [`guess_mime`](crate::domain::material::guess_mime)
- `InstanceId` — Surrogate id for this Asterism instance — the single
- `JobId` — Surrogate id for `Job`.
- `Keyword` — Raw keyword extracted by the auto-tag pipeline; a `Tag` may be
- `Label` — Free-form annotation attached to an asset.
- `MaterialLayerId` — Surrogate id for a `MaterialLayer` — one band of marks over an
- `MaterialMarkId` — Surrogate id for a `MaterialMark` — one mark in an Asset's
- `MediaKind` — Media-render variant selected by a [`ContentKind`].
- `MessageId` — Surrogate id for a `Message` — one entry appended to a
- `MimeType` — What an asset's bytes are, parsed once at the mapping boundary.
- `Modality` — Primary modality slug for an asset (open slug).
- `OnDuplicate` — What should happen if this asset turns out to hold bytes another
- `PackId` — Natural key from an external persona pack (unique when present).
- `Page` — A paginated result set.
- `PersonaId` — Surrogate id for `Persona`. The natural key (external pack id) is
- `PreviewMode` — Preview mode selected by a [`ContentKind`] for the QuickLook overlay.
- `Progress` — Job progress payload; the `ProgressEmitter` forwards it to the UI.
- `PursuitEventId` — Surrogate id for a `PursuitEvent` — one one-way lifecycle fact
- `PursuitId` — Surrogate id for a `Pursuit` — the minted unit of work that
- `PursuitRestampId` — Surrogate id for a `PursuitRestamp` — one recorded move of a
- `PursuitTxId` — Surrogate id for a `PursuitTx` — one entry in a pursuit's
- `RegisterNote` — Short annotation about the asset's register / tone; the presentation
- `SessionId` — Session identifier attached to a dialogue asset — after the
- `SnapshotId` — Surrogate id for a `Snapshot` — the immutable, content-addressed
- `SourceKind` — Ingest source slug for an asset (open slug).
- `SourceRef` — Reference to the real source of truth for an asset.
- `StrategyId` — Surrogate id for a
- `TagId` — Surrogate id for `Tag`.
- `ThreadId` — Surrogate id for a `Thread` — the app-level container that
- `VideoFormat` — The `video/*` formats, plus anything else that arrived as `video/*`.
- `Viewer` — Subject requesting a view of an asset (used to enforce visibility).
- `Visibility` — Visibility of an asset.

