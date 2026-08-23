//! `UiError` — error type crossed by Tauri command handlers.
//!
//! Tauri commands must return `Result<T, E>` where `E: Serialize`
//! (`anyhow` will not do). We use a `serde` tagged enum
//! (`{ kind, message }`) so the TypeScript side can pattern-match on the
//! variant.
//!
//! Internally tagged with named fields rather than
//! `content = "message"` around a tuple, and the reason is one field
//! on one variant: a conflict carries `reason` beside its message, and
//! a variant holding two things cannot be a tuple behind a `content`
//! key without nesting them. What crosses the wire is unchanged for
//! every other variant — `{ kind, message }`, read that way in
//! `src/lib/mutate.ts` — and a conflict is that plus one field a
//! reader is free to ignore.

use asterism_core::DomainError;
use asterism_core::error::ConflictKind;

/// Error returned to the frontend from every Tauri command.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "kind")]
pub enum UiError {
    /// Target does not exist. Restricted assets are hidden with the same
    /// variant (they surface as "not found" for viewers outside their
    /// sharing list).
    #[error("not found: {message}")]
    NotFound {
        /// What was not found.
        message: String,
    },
    /// Input failed validation.
    #[error("validation error: {message}")]
    Validation {
        /// What was wrong with the input.
        message: String,
    },
    /// Conflict — for example, `pack_id` is already registered.
    ///
    /// `reason` is what separates "send it again" from "this will
    /// never work", which the variant alone cannot say. Same token the
    /// HTTP surface puts in its body, from the same
    /// [`ConflictKind`](asterism_core::error::ConflictKind).
    #[error("conflict: {message}")]
    Conflict {
        /// What the state said.
        message: String,
        /// What the caller can do about it: `"raced"`, `"blocked"`,
        /// `"settled"` or `"clashes"`.
        reason: &'static str,
    },
    /// Internal error (details are in the logs; the UI sees a summary).
    #[error("internal error: {message}")]
    Internal {
        /// A summary; the detail is in the logs.
        message: String,
    },
}

impl From<DomainError> for UiError {
    fn from(err: DomainError) -> Self {
        match err {
            DomainError::PersonaNotFound(id) => Self::NotFound {
                message: format!("persona {id}"),
            },
            DomainError::AssetNotFound(id) => Self::NotFound {
                message: format!("asset {id}"),
            },
            DomainError::NotFound { entity, id } => Self::NotFound {
                message: format!("{entity} {id}"),
            },
            // The caller has to name a different pack, which is what
            // `Clashes` means. Said here because the variant predates
            // the kind and has nowhere to carry one.
            DomainError::DuplicatePersona(pack) => Self::Conflict {
                message: format!("persona already registered: {pack}"),
                reason: ConflictKind::Clashes.as_str(),
            },
            DomainError::Validation(message) => Self::Validation { message },
            DomainError::Conflict { kind, message } => Self::Conflict {
                message,
                reason: kind.as_str(),
            },
            DomainError::Infra(err) => Self::Internal {
                message: err.to_string(),
            },
        }
    }
}
