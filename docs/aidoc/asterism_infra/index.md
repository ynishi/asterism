# asterism-infra 0.0.0

# asterism-infra — outbound adapters (SQLite, filesystem, job engine)

## Architecture

Concrete implementations of the ports declared in `asterism-core`
(repository traits, `JobQueue`, `ProgressEmitter`, and so on). The
dependency direction is one-way — `asterism-core` never depends on
this crate.

- **SQLite backend** — built on `rusqlite-isle`, which confines the
  `Connection` to a dedicated thread and gives us `sqlite3_interrupt`-
  based cancellation, a WAL reader pool, and panic containment. The
  `SqlitePersonaRepository`, `SqliteAssetRepository`, `SqliteTagRepository`,
  and `SqliteEdgeRepository` all live here.
- **Filesystem / media pipeline** (planned) — originals stay on the
  filesystem and are served through Tauri's asset protocol, while
  thumbnails will be cached as SQLite BLOBs. `walkdir`, `notify`,
  `image`, `rayon`, and `kamadak-exif` are wired for future use.
- **Job engine** — apalis + apalis-sql (SQLite backend) coexists with
  `rusqlite-isle` on the same SQLite file. The engine owns the job
  lifecycle; adapters here expose it to the domain through the
  `JobQueue` port.
- **Artefact probes** — one implementation of
  `asterism_core::domain::probe::ArtefactProbe` per container format,
  plus the registry `fingerprint` asks. Each holds a format's
  judgement about its own bytes (which of them are the picture, which
  are notes about it); the byte-level parsing underneath is
  `asterism-media-probe`, which has no such judgement.

## Design intent

- **Ports live in `asterism-core`.** This crate defines adapters only,
  never traits.
- **`libsqlite3-sys` cluster alignment.** `rusqlite-isle` publishes
  separate release lines per `libsqlite3-sys` cluster; we pin to the
  line that lines up with `apalis-sql` (see the workspace `Cargo.toml`).

## Modules

