//! `UiError` — error type crossed by Tauri command handlers.
//!
//! Tauri commands must return `Result<T, E>` where `E: Serialize`
//! (`anyhow` will not do). We use a `serde` tagged enum
//! (`{ kind, message }`) so the TypeScript side can pattern-match on the
//! variant.

use asterism_core::DomainError;

/// Error returned to the frontend from every Tauri command.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum UiError {
    /// Target does not exist. Restricted assets are hidden with the same
    /// variant (they surface as "not found" for viewers outside their
    /// sharing list).
    #[error("not found: {0}")]
    NotFound(String),
    /// Input failed validation.
    #[error("validation error: {0}")]
    Validation(String),
    /// Conflict — for example, `pack_id` is already registered.
    #[error("conflict: {0}")]
    Conflict(String),
    /// Internal error (details are in the logs; the UI sees a summary).
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<DomainError> for UiError {
    fn from(err: DomainError) -> Self {
        match err {
            DomainError::PersonaNotFound(id) => Self::NotFound(format!("persona {id}")),
            DomainError::AssetNotFound(id) => Self::NotFound(format!("asset {id}")),
            DomainError::NotFound { entity, id } => Self::NotFound(format!("{entity} {id}")),
            DomainError::DuplicatePersona(pack) => {
                Self::Conflict(format!("persona already registered: {pack}"))
            }
            DomainError::Validation(message) => Self::Validation(message),
            DomainError::Conflict(message) => Self::Conflict(message),
            DomainError::Infra(err) => Self::Internal(err.to_string()),
        }
    }
}
