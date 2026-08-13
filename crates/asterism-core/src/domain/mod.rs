//! Domain layer — entities, value objects, and repository ports.
//!
//! This module is the source of truth for the domain design; drafts of the
//! narrative live in private design notes.
//!
//! ## Modules
//!
//! - [`value`]        — value objects (id / slug / text newtypes, `Visibility`,
//!   `SourceRef`, `Page<T>`).
//! - [`app_setting`]  — closed registry of application setting keys plus
//!   the stored-override entity behind the `app_setting` table.
//! - [`persona`]      — `Persona`, the primary aggregate root.
//! - [`persona_theme`] — 1:1 side aggregate carrying persona-scoped
//!   UI chrome (wallpaper asset reference).
//! - [`asset`]        — `Asset` entity, the `AssetCard` read projection, and
//!   `AssetQuery`.
//! - [`material_layer`] — `MaterialLayer`: one band of marks over a
//!   material, carrying who produced it (`LayerOrigin`: imported from
//!   the file / written by the user / derived by a job) and what it
//!   holds (`LayerRole`: structure or annotation). What makes
//!   "re-read the file" able to replace the file's own reading without
//!   touching a person's.
//! - [`material_mark`] — `MaterialMark`: a note fastened to a point in
//!   an asset's *material* (the coordinate space its content carries).
//!   `MaterialAnchor` names which space and where — today the playback
//!   timeline (an instant or a half-open interval), tomorrow a
//!   rectangle on an image plane. Distinct from a comment on the asset
//!   as a whole. Belongs to an `Annotation` layer.
//! - [`chapter_mark`] — `ChapterMark`: one named section of a material,
//!   as the container declares it. Belongs to a `Structure` layer.
//!   Shares `TimelineSpan` with `material_mark` and nothing else — a
//!   note points *at* a position, a chapter states a *division*.
//! - [`attribution`]  — the attribution doctrine: `Author` /
//!   `OperatorRef` / `AttributionChannel` (who a record is by, which
//!   agent operated on their behalf, and through which channel that
//!   arrived), plus the `AttributionContext` write-path carrier.
//!   Distinct from `provenance`, which records where an artefact came
//!   from.
//! - [`instance`]     — `InstanceIdentity`: the single owner record
//!   `Author::Owner` refers to.
//! - [`material`]     — `Material`, the physical-original layer of an
//!   asset (locator / size / mime facts; aggregate-internal).
//! - [`source_locator`] — `SourceLocator` and the four shapes it is a
//!   sum of: where an artefact's bytes are, held as a typed value and
//!   the only code that knows the storage encoding.
//! - [`color`]        — `ColorBucket`, the closed swatch set the palette
//!   facet filters on (a projection of `asset.palette`).
//! - [`content_hash`] — fingerprint of an original's bytes; the axis
//!   duplicate detection groups on (distinct from `snapshot_hash`,
//!   which fingerprints a member list).
//! - [`content_region`] — the vocabulary of the content axis: the three
//!   things a reading of "which of an artefact's bytes decide what it
//!   decodes to" can conclude, and the markers each is stored as.
//! - [`series`]         — "made the same way": a `Strategy` (a data
//!   rule for reading a material's metadata) and the key it derives.
//!   A second sentence over `material_meta`'s map rather than a change
//!   to it, and derived without reading a byte, so a rule can be
//!   rewritten and the whole library re-derived from rows.
//! - [`probe`]          — the port those readings are written against.
//!   Format knowledge and container parsing live behind it, in
//!   `asterism-media-probe` and the adapters in `asterism-infra`.
//! - [`duplicate_conflict`] — the open question two rows holding the
//!   same bytes raise, and how it is answered (the edge records the
//!   fact; this records what is still to decide).
//! - [`merge_plan`]   — a person's ruling that a set of rows is one
//!   thing, checked as a declaration (the manual counterpart to the
//!   automatic 1:1 fold a `duplicate_conflict` answer produces).
//! - [`tag`]          — `Tag` (the channel entity, shared across personas).
//! - [`dir`]          — `Dir` (sidebar folder tree; organisation axis).
//! - [`edge`]         — `ConstellationEdge` (the hover-burst backbone).
//! - [`constellation`] — pure planning function for edge weights and labels.
//! - [`provenance`]   — `ProvenanceRef`, the declared origin a
//!   re-ingested artefact carries when it comes back from outside.
//! - [`job`]          — `Job` lifecycle model.
//! - [`observation`]  — the four observation streams (action / job /
//!   diag / perf) and the retention, sampling and persistence policy
//!   declared for each.
//! - [`repository`]   — port traits; implementations live in `asterism-infra`.
//!
//! ## Aggregate boundaries
//!
//! `Persona` and `Asset` are separate aggregates. "Persona is the primary
//! aggregate root" is realised as a `persona_id` bucket plus a cascade-delete
//! invariant, not as object containment (assets can reach tens of thousands
//! per persona and 100k+ overall).
//!
//! ## Invariants
//!
//! 1. Every `Asset` carries a `persona_id`.
//! 2. Deleting a `Persona` cascades to (or archives) its assets through the
//!    application service — repositories never cascade implicitly.
//! 3. `pack_id` is unique when present.
//! 4. `ConstellationEdge`: `from != to`, and `(from, to, kind)` is unique.
//! 5. `Restricted` visibility must be enforced at the query layer — assets
//!    hidden from a viewer must not appear in listings.
//! 6. Asterism never writes back to the real source (`source.locator`); only
//!    metadata / index / cover columns are Asterism's to change.

pub mod album_meta;
pub mod app_setting;
pub mod asset;
pub mod asset_comment;
pub mod attribution;
pub mod chapter_mark;
pub mod color;
pub mod constellation;
pub mod content_hash;
pub mod content_region;
pub mod dir;
pub mod disclosure;
pub mod dispatch;
pub mod duplicate_conflict;
pub mod edge;
pub mod group;
pub mod instance;
pub mod job;
pub mod material;
pub mod material_layer;
pub mod material_mark;
pub mod material_meta;
pub mod material_meta_raw;
pub mod merge_plan;
pub mod modality;
pub mod observation;
pub mod persona;
pub mod persona_profile;
pub mod persona_theme;
pub mod probe;
pub mod provenance;
pub mod query_group_eval;
pub mod render;
pub mod repository;
pub mod series;
pub mod session;
pub mod snapshot;
pub mod snapshot_hash;
pub mod sort_eval;
pub mod source_locator;
pub mod tag;
pub mod thread;
pub mod value;
