//! `DomainError` — the innermost error type shared across every layer of
//! `asterism-core`.
//!
//! Outer layers (the Tauri `UiError`, the HTTP `ApiError`, MCP tool errors)
//! convert from this enum. Infrastructure-level failures are collapsed into
//! `Infra` so that domain vocabulary (`NotFound` / `Duplicate` / `Validation`
//! / `Conflict`) stays clean.

use crate::domain::value::{AssetId, PackId, PersonaId};

/// Errors raised by the domain layer.
///
/// Convention: `asterism-infra` converts adapter failures into
/// `anyhow::Error` and passes them through `Infra`; application services
/// return the specific domain variants directly.
#[derive(thiserror::Error, Debug)]
pub enum DomainError {
    /// No persona exists for the given id.
    #[error("persona not found: {0}")]
    PersonaNotFound(PersonaId),

    /// No asset exists for the given id.
    #[error("asset not found: {0}")]
    AssetNotFound(AssetId),

    /// No row of `entity` exists for `id` — the generic sibling of
    /// [`Self::PersonaNotFound`] / [`Self::AssetNotFound`] covering every
    /// other aggregate (session / modality / group / dir / snapshot /
    /// dispatch / thread / comment).
    ///
    /// Use this rather than `Validation` or `Conflict` for a missing
    /// target: the transport layers map it to `404`, so a client can tell
    /// "you asked for something that is not here" (`404`) apart from "your
    /// request was malformed" (`400`) and "the state fought back" (`409`).
    /// `entity` is a `&'static str` label (`"session"`, `"group"`, …) that
    /// only feeds the message; callers should not branch on it.
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Aggregate label used in the message (`"session"`, `"group"`, …).
        entity: &'static str,
        /// Identifier the caller asked for, rendered as text.
        id: String,
    },

    /// A persona with the same `pack_id` is already registered
    /// (invariant: `pack_id` is unique when present).
    #[error("persona already registered for pack id: {0}")]
    DuplicatePersona(PackId),

    /// Value-object / entity construction failed validation (empty slug,
    /// self-loop edge, out-of-range timestamp, and so on).
    #[error("validation error: {0}")]
    Validation(String),

    /// State conflict against the persistence layer (unique-constraint
    /// violation, optimistic-lock race, and so on).
    #[error("conflict: {0}")]
    Conflict(String),

    /// Failure originating from infrastructure (SQLite, filesystem, MCP
    /// client, etc.).
    #[error("infrastructure error: {0}")]
    Infra(#[from] anyhow::Error),
}

impl DomainError {
    /// Builds [`Self::NotFound`] from any `Display` id, so call sites do
    /// not repeat `id: id.to_string()`.
    pub fn not_found(entity: &'static str, id: impl std::fmt::Display) -> Self {
        Self::NotFound {
            entity,
            id: id.to_string(),
        }
    }
}
