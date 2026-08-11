//! `Snapshot` — an immutable, content-addressed freeze of an ordered
//! asset set (a git-tree analogue).
//!
//! A Snapshot is a *pure content object*: it knows its ordered members
//! and the hash of that membership, and nothing else. Where it came from
//! (a dispatch, a promote) is recorded on the referencing event
//! (`dispatch_job` / `bucket.origin_snapshot_id`), never on the Snapshot
//! itself — that is what lets two producers that happen to freeze the
//! same members in the same order share one row without their provenance
//! bleeding together.
//!
//! # Invariants
//!
//! - `asset_ids` is non-empty (a freeze with no members is meaningless —
//!   [`Snapshot::new`] rejects it).
//! - `content_hash` is derived from `asset_ids` in frozen order via
//!   [`content_hash`](crate::domain::snapshot_hash::content_hash); order
//!   is part of the identity, so the same set in a different order is a
//!   different Snapshot.
//! - `asset_ids` is a *snapshot* — if the underlying assets are later
//!   deleted the row retains its ids; dispatch runs validate freshness
//!   when they materialise the input set.
//! - Every asset belongs to `persona_id` (enforced by the application
//!   service at construction; the entity only knows the ids).

use chrono::{DateTime, Utc};

use crate::domain::snapshot_hash::content_hash;
use crate::domain::value::{AssetId, PersonaId, SnapshotId};
use crate::error::DomainError;

/// An immutable content-addressed freeze of an ordered asset set.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Surrogate id (UUID v7).
    pub id: SnapshotId,
    /// Persona the Snapshot belongs to. All `asset_ids` share this
    /// persona (enforced by the application service). Dedupe scope is
    /// `(persona_id, content_hash)`.
    pub persona_id: PersonaId,
    /// Canonical fingerprint of the ordered `asset_ids` (see
    /// [`content_hash`](crate::domain::snapshot_hash::content_hash)).
    pub content_hash: String,
    /// Frozen list of asset ids in freeze order. Non-empty by invariant.
    pub asset_ids: Vec<AssetId>,
    /// Creation time of the canonical row.
    pub created_at: DateTime<Utc>,
}

impl Snapshot {
    /// Builds a new Snapshot, computing its `content_hash` from the
    /// ordered members. Rejects an empty `asset_ids`.
    ///
    /// The returned entity carries a fresh id; a producer hands it to
    /// [`SnapshotRepository::create_or_reuse`](crate::domain::repository::SnapshotRepository::create_or_reuse),
    /// which either persists it or returns the pre-existing row that
    /// shares its `(persona_id, content_hash)`.
    pub fn new(
        persona_id: PersonaId,
        asset_ids: Vec<AssetId>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        if asset_ids.is_empty() {
            return Err(DomainError::Validation(
                "Snapshot must contain at least one asset".into(),
            ));
        }
        let id_strings: Vec<String> = asset_ids.iter().map(|a| a.to_string()).collect();
        let hash = content_hash(id_strings.iter().map(String::as_str));
        Ok(Self {
            id: SnapshotId::new(),
            persona_id,
            content_hash: hash,
            asset_ids,
            created_at: now,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_asset_list() {
        let persona = PersonaId::new();
        let now = Utc::now();
        assert!(Snapshot::new(persona, vec![], now).is_err());
    }

    #[test]
    fn accepts_single_asset_and_hashes_members() {
        let persona = PersonaId::new();
        let now = Utc::now();
        let asset = AssetId::new();
        let snap = Snapshot::new(persona, vec![asset], now).unwrap();
        assert_eq!(snap.asset_ids, vec![asset]);
        assert_eq!(snap.persona_id, persona);
        assert_eq!(snap.content_hash.len(), 64, "sha-256 hex fingerprint");
    }

    #[test]
    fn order_is_part_of_the_content_hash() {
        let persona = PersonaId::new();
        let now = Utc::now();
        let a = AssetId::new();
        let b = AssetId::new();
        let ab = Snapshot::new(persona, vec![a, b], now).unwrap();
        let ba = Snapshot::new(persona, vec![b, a], now).unwrap();
        assert_ne!(
            ab.content_hash, ba.content_hash,
            "the same members in a different order are a different snapshot"
        );
    }
}
