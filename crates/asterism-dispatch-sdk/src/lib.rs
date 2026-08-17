//! # asterism-dispatch-sdk
//!
//! Building blocks for Asterism **exporters** (outbound adapters).
//!
//! An exporter is the OUT-side counterpart of the importer: given a
//! Selection of already-ingested [`Asset`][crate-note-asset]s, an
//! exporter drives an external backend (ComfyUI HTTP, Gemini API,
//! VDSL CLI, algocline `alc.sd.bake()`, …) and returns
//! [`Derived`] payloads that the core reifies as new Assets whose
//! `parent_ids` point back at the Selection's members via
//! `ConstellationEdge { kind: "derived_from" }`.
//!
//! ## Pipeline
//!
//! ```text
//!  Selection<AssetId>
//!         │
//!         ▼
//!  DispatchJob { selection_id, exporter_slug, action, params }
//!         │
//!         ▼
//!  ┌── asterism-dispatch-sdk ─────────────────────────┐
//!  │  Exporter::dispatch(ctx)  -> Handle              │
//!  │  Exporter::poll(&handle)  -> DispatchState       │
//!  │  Exporter::harvest(&h)    -> Vec<Derived>        │
//!  └────────────────────┬─────────────────────────────┘
//!                       ▼
//!  Core: reify_derived(job, derived[i]) -> new Asset
//!    + ConstellationEdge { kind: "derived_from" }
//! ```
//!
//! ## Why this shape (and not `Footprint` reuse)
//!
//! The importer side hands parsers a `Footprint` shape whose
//! `FootprintSource { kind, locator, platform }` field encodes
//! **external world anchoring** — every importer represents "here is a
//! thing we found in the wild". Exporters, by contrast, always run
//! against Assets we already own, and the derived output's provenance
//! (persona, session, parent_ids, source_kind) is always supplied by
//! the core from the [`DispatchContext`]. Reusing `Footprint` would
//! either force exporter authors to fill in fake source metadata or
//! introduce an escape hatch the core has to remember to overwrite —
//! either way the shape argues against itself.
//!
//! [`Derived`] is therefore intentionally smaller than `Footprint`
//! (no source_kind, no persona_id, no session_id) and semantically
//! different: it describes "a new artefact I made, hook it up as a
//! child of these Assets".
//!
//! [crate-note-asset]: The full-fat `Asset` entity lives in
//! `asterism-core`. Every shared boundary type — `AssetCardDto`
//! (what an exporter's [`DispatchContext.inputs`][DispatchContext]
//! carries) and [`Derived`] (what an exporter hands back) — lives
//! in the leaf `asterism-contract` crate so the SDK sits above
//! contract (and only contract), the core sits above contract
//! (and only contract), and neither depends on the other.
//!
//! ## SDK as external contract
//!
//! Beyond the Rust `Exporter` trait, this crate publishes the
//! **canonical wire shape** every backend author (Rust adapter,
//! Comfy watch-folder plugin, Python receiver, CC-written adapter)
//! is supposed to consume. The actual DTOs live in
//! `asterism-contract`; the SDK exposes them via re-exports plus
//! example-JSON artifacts under [`schema`]. Mirrors the harvest
//! importer pattern where `asterism-importer-harvest
//! --print-schema` streams the canonical ingest shape.
//!
//! ## What this crate is not
//!
//! It is the port, so it holds the trait, the types crossing it, and
//! the schema artifacts that describe them — and nothing an adapter
//! merely finds convenient. Machinery shared *between* adapters, such
//! as the `{{...}}` substitution and path grammar a schema-driven
//! exporter is configured with, sits one layer out in
//! `asterism-exporter-common`, which depends on this crate rather than
//! the other way round. Putting it here would make every backend
//! author consuming the port read a grammar their adapter may never
//! use, and would let a change in one adapter's convenience reach the
//! contract every other adapter is written against.

pub mod attempt;
pub mod derived;
pub mod exporter;
pub mod handle;
pub mod schema;
pub mod state;

pub use attempt::{AttemptRecord, AttemptRecorder, DISCARD_ATTEMPTS, DiscardAttempts};
pub use derived::{COVER_MAX_CHARS, Derived, REGISTER_MAX_CHARS};
pub use exporter::{DispatchContext, Exporter, ExporterError};
pub use handle::Handle;
pub use schema::{
    SDK_SCHEMAS, SdkSchemaEntry, derived_example_json, dispatch_context_example_json,
    find_sdk_schema,
};
pub use state::{DispatchState, ProgressHint};
