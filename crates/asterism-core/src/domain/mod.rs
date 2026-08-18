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
//!     crates/asterism-core/src/domain/*.rs crates/asterism-core/src/domain/*/*.rs
//! ```
//!
//! What follows is the part no index can generate: how the modules hang
//! together, and the one dependency that decides where a new one goes.
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
//! [`snapshot_hash`]; [`provenance`] is the declared origin a re-ingested
//! artefact carries back, and [`disclosure`] its outbound counterpart — the pure
//! judgement of which IPTC digital-source term the recorded evidence
//! makes true of an artefact on its way out; [`edge`] holds typed
//! asset↔asset facts — from derivation
//! and identity to co-occurrence — that lineage walks and the hover burst
//! renders; [`attribution`] types who a write is by, what operated on their
//! behalf, and through which channel that answer arrived.
//!
//! **The forge layer.** [`forge`] is where intent lives: a line of work
//! (`forge::pursuit`), the culls and ledger entries it records, and the
//! conclusion it freezes. The rounds themselves are [`dispatch`], which
//! is a catalogue module — an exporter running over a frozen set is
//! something that happened to the bytes, and it works with no pursuit in
//! sight. Every other group above describes what is
//! true of the stored bytes; this one describes what somebody was trying
//! to do, and its own module doc states the boundary the two keep.
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
//! ## The one dependency
//!
//! ```text
//!   forge ──uses──▶ catalogue
//! ```
//!
//! The forge names catalogue types. The catalogue should name a forge
//! **id** and nothing else — no forge entity, no forge event, no forge
//! port — and a change that adds one is the change to refuse.
//!
//! **The arrow does not hold yet, and this is not a claim that it
//! does.** [`repository`] declares `PursuitRepository` and
//! `ProjectRepository` alongside the catalogue's own ports and imports
//! seven forge types to do it; `application::asset_service` holds an
//! `Arc<dyn PursuitRepository>`; `application::mapping` parses a
//! pursuit id. Those are the open work on #81, not exceptions the rule
//! makes room for. Cutting [`forge`] into its own crate is what would
//! put the refusal in the compiler rather than in a reviewer's head,
//! and until then this paragraph is the whole enforcement.
//!
//! No other cross-module rule is stated here, because the ones that
//! used to be stated twice drifted. Each lives next to the type that
//! enforces it — [`attribution`] for who a write is by, [`snapshot`]
//! for why a freeze carries no origin story, [`edge`] for the two
//! populations that share one table, and [`forge::pursuit`] for why a
//! unit of work is minted rather than derived.
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
pub mod derived_text;
pub mod dir;
pub mod disclosure;
pub mod dispatch;
pub mod duplicate_conflict;
pub mod edge;
pub mod embedded_text;
pub mod forge;
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
