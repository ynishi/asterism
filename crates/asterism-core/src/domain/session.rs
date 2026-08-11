//! `Session` — the Dialog-modality 1st-class aggregate root.
//!
//! Session was previously "an `asset.session_id` GROUP BY projection"
//! (the removed `SessionSummary` shape,
//! [`AssetRepository::list_sessions`](crate::domain::repository::AssetRepository::list_sessions))
//! with two failure modes. First, importers on non-dialogue modalities
//! (tape / journal / image) also wrote `session_id`, so the
//! SessionsView tile grid was flooded with per-file tape rows and
//! per-kind journal buckets that had nothing to do with a Dialog
//! session. Second, the projection carried no per-Session metadata,
//! so the user could not attach a title / note / cover to a run they
//! cared about.
//!
//! P1a (this subtask) makes Session a **1st-class entity** stored in
//! the `session` table with user-editable metadata, scoped to Dialog
//! modality via a DB CHECK (`asset.session_id IS NULL OR modality =
//! 'dialogue'`, added in V27). The non-dialogue `session_id` values
//! are moved to a separate `asset.bundle_id` column (V25) that keeps
//! the constellation-edge grouping intact under its own name — see
//! [`BundleId`](crate::domain::value::BundleId).
//!
//! # Metadata
//!
//! `title` / `note` / `cover_hint` are all `Option<String>` and share
//! the "user-edited free-form text, default absent" contract Group and
//! PersonaProfile use. They are grouped into [`SessionMetadata`] so
//! P1b's HTTP `PATCH …/metadata` handler can carry them as one
//! payload without leaking through the Session's identity fields.
//!
//! # Derived aggregates
//!
//! `started_at_ms` / `ended_at_ms` / `message_count` are aggregates
//! over the participating `asset` rows and stay on the Session row so
//! the SessionsView listing can render without a per-row `GROUP BY`.
//! The `SessionRebuild` job in P1b keeps them in sync; P1a only
//! initialises them at find-or-create time.

use chrono::{DateTime, Utc};

use crate::domain::value::{ExternalSessionKey, PersonaId, SessionId};
use crate::error::DomainError;

/// Per-Session user-editable annotations. Every field is `None` by
/// default; the SessionsView renders the untitled fallback when they
/// are absent.
///
/// The struct is a plain data bundle (no invariants beyond field-level
/// validity). It is the "full state" bundle used at read time; write
/// paths use [`SessionMetadataPatch`] instead so they can carry
/// partial-update semantics (P2 CRUD).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMetadata {
    /// Human-facing title. `None` means "untitled" — the UI falls
    /// back to a first-line snippet or the external key.
    pub title: Option<String>,
    /// Free-form note (user's annotation of the session as a whole).
    pub note: Option<String>,
    /// Cover hint for the SessionsView tile — usually the first
    /// message excerpt captured at import time. `None` while the
    /// hint has not been derived yet.
    pub cover_hint: Option<String>,
}

/// Partial-update payload for
/// [`SessionRepository::update_metadata`](crate::domain::repository::SessionRepository::update_metadata):
/// each field is `None = leave unchanged`, `Some(v) = set to v`. The
/// pattern mirrors [`UpdateModalityCommand`](asterism_contract::command::UpdateModalityCommand)
/// so the HTTP `PATCH` handler and the underlying SQL `COALESCE`
/// stay symmetric.
///
/// Clearing a field back to `NULL` is deliberately **not** expressible
/// through the patch shape (single-level `Option<String>` per field, no
/// `Option<Option<_>>`). The title-specific `rename` path is
/// the canonical clearer for `title`; `note` / `cover_hint` currently
/// have no clearer surface — assign the empty string when a caller
/// wants a "blanked" state (the UI treats both as "no user note").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMetadataPatch {
    /// New title (`Some`) or "leave unchanged" (`None`).
    pub title: Option<String>,
    /// New note (`Some`) or "leave unchanged" (`None`).
    pub note: Option<String>,
    /// New cover hint (`Some`) or "leave unchanged" (`None`).
    pub cover_hint: Option<String>,
}

