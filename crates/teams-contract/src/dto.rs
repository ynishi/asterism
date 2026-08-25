//! Response DTOs of the `/teams/*` surfaces a member's client does not
//! speak.
//!
//! Mutation routes answer with the [`LedgerEventDto`] their write
//! appended — the same-tx rule (#83 §2) means the event *is* the
//! receipt, and a role change carrying old+new in its payload reads on
//! its own (#83 §1). That envelope now lives in `asterism-teams-wire`, along
//! with the session, the roster, the ledger page and the content
//! verbs' answers: a member's client reads all of those and may not
//! link this crate (#148 decision 15). It is named here rather than
//! re-spelled, which is the whole point of a leaf both planes depend
//! on.

use asterism_teams_wire::dto::LedgerEventDto;
use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

/// The result of `PUT /teams/{team_id}/blobs?digest=…`.
///
/// Deliberately identical for a first copy and a deduplicated one: the
/// CAS holding the digest already is server-side knowledge only, and a
/// response that said "skipped" would be the Harnik-2010 dedupe side
/// channel (#83 §3). What the caller learns is what changed *for their
/// team*: the link now exists, and here is the event that recorded it.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct BlobUploadedDto {
    /// The verified digest — declared and computed, now equal by
    /// construction — under which the blob is addressable at
    /// `GET /teams/{team_id}/blobs/{digest}`.
    pub digest: String,
    /// The `teams.blob.copy_completed/1` event the upload appended, in
    /// the same transaction as the link row (#83 §3 ordering).
    pub event: LedgerEventDto,
}

/// The result of `POST /teams/{team_id}/blobs/purge/reclaim` (#95).
///
/// Mark and unmark answer with their [`LedgerEventDto`] alone (the
/// receipt convention); reclaim gets a shape of its own because one
/// call removes many links and triggers the zero-link sweep behind it.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PurgeReclaimedDto {
    /// The digests whose links this reclaim removed — exactly the
    /// marked links whose grace window had elapsed; marks still inside
    /// their window stay marked for a later reclaim.
    pub removed_digests: Vec<String>,
    /// How many blobs the post-reclaim zero-link sweep deleted the
    /// bytes of. A **count, deliberately not a list**: the sweep is
    /// instance-wide (registry-GC shape, #83 §3 — it may also collect
    /// orphans, or another team's leftovers from an earlier failed
    /// sweep), and digest values a caller's team never linked must not
    /// cross the team boundary on a surface that otherwise treats
    /// digest existence as protected. Not necessarily equal to
    /// `removed_digests.len()`: a digest still linked in another team
    /// keeps its bytes.
    pub swept: u64,
    /// The `teams.blob_link.reclaimed/1` event the reclaim appended —
    /// its payload carries every removed digest and the window they
    /// waited out.
    pub event: LedgerEventDto,
}

/// The team's marked-for-purge set
/// (`GET /teams/{team_id}/blobs/purge/marked`, #95 — owner or admin,
/// the mark's own authority).
///
/// This is the read surface the grace-visibility boundary (#83 §3
/// [Grace visibility]) *grants*: the mark hides a link from the
/// normal reads, but inside the team it is sayable state — and whoever
/// may mark must be able to see what is marked, or unmark is a verb
/// aimed blind.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MarkedBlobsDto {
    /// The team whose marked set this is.
    pub team_id: String,
    /// The marked links, one row each.
    pub marked: Vec<MarkedBlobLinkDto>,
}

/// One marked link — everything unmark (or a decision to let reclaim
/// run) needs.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MarkedBlobLinkDto {
    /// The marked digest — what
    /// `POST …/blobs/{digest}/purge/unmark` takes.
    pub digest: String,
    /// When the mark landed, epoch ms.
    pub marked_at_ms: i64,
    /// When the grace window elapses and reclaim may remove this link
    /// (`marked_at_ms` + the instance's grace window), epoch ms.
    pub reclaimable_at_ms: i64,
}

/// The receipt of `PUT /teams/heads/registry` (#132 phase 3) — the
/// envelope facts the instance validated, never the artifact body,
/// which the carrier does not read.
///
/// No [`LedgerEventDto`] here, unlike every team-scoped mutation: the
/// registry is instance-scope, outside the ledger's per-team streams
/// (#83 §2); its history is the storage's own superseded rows.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct HeadPublishedDto {
    /// The published head's label.
    pub label: String,
    /// When the publish landed (and any predecessor was superseded),
    /// epoch ms.
    pub published_at_ms: i64,
}