- [`disclosure`](disclosure.md): Writing a [`DisclosureRecord`] into a file that already exists.
- [`dispatch`](dispatch.md): Outbound-dispatch runtime — `ExporterRegistry` plus the apalis
- [`dispatch::runtime`](dispatch__runtime.md): `DispatchRun` handler + `ExporterRegistry`.
- [`jobs`](jobs.md): Job engine — apalis with the `apalis-sql` SQLite backend.
- [`jobs::chapter_ffmetadata`](jobs__chapter_ffmetadata.md): Reading a container's declared chapter list through an external
- [`jobs::handlers`](jobs__handlers.md): Pipeline job handlers: `cover_gen`, `auto_tag`, `edge_rebuild`.
- [`jobs::preview_ffmpeg`](jobs__preview_ffmpeg.md): Preview-rendition transcode: an unplayable video in, an H.264 MP4
- [`jobs::thumb_ffmpeg`](jobs__thumb_ffmpeg.md): Video frame extraction through an external `ffmpeg`, for the
- [`observe`](observe.md): Observation — the `tracing` subscriber and the streams it writes.
- [`paths`](paths.md): Data-profile and on-disk layout conventions.
- [`probes`](probes.md): The probes this build has, and the one question a caller asks of all
- [`probes::jpeg`](probes__jpeg.md): JPEG's reading of the content axis: which of its segments are the
- [`probes::png`](probes__png.md): PNG's reading of the two walking axes: which of its chunks are the
- [`search`](search.md): Retrieval adapter — Tantivy on-disk index + Lindera Japanese
- [`search::fan_out`](search__fan_out.md): One [`AssetIndexer`] over several.
- [`search::tantivy_index`](search__tantivy_index.md): [`AssetRetriever`] + [`AssetIndexer`] adapter backed by an on-disk
- [`search::tokenizer`](search__tokenizer.md): Registers the `mixed_body` tokenizer on a Tantivy index.
- [`source_text`](source_text.md): Filesystem adapter for `SourceTextReader` — resolves asset
- [`sqlite`](sqlite.md): SQLite backend — connection lifecycle and schema migration built on
- [`sqlite::map`](sqlite__map.md): Row ↔ domain conversion helpers.
- [`sqlite::migrations`](sqlite__migrations.md): SQLite schema migrations — `PRAGMA user_version` scheme.
- [`sqlite::repo`](sqlite__repo.md): Repository adapters — SQLite implementations of the ports declared in
- [`sqlite::repo::app_setting`](sqlite__repo__app_setting.md): SQLite adapter for the `AppSettingRepository` port.
- [`sqlite::repo::asset`](sqlite__repo__asset.md): SQLite adapter for the `AssetRepository` port.
- [`sqlite::repo::asset_body`](sqlite__repo__asset_body.md): SQLite adapter for the `AssetBodyRepository` port.
- [`sqlite::repo::asset_comment`](sqlite__repo__asset_comment.md): SQLite adapter for the `AssetCommentRepository` port.
- [`sqlite::repo::asset_text_index`](sqlite__repo__asset_text_index.md): SQLite adapter for the write side of the **Query-side** text index
- [`sqlite::repo::attribution_guard`](sqlite__repo__attribution_guard.md): Write-side guard for the attribution columns.
- [`sqlite::repo::chapter_mark`](sqlite__repo__chapter_mark.md): SQLite adapter for the `ChapterMarkRepository` port.
- [`sqlite::repo::dir`](sqlite__repo__dir.md): SQLite adapter for `DirRepository`.
- [`sqlite::repo::dispatch`](sqlite__repo__dispatch.md): SQLite adapter for the `DispatchRepository` port.
- [`sqlite::repo::edge`](sqlite__repo__edge.md): SQLite adapter for the `EdgeRepository` port.
- [`sqlite::repo::group`](sqlite__repo__group.md): SQLite adapter for `GroupRepository`.
- [`sqlite::repo::instance`](sqlite__repo__instance.md): SQLite adapter for the `InstanceRepository` port.
- [`sqlite::repo::material_layer`](sqlite__repo__material_layer.md): SQLite adapter for the `MaterialLayerRepository` port.
- [`sqlite::repo::material_mark`](sqlite__repo__material_mark.md): SQLite adapter for the `MaterialMarkRepository` port.
- [`sqlite::repo::modality`](sqlite__repo__modality.md): SQLite adapter for the `ModalityRepository` port — the Modality
- [`sqlite::repo::persona`](sqlite__repo__persona.md): SQLite adapter for the `PersonaRepository` port (backed by rusqlite-isle).
- [`sqlite::repo::persona_profile`](sqlite__repo__persona_profile.md): SQLite adapter for the `PersonaProfileRepository` port.
- [`sqlite::repo::persona_theme`](sqlite__repo__persona_theme.md): SQLite adapter for the `PersonaThemeRepository` port.
- [`sqlite::repo::pursuit`](sqlite__repo__pursuit.md): SQLite adapter for the `PursuitRepository` port (#29).
- [`sqlite::repo::query_group`](sqlite__repo__query_group.md): SQLite adapter for `QueryGroupRepository` — the persistence half of
- [`sqlite::repo::series`](sqlite__repo__series.md): SQLite adapter for the `SeriesRepository` port — the series axis's
- [`sqlite::repo::session`](sqlite__repo__session.md): SQLite adapter for the [`SessionRepository`] port.
- [`sqlite::repo::snapshot`](sqlite__repo__snapshot.md): SQLite adapter for the `SnapshotRepository` port.
- [`sqlite::repo::tag`](sqlite__repo__tag.md): SQLite adapter for the `TagRepository` port (backed by rusqlite-isle).
- [`sqlite::repo::thread`](sqlite__repo__thread.md): SQLite adapter for the `ThreadRepository` port.
- [`sqlite::repo::thumb`](sqlite__repo__thumb.md): SQLite adapter for the `ThumbRepository` port.
- [`telemetry`](telemetry.md): Local telemetry — append-only `action_log` access (dogfooding

