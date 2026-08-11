//! # asterism-core — Domain + Application layer
//!
//! ## Architecture
//!
//! Hexagonal core of Asterism (no Tauri, no SQLite dependency), organised as:
//!
//! - `domain/`: Entity types (Persona / Asset / Tag / ConstellationEdge), Value
//!   Objects (PersonaId / AssetId / Modality / …), repository traits (ports),
//!   and `DomainError` (via `thiserror`).
//! - `application/`: use case services (PersonaService / AssetService) plus a
//!   `JobQueue` port; the actual job engine lives in `asterism-infra`.
//!   Everything here is fronted by a transport adapter (a Tauri command,
//!   an HTTP route, or both).
//! - `application_support/`: use case services nothing on the wire
//!   fronts — the retention sweep, the bulk Query Group refresh, and the
//!   runner-side dispatch transitions. Driven by the job worker, the
//!   dispatch runner, or startup. The split is enforced by which context
//!   carries the handle, not by convention: the support bundle lives on
//!   `CoreCtx` and is deliberately absent from `ServerCtx` / `AppState`.
//! - `error.rs`: the `DomainError` enum.
//!
//! Ports are declared here; adapters that implement them live in
//! `asterism-infra` (dependency inversion).
//!
//! ## Design intent
//!
//! - **Persona is the primary aggregate root.** Every asset belongs to
//!   exactly one persona (`persona_id`); persona deletion cascades (or
//!   archives) its assets and assets are never shared directly between
//!   personas. Membership says where an asset is filed, not who wrote it
//!   (`domain::attribution`).
//! - **Weak CQRS split.** Reads use `&self` methods returning projections;
//!   writes take `&mut self` and emit domain events. Read models are not
//!   maintained as a separate store.
//! - **Simple in-process state.** No workspace state machine; SQLite for
//!   persistence, `Arc<Service>` in Tauri state, and event emission are
//!   enough.
//! - **Compose existing primitives.** New code is a thin glue layer where
//!   possible.
//! - **Template-driven only.** No character generation / gacha loops; the
//!   product is a memory grid over existing personas, not a persona factory.
//!
//! ## Status
//!
//! Turn B–D landed: `domain/` (entities / VOs / ports) + `application/`
//! (services + DTO mapping) + `error.rs`. The job engine and repository
//! adapters live in `asterism-infra`. Domain design rationale is tracked in
//! the private design notes.

#![warn(missing_docs)]

pub mod application;
pub mod application_support;
pub mod domain;
pub mod error;

pub use error::DomainError;
