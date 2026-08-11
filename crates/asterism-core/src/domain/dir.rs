//! `Dir` — a persona-scoped folder tree for organising the sidebar.
//!
//! Dir and Group are two deliberately orthogonal mechanisms:
//!
//! - **Dir (organisation axis)** — a strict tree (`parent_id`, at most
//!   one parent) whose only job is to lay out the sidebar. A dir
//!   contains dirs and groups; it never contains assets and it never
//!   filters the grid on the SQL side. Selecting a dir in the UI just
//!   expands to the group ids beneath it and feeds the existing
//!   `group_ids` OR filter.
//! - **Group nesting (curation axis)** — the m:n `bucket_link`
//!   connection (see [`GroupLink`](crate::domain::group::GroupLink)),
//!   which is about what a collection *contains*, Are.na style.
//!
//! Keeping the two apart means the query layer never has to answer
//! "does selecting a parent group include its descendants?" — the tree
//! semantics live entirely in the Dir axis and the client.
//!
//! # Naming
//!
//! `dir` is not a reserved word in SQLite, so unlike `Group`/`bucket`
//! the domain name and the table name coincide.

use chrono::{DateTime, Utc};

use crate::domain::value::{DirId, PersonaId};
use crate::error::DomainError;

/// A sidebar folder, scoped to one persona.
///
/// `(persona_id, parent_id, name)` is unique (root-level dirs are
/// unique per `(persona_id, name)`), so a folder listing never shows
/// two identical siblings.
#[derive(Debug, Clone, PartialEq)]
pub struct Dir {
    /// Surrogate id (UUID v7).
    pub id: DirId,
    /// Persona that owns this dir.
    pub persona_id: PersonaId,
    /// Parent dir; `None` = the persona's root level.
    pub parent_id: Option<DirId>,
    /// Human-facing name. Unique among siblings.
    pub name: String,
    /// Manual sort hint among siblings (ties broken by name). No
    /// reorder API exists yet; the column is reserved so adding one
    /// later is not a schema change.
    pub position: i64,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last-updated time (mutates on rename / move).
    pub updated_at: DateTime<Utc>,
}

impl Dir {
    /// Constructor rejecting an empty (post-trim) name.
    pub fn new(
        persona_id: PersonaId,
        parent_id: Option<DirId>,
        name: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::Validation("Dir name must not be empty".into()));
        }
        Ok(Self {
            id: DirId::new(),
            persona_id,
            parent_id,
            name,
            position: 0,
            created_at: now,
            updated_at: now,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_requires_name() {
        assert!(Dir::new(PersonaId::new(), None, "  ", Utc::now()).is_err());
        assert!(Dir::new(PersonaId::new(), None, "art", Utc::now()).is_ok());
    }
}