/// A Dialog-modality session — the aggregate root that owns a run of
/// dialogue assets plus the user's metadata for that run.
///
/// # Invariants
///
/// - `(persona_id, external_key)` is unique across the vault
///   (`UNIQUE(persona_id, external_key)` on the `session` table). The
///   importer relies on this so
///   [`SessionRepository::find_by_external_key`](crate::domain::repository::SessionRepository::find_by_external_key)
///   / `create` is an idempotent find-or-create on re-import.
/// - `started_at_ms <= ended_at_ms` (both stamped from participating
///   asset occurrences). P1a only guarantees this at initialisation
///   time; P1b's `SessionRebuild` maintains it as the membership
///   changes.
/// - Session may be deleted only when no `asset` row references it —
///   see [`SessionRepository::delete_if_empty`](crate::domain::repository::SessionRepository::delete_if_empty).
///   Orphaning delete is forbidden (mirrors the Modality delete
///   guard).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Surrogate id (stored as a hyphenated UUID v7 string in the
    /// `session.id TEXT PRIMARY KEY` column). Reused verbatim as the
    /// value written into `asset.session_id` after V26 so the
    /// dialogue Assets and the Session row share the same key.
    pub id: SessionId,
    /// Owning persona (`(persona_id, external_key)` is the unique
    /// natural key).
    pub persona_id: PersonaId,
    /// The raw session identifier supplied by the importer. Used by
    /// the find-or-create path to converge re-imports onto the same
    /// row; not surfaced on the UI (users see the metadata title
    /// instead).
    pub external_key: ExternalSessionKey,
    /// User-editable metadata (title / note / cover hint). Kept in a
    /// separate struct so the P1b HTTP `PATCH …/metadata` handler can
    /// carry the whole bundle as one payload.
    pub metadata: SessionMetadata,
    /// Earliest occurrence time among participating assets. Derived —
    /// P1a stamps the migration-time value, P1b's `SessionRebuild`
    /// keeps it in sync.
    pub started_at_ms: i64,
    /// Latest occurrence time among participating assets. Derived,
    /// same lifecycle as `started_at_ms`.
    pub ended_at_ms: i64,
    /// Count of participating assets (dialogue only). Derived; the
    /// SessionsView tile chip reads this directly rather than joining
    /// against `asset`.
    pub message_count: u64,
    /// Creation time (unix epoch ms).
    pub created_at_ms: i64,
    /// Last modification time (unix epoch ms). Bumped by metadata
    /// edits (P1b); not touched by the `SessionRebuild` telemetry
    /// stamp so a title edit and a background aggregate refresh are
    /// distinguishable.
    pub updated_at_ms: i64,
}

impl Session {
    /// Builds a new Session at find-or-create time. `id` is minted by
    /// the application layer (`SessionService::find_or_create_by_external_key`)
    /// as a fresh UUID v7 in string form. `started_at_ms` /
    /// `ended_at_ms` / `message_count` are set from the participating
    /// assets by the caller (P1b) — the constructor takes them
    /// explicitly so the migration path can seed them from the
    /// `MIN(occurred_at)` / `MAX(occurred_at)` / `COUNT(*)` scan the
    /// V26 batch runs.
    ///
    /// `now` is stamped into both `created_at_ms` and `updated_at_ms`
    /// so the initial row is coherent without a follow-up touch.
    pub fn new(
        id: SessionId,
        persona_id: PersonaId,
        external_key: ExternalSessionKey,
        started_at_ms: i64,
        ended_at_ms: i64,
        message_count: u64,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        if started_at_ms > ended_at_ms {
            return Err(DomainError::Validation(format!(
                "Session temporal window must be monotonic: \
                 started_at_ms={started_at_ms} > ended_at_ms={ended_at_ms}"
            )));
        }
        let now_ms = now.timestamp_millis();
        Ok(Self {
            id,
            persona_id,
            external_key,
            metadata: SessionMetadata::default(),
            started_at_ms,
            ended_at_ms,
            message_count,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ExternalSessionKey {
        ExternalSessionKey::new("cc.session.42").unwrap()
    }

    #[test]
    fn new_stamps_now_into_both_timestamps_and_starts_untitled() {
        let now = Utc::now();
        let s = Session::new(
            SessionId::new("018f8e57-1234-7abc-9def-0123456789ab").unwrap(),
            PersonaId::new(),
            key(),
            10,
            20,
            3,
            now,
        )
        .unwrap();
        assert_eq!(s.created_at_ms, now.timestamp_millis());
        assert_eq!(s.updated_at_ms, now.timestamp_millis());
        assert_eq!(s.metadata, SessionMetadata::default());
        assert_eq!(s.message_count, 3);
    }

    #[test]
    fn new_rejects_reversed_temporal_window() {
        let err = Session::new(
            SessionId::new("018f8e57-1234-7abc-9def-000000000001").unwrap(),
            PersonaId::new(),
            key(),
            100,
            50,
            1,
            Utc::now(),
        );
        assert!(matches!(err, Err(DomainError::Validation(_))));
    }

    #[test]
    fn zero_message_count_is_allowed_at_create_time() {
        // Migration can materialise a Session before its assets have
        // moved onto it (V26 orders session-insert-then-asset-update);
        // the initial row must be legal with `message_count = 0`.
        let s = Session::new(
            SessionId::new("018f8e57-1234-7abc-9def-000000000002").unwrap(),
            PersonaId::new(),
            key(),
            0,
            0,
            0,
            Utc::now(),
        )
        .unwrap();
        assert_eq!(s.message_count, 0);
    }
}
