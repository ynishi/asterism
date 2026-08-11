//! `Persona` — the primary aggregate root.
//!
//! "Primary aggregate root" is realised through a `persona_id` bucket
//! relationship rather than object containment (a persona may accumulate
//! tens of thousands of assets, so nested containment does not scale). The
//! `Asset` aggregate holds a `persona_id` reference and the application
//! service enforces the cascade-delete invariant.

use chrono::{DateTime, Utc};

use crate::domain::value::{PackId, PersonaId};
use crate::error::DomainError;

/// A persona registered inside Asterism (aggregate root).
///
/// When the identity source of truth is an external persona pack, set
/// `pack_id` to the pack's own id. Leaving `pack_id` at `None` marks the
/// persona as an ad-hoc bucket for a generic consumer (imported chat
/// history, an image library, and so on) — Asterism is a general-purpose
/// grid product, not a persona-pack-only tool.
#[derive(Debug, Clone, PartialEq)]
pub struct Persona {
    /// Surrogate id (UUID v7).
    pub id: PersonaId,
    /// Optional natural key from an external persona pack. Unique when
    /// present; enforced by the repository and by the application service.
    pub pack_id: Option<PackId>,
    /// Display name.
    pub name: String,
    /// Accent colour applied to sidebar entries and card highlights.
    pub accent_color: Option<String>,
    /// Sort order in the sidebar.
    pub display_order: i64,
    /// Archive flag — a sidebar visibility toggle over data that is
    /// still fully live. Distinct from [`trashed_at`](Self::trashed_at):
    /// archiving hides, trashing is the first half of deletion.
    pub archived: bool,
    /// Trash stamp — `None` = live, `Some(t)` = moved to the trash at
    /// `t`, together with every asset that was live under it at that
    /// moment (they carry this same timestamp, which is how restore
    /// puts back exactly that set).
    ///
    /// Carried on the entity rather than fetched on demand because the
    /// write paths need it on every call: an asset must not be created
    /// or restored under a trashed persona, or it would reappear in a
    /// grid whose emptiness depends entirely on those stamps.
    pub trashed_at: Option<DateTime<Utc>>,
    /// When the persona was registered in Asterism.
    pub created_at: DateTime<Utc>,
    /// Last modification timestamp.
    pub updated_at: DateTime<Utc>,
}

impl Persona {
    /// Creates a new persona, rejecting an empty display name.
    pub fn new(name: impl Into<String>, pack_id: Option<PackId>) -> Result<Self, DomainError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::Validation(
                "Persona name must not be empty".into(),
            ));
        }
        let now = Utc::now();
        Ok(Self {
            id: PersonaId::new(),
            pack_id,
            name,
            accent_color: None,
            display_order: 0,
            archived: false,
            trashed_at: None,
            created_at: now,
            updated_at: now,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persona_requires_name() {
        assert!(Persona::new("", None).is_err());
        let p = Persona::new("Alice", Some(PackId::new("alice").unwrap())).unwrap();
        assert!(!p.archived);
    }
}
