//! Response shapes of the `/teams/*` routes a member's client reads.
//!
//! Mutation routes answer with the [`LedgerEventDto`] their write
//! appended — the same-tx rule (#83 §2) means the event *is* the
//! receipt, and a role change carrying old+new in its payload reads on
//! its own (#83 §1).

use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

/// A freshly minted session (`POST /teams/auth/login`).
///
/// **No derived `Debug`** — see the hand-written one below. This is
/// the one shape in this crate carrying a live credential.
#[derive(Clone, Serialize, Deserialize, SchemaBridge)]
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

/// Prints everything about a session except the value that would let
/// the reader use it.
///
/// The derived one would put the bearer token into every panic
/// message, test failure and log line that ever formats a session —
/// which is how a credential ends up somewhere nobody meant to put it,
/// copied out by someone who was looking at something else. The token
/// travels in the response and in an `Authorization` header, and
/// nowhere a human reads.
impl std::fmt::Debug for SessionDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionDto")
            .field("token", &"<not shown>")
            .field("user_id", &self.user_id)
            .field("display_name", &self.display_name)
            .field("admin", &self.admin)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
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

/// The teams the caller is a member of (`GET /teams`).
///
/// The roster read turned around: that one takes a team and answers
/// with users, this one takes the caller and answers with teams. It is
/// the only team read that names no team in its path, because the
/// question is not about one.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MyTeamsDto {
    /// One row per membership the caller holds, ordered oldest team
    /// first — the order the teams were created in, which is the only
    /// order the rows carry a fact for.
    pub teams: Vec<MyTeamDto>,
}

/// One team the caller belongs to.
///
/// **No name, because a team has none.** The `team` table is an id and
/// a creation time; nothing in the model, the ledger's kinds or this
/// wire carries a label for one. So a picker over these rows shows
/// ids, the way the roster shows user ids and says why. Naming a team
/// is a change to the model rather than a field this DTO is missing.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MyTeamDto {
    /// The team, and the path segment of every team-scoped route.
    pub team_id: String,
    /// What the caller is in it: `"owner"` or `"member"`.
    ///
    /// The one fact beyond the id that distinguishes one row from
    /// another today, and it is free — the membership row the query
    /// reads is where it lives.
    pub role: String,
    /// When the team was created, unix epoch milliseconds.
    pub created_at_ms: i64,
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

/// What the team minted for content that entered it
/// (`PUT /teams/{team_id}/forge/pursuits/{id}/content?digest=…`).
///
/// The asset id is the team's own surrogate and never a local one
/// (#148 decision 6): a client keeps the correspondence on its own
/// machine, and reading this as a local `AssetId` is the one thing
/// that boundary forbids.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ContentEnteredDto {
    /// The `TeamAsset` this promotion minted. One per promotion, so
    /// two members bringing identical bytes get one each (#148
    /// decision 7) — this is the id a round names as content.
    pub asset_id: String,
    /// The digest the bytes hashed to, as the server verified it.
    pub digest: String,
    /// The open work the content entered against (#148 decision 5).
    pub pursuit_id: String,
    /// The `forge.content.entered/1` event the write appended, in the
    /// same transaction as the rows (#148 decision 17).
    pub event: LedgerEventDto,
}

/// One asset a team holds, as the bulk resolve answers about it.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct HeldAssetDto {
    /// The team's surrogate for it.
    pub asset_id: String,
    /// What it was converted from, when the conversion was one blob —
    /// which is the whole of v0. Absent for a conversion composed some
    /// other way (#148 decision 3), which is a shape the schema admits
    /// and nothing writes yet.
    pub digest: Option<String>,
    /// The work the content entered against, when the row records one.
    pub entered_for_pursuit_id: Option<String>,
    /// When the team minted it, epoch ms.
    pub created_at_ms: i64,
}

/// The bulk resolve's answer
/// (`POST /teams/{team_id}/forge/content/resolve`).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ResolvedContentDto {
    /// The asked-about ids this team holds, with what each carries.
    pub held: Vec<HeldAssetDto>,
    /// The asked-about ids it does not — which includes ids another
    /// team holds, because an id outside this team reads as absent
    /// here and a caller learns nothing else about it.
    pub unknown: Vec<String>,
}

/// The have-check's answer
/// (`POST /teams/{team_id}/forge/content/have`).
///
/// The digests this team holds, and nothing about the rest: the
/// question is what a client may skip sending, and an answer shaped as
/// "held / not held" per digest is one a caller can line up against
/// the wrong list.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct HeldContentDto {
    /// Which of the asked digests are in this team's store now.
    ///
    /// A digest marked for purge is **not** here (#95): a client told
    /// it could skip a send for bytes a reclaim is about to take would
    /// have skipped it wrongly.
    pub held: Vec<String>,
}
