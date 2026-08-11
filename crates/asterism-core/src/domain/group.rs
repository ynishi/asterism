//! `Group` — a user-curated set of assets, persona-scoped.
//!
//! Groups are the first primitive we add to the relationship
//! catalogue beyond `Tag` (auto-labelled channel) and
//! `ConstellationEdge` (auto-derived similarity). They exist because
//! Tag alone cannot express **"assets I hand-picked into a bucket"**
//! — a Tag is an organic label that any auto_tag or importer can
//! attach; a Group is a deliberate act of curation ("read later",
//! "mood board for project X", "favorites"). They are also different
//! from `Session` (auto-grouping key from an import run) — a Group
//! is user-authored, not derived.
//!
//! # Naming
//!
//! Domain type: `Group`.
//! Persistence table: `bucket` (SQL reserves `GROUP` as a keyword;
//! quoting the identifier everywhere is fragile so the storage
//! adapter renames the table). The wire DTO and every API surface
//! use the `Group` label — only the DB layer sees `bucket`.

use chrono::{DateTime, Utc};

use crate::domain::value::{DirId, GroupId, PersonaId, SnapshotId};
use crate::error::DomainError;

/// Whether a Group's membership is hand-curated or query-defined.
///
/// - `Manual` — the classic bucket: add / remove / reorder / D&D.
/// - `Query` — membership is the materialised result of the rule in
///   `Group::query_json`; hand edits are rejected at the command layer
///   and the evaluation job owns the `asset_bucket` rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// Hand-curated membership (the default).
    Manual,
    /// Query-defined membership, materialised by the evaluation job.
    Query,
}

impl GroupKind {
    /// The persisted / wire token (`bucket.kind` CHECK values).
    pub fn as_str(&self) -> &'static str {
        match self {
            GroupKind::Manual => "manual",
            GroupKind::Query => "query",
        }
    }

    /// Parses the persisted token; anything but the two CHECK values is
    /// a data error.
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        match s {
            "manual" => Ok(GroupKind::Manual),
            "query" => Ok(GroupKind::Query),
            other => Err(DomainError::Validation(format!(
                "unknown group kind: {other:?}"
            ))),
        }
    }
}

/// A user-created bucket of assets, scoped to one persona.
///
/// `(persona_id, name)` is unique — a persona cannot have two groups
/// with the same name — so the sidebar list stays readable and
/// callers can safely address a group by `(persona, name)` when
/// wiring imports or automation.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// Surrogate id (UUID v7).
    pub id: GroupId,
    /// Persona that owns this group.
    pub persona_id: PersonaId,
    /// Human-facing name. Unique per persona.
    pub name: String,
    /// Optional freeform description (shown in the group panel).
    pub description: Option<String>,
    /// Sidebar folder this group is filed under; `None` = the
    /// persona's root level. Organisation axis only — orthogonal to
    /// the `GroupLink` curation nesting, and never part of the asset
    /// filter semantics.
    pub dir_id: Option<DirId>,
    /// Manual (hand-curated) or Query (rule-defined) membership.
    pub kind: GroupKind,
    /// The `query_json` v1 rule for `kind == Query`; `None` for manual
    /// groups. Opaque here — the typed form is
    /// `asterism_contract::query_group::QueryGroupQuery`.
    pub query_json: Option<String>,
    /// Birth record: the Snapshot this Group was promoted
    /// from, `None` for directly-created groups. Stamped once by the
    /// promote path; feeds the "promoted from" provenance entry that
    /// opens the Snapshot view.
    pub origin_snapshot_id: Option<SnapshotId>,
    /// Outcome of the last query-group refresh (W4-b failure
    /// signal): when the evaluate ran, `"ok"` / `"failed"`, and the
    /// failure text. All `None` for manual groups and for query
    /// groups that never refreshed.
    pub last_refresh_at: Option<DateTime<Utc>>,
    /// `"ok"` / `"failed"` slug of the last refresh; `None` = never
    /// refreshed (see `last_refresh_at`).
    pub last_refresh_status: Option<String>,
    /// Failure text of the last refresh (`None` on success) — the UI
    /// tooltip's error body.
    pub last_refresh_error: Option<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last-updated time (mutates on rename / description edit).
    pub updated_at: DateTime<Utc>,
}

impl Group {
    /// Constructor rejecting an empty (post-trim) name. Builds a
    /// `Manual` group — query groups are minted by the query-group
    /// repository path, which stamps `kind = Query` + the rule.
    pub fn new(
        persona_id: PersonaId,
        name: impl Into<String>,
        description: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::Validation(
                "Group name must not be empty".into(),
            ));
        }
        Ok(Self {
            id: GroupId::new(),
            persona_id,
            name,
            description,
            dir_id: None,
            kind: GroupKind::Manual,
            query_json: None,
            origin_snapshot_id: None,
            last_refresh_at: None,
            last_refresh_status: None,
            last_refresh_error: None,
            created_at: now,
            updated_at: now,
        })
    }
}

/// One group-in-group connection — the Are.na "channel connected into
/// a channel" affordance.
///
/// This is an m:n *connection*, not a parent pointer: the same child
/// group can be connected into many parents, each with its own
/// position, exactly like `asset_bucket` membership. The link graph
/// must stay acyclic (the repository rejects a link that would make
/// the child reach its own ancestor chain) and both ends must belong
/// to the same persona.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupLink {
    /// The containing group.
    pub parent_id: GroupId,
    /// The contained group.
    pub child_id: GroupId,
    /// Hand-arranged order among the parent's child groups.
    pub position: i64,
}

/// A group paired with the number of assets currently linked to it,
/// used to render the sidebar Groups section and chip count badges.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupSummary {
    /// The group itself.
    pub group: Group,
    /// Number of distinct assets currently attached.
    pub asset_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_requires_name() {
        assert!(Group::new(PersonaId::new(), "  ", None, Utc::now()).is_err());
        assert!(Group::new(PersonaId::new(), "Read later", None, Utc::now()).is_ok());
    }

    #[test]
    fn group_trims_are_left_to_the_repository() {
        // The domain constructor rejects blank names but does not
        // itself trim; the storage adapter is expected to canonicalise
        // whitespace so `(persona_id, name)` uniqueness behaves.
        let g = Group::new(PersonaId::new(), "  keep raw  ", None, Utc::now()).unwrap();
        assert_eq!(g.name, "  keep raw  ");
    }
}
