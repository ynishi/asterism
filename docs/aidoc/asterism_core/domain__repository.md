# asterism-core::domain::repository

Repository ports — the persistence traits declared here and implemented
in `asterism-infra` (dependency inversion: trait declarations belong to
the consuming crate).

Every trait is `Send + Sync` because Tauri v2 uses a multi-threaded
tokio runtime. Hot-path list / search methods return the `AssetCard`
projection instead of full `Asset` entities.

## Types

- `Candidate` — One candidate from [`AssetRetriever::retrieve`] — an asset, its
- `ChapterScanCandidate` — One material no chapter reading has reached yet — the unit the
- `DimsCandidate` — One row of a dimension-measuring pass.
- `DimsProbe` — What reading an artefact's bytes produced.
- `DimsScope` — Which rows a measuring pass is about.
- `DimsWritePolicy` — What a measuring pass may do to a row that already has an answer.
- `DuplicateGroup` — A set of live assets that share one content fingerprint.
- `Evidence` — Why one asset came back as a candidate.
- `FingerprintedMaterial` — One material whose fingerprints are already written — the unit the
- `FoldOutcome` — What one call to [`AssetRepository::fold_into`] did.
- `FoldRefusal` — Why a fold wrote nothing.
- `FoldReport` — Counts from a fold that went through. Every number is a row count,
- `IndexDoc` — One asset as handed to the retrieval index.
- `LayerScope` — The `(asset, material, role)` triple a layer lookup is scoped by.
- `MaterialFingerprint` — The pair of values one fingerprint pass produces for one material.
- `MergeOutcome` — What one call to [`AssetRepository::merge_into`] did — or, on a dry
- `QueryGroupRow` — One query group as listed by
- `RegisteredStrategy` — One `series_strategy` row: the rule, and what the row says about
- `RetrievalIntent` — What the caller is looking for, in the Retrieval domain's own terms.
- `RetrievalQuery` — One Retrieval request.
- `Retrieved` — The answer to one [`RetrievalQuery`] — a ranked shortlist.
- `SourceLookupScope` — Which rows count as holding a Source value, for
- `TextLocator` — A locator established to point at text.
- `UnderivedSeries` — One `(material, rule)` pair nothing has answered yet — the unit the
- `UnhashedMaterial` — One material still waiting for its fingerprints — the unit the

## Traits

- `AppSettingRepository` — Persistence port for stored setting overrides (`app_setting` table).
- `AssetBodyRepository` — Persistence port for the full-text search **body cache** — the
- `AssetCommentRepository` — Persistence port for [`AssetComment`] — the per-Asset thread of
- `AssetIndexer` — Ingest side of retrieval — keeps the index in step with the assets.
- `AssetRepository` — Persistence port for [`Asset`], including the read projection.
- `AssetRetriever` — Retrieval port — "find me something like this", answered as a
- `ChapterMarkRepository` — Persistence port for [`ChapterMark`] — the sections a structure
- `DirRepository` — Persistence port for [`Dir`], the sidebar organisation tree.
- `DispatchRepository` — Persistence port for [`DispatchJob`].
- `EdgeRepository` — Persistence port for [`ConstellationEdge`].
- `GroupRepository` — Persistence port for [`Group`] and its many-to-many link with
- `InstanceRepository` — Persistence port for the instance identity record (`instance`
- `JobQueue` — Port for enqueueing background jobs. The adapter wraps `apalis`
- `MaterialLayerRepository` — Persistence port for [`MaterialLayer`] — the bands of marks over an
- `MaterialMarkRepository` — Persistence port for [`MaterialMark`] — the marks placed into an
- `ModalityRepository` — Persistence port for the [`ModalityDef`] master (`modality` table).
- `PersonaProfileRepository` — Persistence port for [`PersonaProfile`]. Kept apart from
- `PersonaRepository` — Persistence port for [`Persona`].
- `PersonaThemeRepository` — Persistence port for [`PersonaTheme`]. The theme is a 1:1 side
- `ProgressEmitter` — Port for pushing job progress to the UI. In Tauri, the adapter emits
- `QueryGroupRepository` — Persistence port for the Query Group evaluation core.
- `SeriesRepository` — Persistence port for the series axis — the [`Strategy`] rules
- `SessionRepository` — Persistence port for the [`Session`] entity — the Dialog-modality
- `SnapshotRepository` — Persistence port for [`Snapshot`] — the immutable content-addressed
- `SourceTextReader` — Port for reading the full text of an asset's **original source**
- `TagRepository` — Persistence port for [`Tag`] and its many-to-many link with assets.
- `ThreadRepository` — Persistence port for [`Thread`] and its [`Message`] children —
- `ThumbRepository` — Persistence port for the pre-generated thumbnail cache

## Constants

- `RETRIEVAL_K_CEILING` — Ceiling on [`RetrievalQuery::k`], honoured by every retriever.

