//! `Tag` — the channel entity (a classification axis shared across
//! personas).
//!
//! The pipeline is `Keyword` (raw auto-tag output on an asset) → `Tag`
//! (materialised channel). The `asset_tag` many-to-many table is the source
//! of truth for the relationship; a persona-to-tag join is derived through
//! it. A dedicated `persona_tag` table is deferred until sidebar pinning
//! demands it.

use crate::domain::value::{ChannelAxis, TagId};
use crate::error::DomainError;

/// A single channel, shared across personas.
///
/// `name` is unique; enforcement happens in `TagRepository::find_or_create`
/// and via a DB unique constraint. A separate `Category` entity is
/// intentionally omitted — extensions can go through `axis` variants.
#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    /// Surrogate id (UUID v7).
    pub id: TagId,
    /// Channel name (unique).
    pub name: String,
    /// Optional classification axis (may be assigned later).
    pub axis: Option<ChannelAxis>,
}

impl Tag {
    /// Creates a new tag, rejecting an empty name.
    pub fn new(name: impl Into<String>) -> Result<Self, DomainError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::Validation("Tag name must not be empty".into()));
        }
        Ok(Self {
            id: TagId::new(),
            name,
            axis: None,
        })
    }
}

/// A tag paired with the number of assets currently linked to it.
///
/// Used to render the sidebar Tags section and the count badges on
/// tag chips. Aggregated by [`TagRepository::tag_counts`] with an
/// optional persona scope so the sidebar reflects the active persona
/// filter without pulling every asset over the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct TagCount {
    /// The tag itself (id + name + optional axis).
    pub tag: Tag,
    /// Number of distinct assets attached to `tag` within the query
    /// scope (persona-filtered when the caller passes a persona id,
    /// global otherwise).
    pub asset_count: u64,
}

/// Outcome of folding one tag into another
/// ([`TagRepository::merge`](crate::domain::repository::TagRepository::merge)).
///
/// The two counts partition the source's links: an asset either did
/// not carry the target (its link *moves*) or already did (its
/// duplicate link is *dropped*). Both count `asset_tag` rows, so
/// trashed assets are included — their links survive trashing by
/// design, and the merge has to deal with them either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagMergeOutcome {
    /// Links moved onto the target.
    pub affected_assets: u64,
    /// Links dropped because the asset already carried the target.
    pub already_tagged: u64,
    /// Whether the source row was deleted (`false` on a dry run).
    pub source_removed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_requires_name() {
        assert!(Tag::new(" ").is_err());
        assert!(Tag::new("release-planning").is_ok());
    }
}
