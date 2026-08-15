//! Domain layer — entities, value objects, and repository ports.
//!
//! This module is the source of truth for the domain design. It deliberately
//! carries **no per-module inventory**: rustdoc builds the Modules index on
//! this page from the first paragraph of every child's own doc, so a
//! hand-written copy can only ever be a stale duplicate — the one this file
//! used to carry covered 27 of 42 modules by the time the gap was caught
//! (#25). To read the full, always-current index straight from the tree
//! with no doc build (each opening paragraph, which may wrap over a few
//! source lines):
//!
//! ```text
//! awk '/^\/\/!$/ { nextfile } /^\/\/!/ { print FILENAME ": " substr($0, 5) }' \
//!     crates/asterism-core/src/domain/*.rs
//! ```
//!
//! What follows is the part no index can generate: how the modules hang
//! together, and the doctrines that reading one module at a time gets wrong.
//!
//! ## A tour of the domain
//!
//! **The catalogue.** [`persona`] is the primary aggregate root; [`asset`]
//! is one catalogued footprint, with [`material`] as its physical-original
//! layer and [`value`] holding the shared newtypes. [`modality`], [`tag`],
//! [`dir`] and [`group`] are the organisation axes over it; [`instance`]
//! names the single owner that `Author::Owner` refers to.
//!
//! **The identity of bytes.** What makes two rows "the same":
//! [`content_hash`] fingerprints a whole file, [`content_region`] narrows
//! to the bytes that decide what it decodes to, [`material_meta`] /
//! [`material_meta_raw`] canonicalise and keep the container's metadata,
//! [`series`] derives "made the same way", [`source_locator`] types where
//! the bytes live, and [`probe`] is the port those readings are written
//! against. When identity collides, [`duplicate_conflict`] holds the open
//! question and [`merge_plan`] a person's ruling over a whole set.
//!
//! **The record layer.** [`snapshot`] freezes an ordered asset set —
//! content-addressed, a git-tree analogue, fingerprinted by
//! [`snapshot_hash`]. [`dispatch`] is one exporter invocation against such
//! a freeze; [`pursuit`] is the minted unit of work those invocations are
//! stamped with — the line of rounds and verdicts toward one intent;
//! [`provenance`] is the declared origin a re-ingested artefact
//! carries back, and [`disclosure`] its outbound counterpart — the pure
//! judgement of which IPTC digital-source term the recorded evidence
//! makes true of an artefact on its way out; [`edge`] holds typed
//! asset↔asset facts — from derivation
//! and identity to co-occurrence — that lineage walks and the hover burst
//! renders; [`attribution`] types who a write is by, what operated on their
//! behalf, and through which channel that answer arrived.
//!
//! **The annotation layer.** [`thread`] collects messages from humans and
//! agents alike; [`asset_comment`] is the short-note thread on one asset;
//! [`material_layer`] bands marks by who produced them, [`material_mark`]
//! pins a note into a material's own coordinate space, [`chapter_mark`]
//! states a division, and [`album_meta`] is what a person or an agent says
//! about an asset in Album's own words.
//!
//! **Evaluation and presentation.** [`query_group_eval`] holds the pure
//! pieces of query-group materialisation and [`sort_eval`] the backend port
//! of the grid comparator; [`render`] decides thumbnail eligibility and
//! preview mode in one place; [`constellation`] plans edge weights and
//! labels; [`color`] is the palette facet's closed swatch set; [`session`]
//! is the Dialog-modality aggregate root.
//!
//! **Runtime support.** [`job`] models asynchronous work, [`observation`]
//! the four telemetry streams and their policies, [`app_setting`] the
//! closed setting registry, [`persona_profile`] / [`persona_theme`] a
//! persona's identity signal and visual chrome, and [`repository`] the
//! persistence ports `asterism-infra` implements.
//!
//! The tour groups by capability and makes no completeness promise — a
//! module absent here is an omission from a narrative, not from the
//! record; the generated index and the command above are the inventory.
//!
//! ## Doctrines
//!
//! The recurring decisions that reading code alone misleads on. Each is
//! stated in full next to the type that carries it; this is the
//! cross-module view.
//!
//! 1. **Events, not state.** A recorded act is one row per invocation over
//!    a frozen set, with one-way lifecycle transitions — re-dispatching a
//!    snapshot inserts a new `DispatchJob` rather than rolling a status
//!    row per snapshot-exporter pair — and "current standing" is derived
//!    on read: `duplicate_conflict` refuses a third resolution value, and
//!    its repository re-derives "one of them went away" against the
//!    assets on every read rather than writing it into the queue row.
//! 2. **Facts and verdicts stay apart.** Edges are facts about content and
//!    deliberately carry no actor and no timestamp; verdicts — a conflict
//!    resolution, a fold, a merge ruling — live on their own rows, where
//!    who and when can be recorded.
//! 3. **Freeze, then refer.** A snapshot carries no name, no note, no
//!    origin story. Every statement of *where it came from* lives on the
//!    referencing event (`dispatch_job.source_group_id` /
//!    `source_query_json`), never on the snapshot itself (`migrations.rs`,
//!    `v19_selection_model`). Extends to pursuits: a pursuit carries
//!    intent (`title`, `note`) and lineage of work (`parent_id`) but never
//!    content — what happened lives on the stamped events, and the one
//!    materialised set (the close product) is itself a frozen snapshot
//!    the event refers to.
//! 4. **Attribution answers "whose write is this" and stops there.** The
//!    `(author, operator, via)` triple is about the write; where an
//!    artefact came from is [`provenance`]'s question. The two vocabularies
//!    do not mix.
//! 5. **The unit of work is minted, never derived.** Content identity
//!    changes whenever work is redone, so correlation by ancestry alone
//!    cannot express succession, rejection, or abandonment. A [`pursuit`]
//!    is identified by a minted id stamped on its events, work cannot
//!    happen outside one (a dispatch without a pursuit gets one minted in
//!    the same request; a mint stranded by a failed dispatch write is the
//!    legal pre-created-empty state, not debris), and everything else
//!    about it — standing, membership, rollups — is projection.
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
pub mod pursuit;
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
