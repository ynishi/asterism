# asterism-core 0.0.0

# asterism-core — Domain + Application layer

## Architecture

Hexagonal core of Asterism (no Tauri, no SQLite dependency), organised as:

- `domain/`: Entity types (Persona / Asset / Tag / ConstellationEdge), Value
  Objects (PersonaId / AssetId / Modality / …), repository traits (ports),
  and `DomainError` (via `thiserror`).
- `application/`: use case services (PersonaService / AssetService) plus a
  `JobQueue` port; the actual job engine lives in `asterism-infra`.
  Everything here is fronted by a transport adapter (a Tauri command,
  an HTTP route, or both).
- `application_support/`: use case services nothing on the wire
  fronts — the retention sweep, the bulk Query Group refresh, and the
  runner-side dispatch transitions. Driven by the job worker, the
  dispatch runner, or startup. The split is enforced by which context
  carries the handle, not by convention: the support bundle lives on
  `CoreCtx` and is deliberately absent from `ServerCtx` / `AppState`.
- `error.rs`: the `DomainError` enum.

Ports are declared here; adapters that implement them live in
`asterism-infra` (dependency inversion).

## Design intent

- **Persona is the primary aggregate root.** Every asset belongs to
  exactly one persona (`persona_id`); persona deletion cascades (or
  archives) its assets and assets are never shared directly between
  personas. Membership says where an asset is filed, not who wrote it
  (`domain::attribution`).
- **Weak CQRS split.** Reads use `&self` methods returning projections;
  writes take `&mut self` and emit domain events. Read models are not
  maintained as a separate store.
- **Simple in-process state.** No workspace state machine; SQLite for
  persistence, `Arc<Service>` in Tauri state, and event emission are
  enough.
- **Compose existing primitives.** New code is a thin glue layer where
  possible.
- **Template-driven only.** No character generation / gacha loops; the
  product is a memory grid over existing personas, not a persona factory.

## Status

Turn B–D landed: `domain/` (entities / VOs / ports) + `application/`
(services + DTO mapping) + `error.rs`. The job engine and repository
adapters live in `asterism-infra`. Domain design rationale is tracked in
the private design notes.

## Modules

