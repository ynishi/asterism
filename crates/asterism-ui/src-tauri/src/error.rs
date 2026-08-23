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
        // Asked before the value is taken apart, and asked of
        // [`DomainError::reason`] rather than decided here: whether a
        // refusal carries retry advice is one question, and this
        // surface answering it separately from HTTP and MCP is what
        // let the three drift. Carrying a token is also what makes a
        // refusal a conflict, so the answer picks the variant too.
        if let Some(reason) = err.reason() {
            let message = match err {
                DomainError::DuplicatePersona(pack) => {
                    format!("persona already registered: {pack}")
                }
                DomainError::Conflict { message, .. } => message,
                // Unreached today: only the two above answer a reason.
                // Written as a value rather than a panic so that a
                // variant gaining one later arrives with its own words
                // instead of taking the app down.
                other => other.to_string(),
            };
            return Self::Conflict { message, reason };
        }
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
            DomainError::Validation(message) => Self::Validation { message },
            DomainError::Infra(err) => Self::Internal {
                message: err.to_string(),
            },
            // Both are answered above, where the reason was read; the
            // arm is here so that this match stays exhaustive by name
            // and a new variant cannot slip through a catch-all.
            conflict @ (DomainError::DuplicatePersona(_) | DomainError::Conflict { .. }) => {
                Self::Internal {
                    message: conflict.to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both conflict shapes reach the frontend with the token
    /// [`DomainError::reason`] gives them, and a refusal that carries
    /// no token is not dressed as a conflict on the way out.
    ///
    /// The duplicate is the one worth pinning: it has no
    /// `ConflictKind` of its own, so a surface deciding for itself is
    /// how the three transports came to answer this separately.
    #[test]
    fn a_conflict_crosses_with_the_token_the_domain_gives_it() {
        let pack = asterism_core::domain::value::PackId::new("pack").expect("a literal slug");
        let duplicate = UiError::from(DomainError::DuplicatePersona(pack));
        assert!(
            matches!(&duplicate, UiError::Conflict { reason, message }
                if *reason == "clashes" && message.contains("pack")),
            "{duplicate}"
        );

        let blocked = UiError::from(DomainError::blocked("archive it first"));
        assert!(
            matches!(&blocked, UiError::Conflict { reason, .. } if *reason == "blocked"),
            "{blocked}"
        );

        let malformed = UiError::from(DomainError::Validation("a name is not blank".into()));
        assert!(
            matches!(malformed, UiError::Validation { .. }),
            "{malformed}"
        );
    }
}
