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
    /// Whether this account is the instance operator (#83 §1) — acting
    /// inside a team without a membership row is ledger-stamped as
    /// such, never disguised as a member's action.
    pub operator: bool,
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
    /// `"member"` or `"operator"` — the #83 §1 distinguishability,
    /// carried onto the wire.
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

/// One typed reference an event makes.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SubjectRefDto {
    /// The reference's kind: `"digest"`, `"user"`, `"blob"`, or the
    /// reserved `"forge_identity"`.
    pub ref_type: String,
    /// The reference's value in its storage spelling (digest notation,
    /// hyphenated UUID, or the opaque forge identity).
    pub value: String,
}