- [`application`](application.md): Application layer — use-case services.
- [`application::app_setting_service`](application__app_setting_service.md): `AppSettingService` — use cases for application settings.
- [`application::asset_comment_service`](application__asset_comment_service.md): `AssetCommentService` — thread lifecycle on an Asset.
- [`application::asset_service`](application__asset_service.md): `AssetService` — asset lifecycle, grid reads, and detail views.
- [`application::disclosure_service`](application__disclosure_service.md): Building an artefact's disclosure out of the library, and putting it
- [`application::dispatch_service`](application__dispatch_service.md): `DispatchService` — the transport-fronted half of the outbound
- [`application::fold_redirect`](application__fold_redirect.md): Redirecting a named id set through the folds that happened after it
- [`application::forge`](application__forge.md): Forge use cases — the verbs of a line of work.
- [`application::forge::line_service`](application__forge__line_service.md): Line use cases — opening one, reading what is on it, and moving its
- [`application::forge::pursuit_service`](application__forge__pursuit_service.md): Work use cases — opening a line of work, writing rounds, looking at
- [`application::forge::thread_service`](application__forge__thread_service.md): Saying something about work, and correcting it.
- [`application::mapping`](application__mapping.md): Conversion between domain types and contract DTOs.
- [`application::material_layer_service`](application__material_layer_service.md): `MaterialLayerService` — the bands of marks over an Asset's
- [`application::material_mark_service`](application__material_mark_service.md): `MaterialMarkService` — the marks placed into an Asset's material.
- [`application::modality_service`](application__modality_service.md): `ModalityService` — use cases for the Modality master.
- [`application::persona_service`](application__persona_service.md): `PersonaService` — use cases for the persona lifecycle.
- [`application::query_group_invalidation`](application__query_group_invalidation.md): Query Group invalidation — the W4 hook that translates a
- [`application::query_group_service`](application__query_group_service.md): `QueryGroupService` — the Query Group evaluate-and-materialize
- [`application::series_strategy_service`](application__series_strategy_service.md): `SeriesStrategyService` — registering, editing and removing the rules
- [`application::session_service`](application__session_service.md): `SessionService` — use cases for the Session 1st-class entity.
- [`application::snapshot_service`](application__snapshot_service.md): `SnapshotService` — application surface for the immutable `Snapshot`
- [`application::sort_context`](application__sort_context.md): Assembles the [`SortContext`] the sort evaluator needs, sourcing every
- [`application::thread_service`](application__thread_service.md): `ThreadService` — application-layer surface for `Thread` and its
- [`application::thumb_service`](application__thumb_service.md): `ThumbService` — pre-generated thumbnail cache use cases.
- [`application_support`](application_support.md): Application **support** layer — use cases no transport adapter
- [`application_support::chapter_intake`](application_support__chapter_intake.md): What a fresh reading of a material's chapter list means for the
- [`application_support::dispatch_runner_service`](application_support__dispatch_runner_service.md): `DispatchRunnerService` — the runner-side half of the outbound
- [`application_support::duplicate_detection`](application_support__duplicate_detection.md): Duplicate detection — what happens the moment a fingerprint lands on
- [`application_support::query_group_refresh_service`](application_support__query_group_refresh_service.md): `QueryGroupRefreshService` — bulk Query Group re-evaluation.
- [`application_support::retention_service`](application_support__retention_service.md): `RetentionService` — the trash retention sweep.
- [`domain`](domain.md): Domain layer — entities, value objects, and repository ports.
- [`domain::album_meta`](domain__album_meta.md): AlbumMeta — what a person or an agent *says about* an asset, in
- [`domain::app_setting`](domain__app_setting.md): Application settings — the closed key registry and its stored
- [`domain::asset`](domain__asset.md): `Asset` — an aggregate root for a single footprint, plus the read
- [`domain::asset_comment`](domain__asset_comment.md): `AssetComment` — a thread of short notes attached to an Asset.
- [`domain::attribution`](domain__attribution.md): Attribution — *who* a record is by, *what* operated on their behalf,
- [`domain::chapter_mark`](domain__chapter_mark.md): `ChapterMark` — one entry in a chapter list: a named section of an
- [`domain::color`](domain__color.md): `ColorBucket` — the closed set of colours the palette facet filters
- [`domain::constellation`](domain__constellation.md): Constellation edge planning — pure domain logic that decides how a
- [`domain::content_hash`](domain__content_hash.md): `content_hash` — the fingerprint of an original artefact's bytes.
- [`domain::content_region`](domain__content_region.md): `content_region` — what a reading of "the bytes that decide what
- [`domain::derived_text`](domain__derived_text.md): Derived text — the one string an asset offers a full-text index,
- [`domain::dir`](domain__dir.md): `Dir` — a persona-scoped folder tree for organising the sidebar.
- [`domain::disclosure`](domain__disclosure.md): What an artefact discloses about how it was made, and the rule that
- [`domain::disclosure::outcome`](domain__disclosure__outcome.md): What applying a record to a file actually achieved.
- [`domain::disclosure::record`](domain__disclosure__record.md): `DisclosureRecord` — everything one exported file is going to say
- [`domain::disclosure::source_type`](domain__disclosure__source_type.md): `DigitalSourceType` — the one field a synthetic file is obliged to
- [`domain::disclosure::generator_keys`](domain__disclosure__generator_keys.md): Keys that only a generator writes, one per family.
- [`domain::dispatch`](domain__dispatch.md): `DispatchJob` — one exporter invocation against a Snapshot.
- [`domain::duplicate_conflict`](domain__duplicate_conflict.md): `DuplicateConflict` — one open question of the form "these two rows
- [`domain::edge`](domain__edge.md): `ConstellationEdge` — the backbone of the hover-burst experience.
- [`domain::embedded_text`](domain__embedded_text.md): `embedded_text` — the words a container wrote *into* an artefact,
- [`domain::forge`](domain__forge.md): The forge layer — the intentional history over the raw layer: a line
- [`domain::forge::boundary`](domain__forge__boundary.md): The only place another layer's words appear.
- [`domain::forge::boundary::actors`](domain__forge__boundary__actors.md): The face that asks who somebody is.
- [`domain::forge::boundary::store`](domain__forge__boundary__store.md): The face that asks downward, and the client that speaks it.
- [`domain::forge::clock`](domain__forge__clock.md): What time it is.
- [`domain::forge::closings`](domain__forge__closings.md): Ending work — the one call that writes to both logs.
- [`domain::forge::lines`](domain__forge__lines.md): Keeping lines, stated in the forge's own words.
- [`domain::forge::model`](domain__forge__model.md): The forge's model — the domain, and only the domain.
- [`domain::forge::model::act`](domain__forge__model__act.md): When something happened, and who did it.
- [`domain::forge::model::change`](domain__forge__model__change.md): Putting work on a line — the one place both logs are read at once.
- [`domain::forge::model::closing`](domain__forge__model__closing.md): Ending work — the one act that moves both logs.
- [`domain::forge::model::discard`](domain__forge__model__discard.md): What goes when a line goes, and what that lets go of.
- [`domain::forge::model::error`](domain__forge__model__error.md): What the model refuses.
- [`domain::forge::model::history`](domain__forge__model__history.md): A line's history: what it carries, and how it got there.
- [`domain::forge::model::line`](domain__forge__model__line.md): The line — the forge's top entity.
- [`domain::forge::model::op`](domain__forge__model__op.md): What work writes, and what it folds into.
- [`domain::forge::model::pursuit`](domain__forge__model__pursuit.md): One line of work: what it is trying to do, and every round at it.
- [`domain::forge::model::react`](domain__forge__model__react.md): Letting a line's rule answer a collision.
- [`domain::forge::model::restore`](domain__forge__model__restore.md): Building the model back from what a store kept.
- [`domain::forge::model::strategy`](domain__forge__model__strategy.md): How a line settles a collision — the contract, and not the rule.
- [`domain::forge::model::table`](domain__forge__model__table.md): What a change point carries, and what folding a sequence of them
- [`domain::forge::model::thread`](domain__forge__model__thread.md): Saying something about work.
- [`domain::forge::model::value`](domain__forge__model__value.md): The values the model is made of.
- [`domain::forge::pursuits`](domain__forge__pursuits.md): Keeping pursuits, stated in the forge's own words.
- [`domain::forge::strategies`](domain__forge__strategies.md): The rules a line can settle collisions by, and the lookup that
- [`domain::forge::threads`](domain__forge__threads.md): Keeping what was said about work.
- [`domain::generator_params`](domain__generator_params.md): `generator_params` — what an extraction concluded about the
- [`domain::group`](domain__group.md): `Group` — a user-curated set of assets, persona-scoped.
- [`domain::instance`](domain__instance.md): Instance identity — the referent behind
- [`domain::job`](domain__job.md): `Job` — lifecycle model for asynchronous work.
- [`domain::material`](domain__material.md): `Material` — the physical-original layer of an asset (asset-model v4).
- [`domain::material_layer`](domain__material_layer.md): `MaterialLayer` — one band of marks over an Asset's material, and
- [`domain::material_mark`](domain__material_mark.md): `MaterialMark` — a mark placed into an Asset's **material**: the
- [`domain::material_meta`](domain__material_meta.md): `material_meta` — the canonical form the metadata a container
- [`domain::material_meta_raw`](domain__material_meta_raw.md): `material_meta_raw` — the container's metadata bytes, kept verbatim,
- [`domain::measurement`](domain__measurement.md): `measurement` — what a fingerprint column's *status* can say, now
- [`domain::merge_plan`](domain__merge_plan.md): `MergePlan` — a person's ruling that a set of rows is one thing, and
- [`domain::modality`](domain__modality.md): `ModalityDef` — the open Modality master entry (the `modality` table).
- [`domain::observation`](domain__observation.md): Observability domain — the four streams and their policies.
- [`domain::persona`](domain__persona.md): `Persona` — the primary aggregate root.
- [`domain::persona_profile`](domain__persona_profile.md): `PersonaProfile` — a 1:1 side aggregate holding the identity
- [`domain::persona_theme`](domain__persona_theme.md): `PersonaTheme` — a 1:1 aggregate holding the persona-scoped visual
- [`domain::probe`](domain__probe.md): `probe` — the port a format's identity measurement is written
- [`domain::provenance`](domain__provenance.md): `ProvenanceRef` — how a re-ingested artefact names where it came from.
- [`domain::provenance::source`](domain__provenance__source.md): `_trace.source` vocabulary — which channel a provenance claim
- [`domain::query_group_eval`](domain__query_group_eval.md): Query Group evaluation — the pure pieces of the materialize pipeline.
- [`domain::render`](domain__render.md): How an asset is rendered — thumbnail eligibility, media path, and
- [`domain::repository`](domain__repository.md): Repository ports — the persistence traits declared here and implemented
- [`domain::series`](domain__series.md): `series` — "made the same way": a rule for reading a material's
- [`domain::session`](domain__session.md): `Session` — the Dialog-modality 1st-class aggregate root.
- [`domain::snapshot`](domain__snapshot.md): `Snapshot` — an immutable, content-addressed freeze of an ordered
- [`domain::snapshot_hash`](domain__snapshot_hash.md): Snapshot `content_hash` — the canonical member-set fingerprint.
- [`domain::sort_eval`](domain__sort_eval.md): Sort evaluation — the backend port of the UI grid comparator.
- [`domain::source_locator`](domain__source_locator.md): `source_locator` — where an artefact's bytes are, held as a typed
- [`domain::tag`](domain__tag.md): `Tag` — the channel entity (a classification axis shared across
- [`domain::tag_head`](domain__tag_head.md): The trained tag head (#132 phase 2): per-tag logistic rows over
- [`domain::thread`](domain__thread.md): `Thread` — the app-level container that collects `Message`s from
- [`domain::value`](domain__value.md): Value objects: id / slug / text newtypes plus `Visibility`, `SourceRef`,
- [`domain::visual`](domain__visual.md): Visual-feature vocabulary and the encoder port (#112).
- [`error`](error.md): `DomainError` — the innermost error type shared across every layer of

