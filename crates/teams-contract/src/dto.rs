//! Response DTOs of the `/teams/*` surface.
//!
//! Mutation routes answer with the [`LedgerEventDto`] their write
//! appended — the same-tx rule (#83 §2) means the event *is* the
//! receipt, and a role change carrying old+new in its payload reads on
//! its own (#83 §1).

use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

/// A freshly minted session (`POST /teams/auth/login`).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SessionDto {
    /// The opaque bearer token — present it as
    /// `Authorization: Bearer <token>`. The server stores only its
    /// hash; this response is the one time the value exists in full
    /// outside the client.
    pub token: String,
    /// The account the session resolves to.
    pub user_id: String,
    /// The display name the ledger would stamp for this account.
    pub display_name: String,
    /// Whether this account is an instance admin (#83 §1) — acting
    /// inside a team without a membership row is ledger-stamped as
    /// such, never disguised as a member's action.
    pub admin: bool,
    /// When the session stops resolving, epoch ms.
    pub expires_at_ms: i64,
}

/// The result of `POST /teams/create`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamCreatedDto {
    /// The new team's id — the path segment of every team-scoped route.
    pub team_id: String,
    /// The `teams.team.created/1` event the creation appended.
    pub event: LedgerEventDto,
}

/// The team's current membership set
/// (`GET /teams/{team_id}/roster`).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RosterDto {
    /// The team the roster describes.
    pub team_id: String,
    /// The membership rows, one per member.
    pub members: Vec<RosterMemberDto>,
}

/// One membership row as the roster lists it.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RosterMemberDto {
    /// The member.
    pub user_id: String,
    /// The member's current role: `"owner"` or `"member"`.
    pub role: String,
}

/// One entry of a team's append-only stream
/// (`GET /teams/{team_id}/events`, and the receipt every mutation
/// route returns).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct LedgerEventDto {
    /// Storage-assigned position within the team's stream, from 1.
    pub seq: i64,
    /// Globally unique id of the event.
    pub event_id: String,
    /// The stream the event belongs to.
    pub team_id: String,
    /// `"member"` or `"admin"` — the #83 §1 distinguishability,
    /// carried onto the wire.
    ///
    /// An event written before the capacity was renamed still says
    /// `"admin"` here: the storage row keeps the word it was written
    /// with, because `ledger_event` is append-only, and the domain's
    /// read maps the old tag onto the current one on the way out.
    pub actor_kind: String,
    /// Who acted.
    pub actor_user_id: String,
    /// The actor's display name as it read at write time — a later
    /// rename never rewrites this.
    pub actor_display_name: String,
    /// When, epoch ms.
    pub occurred_at_ms: i64,
    /// The namespaced + versioned kind, e.g.
    /// `"teams.membership.role_changed/1"`.
    pub kind: String,
    /// The typed refs the event makes — what trace queries walk.
    pub subjects: Vec<SubjectRefDto>,
    /// The kind-versioned body, serialised JSON. A role change carries
    /// `{"user_id": …, "old": …, "new": …}` here (#83 §1).
    pub payload_json: String,
}

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

/// One typed reference an event makes.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SubjectRefDto {
    /// The reference's kind: `"digest"`, `"user"`, `"blob"` or
    /// `"forge_identity"`.
    pub ref_type: String,
    /// The reference's value in its storage spelling: digest notation,
    /// a hyphenated UUID, or a forge handle as `"owner"`,
    /// `"unrecorded"`, `"server"` or `"subject:<token>"`.
    pub value: String,
}

/// One page of a team's stream (`GET /teams/{team_id}/events`).
///
/// The stream only grows, so the read is paged rather than whole and
/// the caller is handed where to resume rather than an offset to
/// count from.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct LedgerPageDto {
    /// The events on this page, seq ascending.
    pub events: Vec<LedgerEventDto>,
    /// The seq to pass as `after` for the next page, or `null` when
    /// this page came back shorter than the limit it asked for.
    ///
    /// A page that filled its limit always carries a cursor, even when
    /// it happened to end exactly at the last event there is: whether
    /// anything follows is only answerable by asking. So `null` is the
    /// short page, and it says that nothing lay past here when the
    /// page was taken rather than that nothing ever will — a ledger
    /// has no final page. A caller following a live stream keeps the
    /// last seq it saw and asks again.
    pub next_after: Option<i64>,
}
