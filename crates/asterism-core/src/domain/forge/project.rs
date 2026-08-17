//! `Project` — the repo of the forge's git analogy (#63): the shared
//! context pursuits file under, and the owner of a mainline.
//!
//! The pursuit answers "one line of work"; the project answers "one
//! body of work" — the unit a mainline is canonical *for*. Without it
//! the mainline would be a persona-wide singleton, and two unrelated
//! efforts would fight over one namespace of living names. With it,
//! scope has a referent (an `In(Existing)` of another project's living
//! asset is cross-project contamination, catchable at IN time) and
//! "the canonical set" means canonical for something in particular.
//!
//! # Shape
//!
//! - [`Project`] is a thin, immutable row: identity, persona, a
//!   required human name, an optional note. No status column, no
//!   members — what belongs to a project derives through its pursuits
//!   and its mainline, never from a membership table.
//! - [`Mainline`](crate::domain::forge::mainline::Mainline) rows are
//!   the project's lines. v1 mints exactly one, named
//!   [`MAIN`](crate::domain::forge::mainline::Mainline::MAIN), in the
//!   same transaction as the project (application-enforced); the
//!   schema admits more so a later multi-line model is an enum's
//!   worth of change, not a migration.
//!
//! # Invariants (service-enforced, entity-checked where local)
//!
//! - `name` is non-blank (checked here); uniqueness among one
//!   persona's projects is an application rule, checked where both
//!   rows are visible — like living-name uniqueness on a mainline,
//!   and unlike a schema UNIQUE, so a later archival verb can free
//!   names without a migration.
//! - A pursuit filing under a project shares its persona
//!   (cross-aggregate, application service — the persona cascade
//!   rule every forge pairing states).

use chrono::{DateTime, Utc};

use crate::domain::attribution::{AttributionContext, PersistedAttribution};
use crate::domain::value::{PersonaId, ProjectId};
use crate::error::DomainError;

/// Trims an optional human label; whitespace-only collapses to `None`
/// so "no note" has one representation in storage.
fn normalized(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The repo above the pursuits. Thin and immutable: identity plus a
/// human name — never content, never status.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// Surrogate id (UUID v7) — minted, never derived from content.
    pub id: ProjectId,
    /// Persona bucket; the mainline, its entries, and every filed
    /// pursuit share it (service-enforced).
    pub persona_id: PersonaId,
    /// Required human name. Uniqueness among the persona's projects
    /// is an application rule, not a schema constraint.
    pub name: String,
    /// One short free-text slot.
    pub note: Option<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Who opened the project. Private as a triple, like
    /// [`Pursuit`](crate::domain::forge::pursuit::Pursuit)'s: set
    /// whole from the context at construction, restored whole by
    /// [`from_persisted`](Self::from_persisted).
    operator_ai: Option<crate::domain::attribution::OperatorRef>,
    author: Option<crate::domain::attribution::Author>,
    attributed_via: Option<crate::domain::attribution::AttributionChannel>,
}

impl Project {
    /// Builds a fresh project. The name is required and non-blank —
    /// a project exists to be named, unlike a pursuit, whose title is
    /// optional intent.
    pub fn new(
        persona_id: PersonaId,
        name: String,
        note: Option<String>,
        now: DateTime<Utc>,
        attribution: &AttributionContext,
    ) -> Result<Self, DomainError> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(DomainError::Validation(
                "project name must not be blank".into(),
            ));
        }
        Ok(Self {
            id: ProjectId::new(),
            persona_id,
            name,
            note: normalized(note),
            created_at: now,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        })
    }

    /// Read-path twin of [`new`](Self::new): restores a stored row as
    /// a fact rather than a request to accept.
    pub fn from_persisted(
        id: ProjectId,
        persona_id: PersonaId,
        name: String,
        note: Option<String>,
        created_at: DateTime<Utc>,
        attribution: PersistedAttribution,
    ) -> Self {
        Self {
            id,
            persona_id,
            name,
            note,
            created_at,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        }
    }

    /// Subject that opened this project (`None` = unrecorded).
    pub fn author(&self) -> Option<&crate::domain::attribution::Author> {
        self.author.as_ref()
    }

    /// Agent that opened this project (`None` = unrecorded).
    pub fn operator_ai(&self) -> Option<&crate::domain::attribution::OperatorRef> {
        self.operator_ai.as_ref()
    }

    /// Channel the pair above arrived through (`None` = unrecorded).
    pub fn attributed_via(&self) -> Option<crate::domain::attribution::AttributionChannel> {
        self.attributed_via
    }

    /// Hands the triple back out whole, for the same reason
    /// `Pursuit` does: the only assemblable form is a recorded fact,
    /// not a mintable one.
    pub fn persisted_attribution(&self) -> PersistedAttribution {
        PersistedAttribution::recorded(
            self.author.clone(),
            self.operator_ai.clone(),
            self.attributed_via,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> AttributionContext {
        AttributionContext::owner_surface()
    }

    #[test]
    fn a_blank_name_is_refused_and_a_padded_one_is_trimmed() {
        let persona = PersonaId::new();
        assert!(Project::new(persona, "   ".into(), None, Utc::now(), &context()).is_err());

        let project = Project::new(
            persona,
            "  key visuals  ".into(),
            None,
            Utc::now(),
            &context(),
        )
        .unwrap();
        assert_eq!(project.name, "key visuals");
    }

    #[test]
    fn a_whitespace_note_collapses_to_none() {
        let project = Project::new(
            PersonaId::new(),
            "album".into(),
            Some("  ".into()),
            Utc::now(),
            &context(),
        )
        .unwrap();
        assert_eq!(project.note, None);
    }
}
