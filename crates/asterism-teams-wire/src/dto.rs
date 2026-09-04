//! Response shapes of the `/teams/*` routes a member's client reads —
//! and, since #213, of the admin's routes under `/teams/admin`, which
//! live here because a shape's crate is the answer to "does a client
//! say this" for whichever client comes to say it.
//!
//! A team's mutation routes answer with the [`LedgerEventDto`] their
//! write appended — the same-tx rule (#83 §2) means the event *is* the
//! receipt, and a role change carrying old+new in its payload reads on
//! its own (#83 §1). The admin's account verbs answer `204` and land
//! in the instance's own record instead ([`AccountEventDto`]), for the
//! reason its migration gives.

use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

/// A freshly minted session (`POST /teams/auth/login`,
/// `POST /teams/auth/device/login`, and the collect of a sign-in
/// through the provider — one session shape, whichever arm issued
/// it).
///
/// **No derived `Debug`** — see the hand-written one below, which is
/// where this crate's rule about formatting a live credential is
/// stated.
#[derive(Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SessionDto {
    /// The opaque bearer token — present it as
    /// `Authorization: Bearer <token>`. The server stores only its
    /// hash; this response is the one time the value exists in full
    /// outside the client.
    pub token: String,
    /// The account the session resolves to.
    pub user_id: String,
    /// The login name the account was created under — the one field
    /// here a person did not necessarily type: a sign-in through a
    /// provider (#163) ends in a session for an account whose login the
    /// provider never heard of, so the session says it. Defaulted on
    /// decode, so a client reads a server from before the field as one
    /// that answered with a blank login rather than as one that
    /// answered nothing.
    #[serde(default)]
    pub login: String,
    /// The display name the ledger would stamp for this account.
    pub display_name: String,
    /// Whether this account is an instance admin (#83 §1) — acting
    /// inside a team without a membership row is ledger-stamped as
    /// such, never disguised as a member's action.
    pub admin: bool,
    /// When the session stops resolving, epoch ms.
    pub expires_at_ms: i64,
    /// The instance's stable id (#163) — what a client is to key what
    /// it stores about this server by, because the URL it connected to
    /// is a name that moves and this is not. Opaque; the same for
    /// every session the instance ever mints. Defaulted on decode for
    /// the reason `login` is.
    #[serde(default)]
    pub instance_id: String,
    /// The tenant this session belongs to, as an opaque id (#163).
    /// One instance hosts one tenant today, and the value is the
    /// instance's; it is here from the first so that a host serving
    /// several tenants later changes nothing a client stores or
    /// compares. Defaulted on decode for the reason `login` is.
    #[serde(default)]
    pub tenant_id: String,
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
            .field("login", &self.login)
            .field("display_name", &self.display_name)
            .field("admin", &self.admin)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("instance_id", &self.instance_id)
            .field("tenant_id", &self.tenant_id)
            .finish()
    }
}

/// What this instance offers besides a password
/// (`GET /teams/auth/providers`, #163). Public — it is what a connect
/// form reads before anybody has signed in, to know whether to offer
/// a provider button at all.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AuthProvidersDto {
    /// The identity provider this instance signs people in through,
    /// or `None` for an instance that verifies passwords and nothing
    /// else.
    pub oidc: Option<OidcProviderDto>,
}

/// One identity provider, as a connect form names it.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct OidcProviderDto {
    /// What to call it on the button — whatever the person hosting
    /// wrote when configuring the instance.
    pub name: String,
}

/// A sign-in attempt through the provider
/// (`POST /teams/auth/oidc/attempts`, #163): where to send the browser,
/// and how long the attempt is good for.
///
/// Nothing here is a credential. The attempt id is the `state` the
/// provider echoes and what the start page is keyed by; presenting it
/// collects nothing without the secret the app kept and the grant the
/// browser brings back.
///
/// What comes back to the app comes through its loopback listener,
/// not through this crate. The contract that listener is to meet: the
/// provider's callback sends the browser to
/// `http://127.0.0.1:<port>/teams/auth/oidc/loopback?attempt=<id>`
/// with either `&grant=<grant>` or `&refused=1`, and the listener is
/// to answer with a `303` to `<start_url>/done`, where the instance
/// says what happened. The collect route then answers with an
/// ordinary [`SessionDto`]: a `401` for an attempt that was refused
/// (to the secret alone — there was no grant), a `404` for one nothing
/// names, past its expiry, already collected, not yet finished in the
/// browser, or whose secret or grant is not its own.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct OidcAttemptDto {
    /// The attempt's id, which the collect route takes and the loopback
    /// redirect names.
    pub attempt_id: String,
    /// The page to open in the system browser: this instance's own,
    /// which shows the device label and asks before sending the person
    /// on to the provider.
    pub start_url: String,
    /// When the attempt stops being collectable, epoch ms on the
    /// instance's clock. What a client derives a wait from when it
    /// meets an instance too old to say `ttl_ms`, which says why that
    /// is not the way.
    pub expires_at_ms: i64,
    /// How long the attempt is collectable from now, ms: what the
    /// listener waits from, up to a ceiling of its own. A duration
    /// rather than the instant above because a client subtracting its
    /// own clock from the instance's gets the difference between two
    /// clocks, and a clock fast by more than the attempt's life ended
    /// the wait before the browser could come back. `0` from an
    /// instance older than the field, which stated only the instant.
    #[serde(default)]
    pub ttl_ms: i64,
}

/// A freshly minted device token (`POST /teams/auth/device`, #204).
///
/// **No derived `Debug`**, for the reason [`SessionDto`]'s
/// hand-written one gives. The difference worth stating here is how
/// long the value is useful for: a session dies in hours, this is the
/// credential a client puts in an OS keychain and presents after a
/// restart, so a copy of it that leaked into a log is a copy that
/// works for months.
///
/// Carries no instance or tenant id: the session that asked for this
/// token said both, and the session the token is later presented for
/// says them again, so what a client keys its store by is never
/// missing at the moment it writes.
#[derive(Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeviceTokenMintedDto {
    /// The token. **This response is the only time it exists outside
    /// the client** — the server stores its SHA-256, so nothing can
    /// answer with it again and a client that loses it mints another.
    pub token: String,
    /// The handle the listing names this token by and the revoke takes
    /// (`DELETE /teams/auth/device/{id}`). Safe to store beside the
    /// token or in place of it: it authenticates nobody.
    pub id: String,
    /// When it stops resolving, epoch ms.
    pub expires_at_ms: i64,
}

impl std::fmt::Debug for DeviceTokenMintedDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceTokenMintedDto")
            .field("token", &"<not shown>")
            .field("id", &self.id)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// The caller's own device tokens (`GET /teams/auth/device`, #204) —
/// or, on the admin's route (`GET /teams/admin/accounts/{user_id}/devices`,
/// #213), another account's, in the same shape and with the same
/// absence: no value, no digest, on either.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeviceTokensDto {
    /// One row per live token, oldest mint first.
    pub tokens: Vec<DeviceTokenDto>,
}

/// One act on an account, from the instance's record (#213): who did
/// what to whom, and when. Why the record is its own table and not
/// the ledger is the migration's to say (`teams-infra`'s V13); the
/// actor is stamped the way a ledger actor is, by id and by the name
/// they had at the time.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AccountEventDto {
    /// The row's place in the record; ascending is the order the acts
    /// happened in.
    pub seq: i64,
    /// When, epoch ms.
    pub occurred_at_ms: i64,
    /// The admin who acted.
    pub actor_user_id: String,
    /// The name they had at the time.
    pub actor_name: String,
    /// The account acted on, or absent for an act on every account
    /// at once.
    pub subject_user_id: Option<String>,
    /// What was done: `locked`, `unlocked`, or `devices_revoked`.
    pub kind: String,
}

/// The instance's record of acts on accounts (#213), whole
/// (`GET /teams/admin/events`) or for one account
/// (`GET /teams/admin/accounts/{user_id}/events`), oldest first. An
/// account's page includes the acts on every account, which reached
/// it too.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AccountEventsDto {
    /// When the account was locked, epoch ms, for one account's page
    /// while it is; absent otherwise, and always absent on the whole
    /// record's page.
    pub locked_at_ms: Option<i64>,
    /// The acts.
    pub events: Vec<AccountEventDto>,
}

/// One device token as its owner sees it.
///
/// **`Debug` is derived, and that is the property worth checking:**
/// nothing on this shape authenticates anybody — not the token, not
/// its hash, not a prefix of either — which is what lets the listing
/// exist at all.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeviceTokenDto {
    /// The handle to revoke by.
    pub id: String,
    /// What the client called this device when it asked.
    pub label: String,
    /// When it was minted, epoch ms.
    pub created_at_ms: i64,
    /// When it was last presented, epoch ms — absent for a token
    /// nobody has used yet, which is a different fact from "used at
    /// the moment it was made".
    pub last_used_at_ms: Option<i64>,
    /// When it stops resolving, epoch ms.
    pub expires_at_ms: i64,
}

/// The result of `POST /teams/create`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamCreatedDto {
    /// The new team's id — the path segment of every team-scoped route.
    pub team_id: String,
    /// The name it was founded with (#218).
    pub name: String,
    /// The `teams.team.created/1` event the creation appended.
    pub event: LedgerEventDto,
}

/// The result of `POST /teams/{team_id}/rename` (#218).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RenamedTeamDto {
    /// The team that was renamed.
    pub team_id: String,
    /// Its name now.
    pub name: String,
}

/// The team's current membership set, and what the caller may do in
/// it (`GET /teams/{team_id}/roster`).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RosterDto {
    /// The team the roster describes.
    pub team_id: String,
    /// The membership rows, one per member.
    pub members: Vec<RosterMemberDto>,
    /// What the caller may do here, said by the gate that already
    /// worked it out rather than derived from the rows (#210).
    ///
    /// A reader cannot get this from `members` alone. An instance
    /// admin reaches a team by standing outside it (#83 §1), so they
    /// hold no row — and a client deriving a role from rows reads
    /// their absence as "nothing you may do" when what they may do is
    /// delete the team. This field is the gate's own answer to the
    /// question the rows cannot be asked.
    pub viewer: ViewerDto,
}

/// The caller's standing in the team whose roster this is.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ViewerDto {
    /// The caller's role, or nothing when they hold no membership row.
    /// `"owner"` or `"member"`.
    pub role: Option<String>,
    /// Whether the caller is an instance admin. Independent of `role`
    /// rather than a third value of it: an admin may also be a member
    /// of the team they are administering, and the two say different
    /// things about what they may do.
    pub admin: bool,
}

/// One membership row as the roster lists it.
///
/// `login` and `display_name` are read live from the account at roster
/// time (#218), not stamped — the distinction the roster's own
/// sentence draws against the ledger: this says what a name *is now*,
/// the ledger's stamp says what it *read then*. A rename between two
/// roster reads changes this field on the next one and nothing it
/// carried on the last.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RosterMemberDto {
    /// The member.
    pub user_id: String,
    /// The account's current login.
    pub login: String,
    /// The account's current display name.
    pub display_name: String,
    /// The member's current role: `"owner"` or `"member"`.
    pub role: String,
}

/// The teams the caller is a member of (`GET /teams`).
///
/// The roster read turned around: that one takes a team and answers
/// with users, this one takes the caller and answers with teams. Its
/// path names no team because the question is not about one.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MyTeamsDto {
    /// One row per membership the caller holds, ordered oldest team
    /// first — the order the teams were created in, which is the only
    /// order the rows carry a fact for.
    pub teams: Vec<MyTeamDto>,
}

/// One team the caller belongs to.
///
/// **`name` is `None` only for a team from before #218** —
/// `TeamMembership` in the domain, which this projects, is where that
/// reading is argued. Every team founded since carries one.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MyTeamDto {
    /// The team, and what its scoped routes are named by.
    pub team_id: String,
    /// The team's name, or `None` for a team from before #218.
    pub name: Option<String>,
    /// What the caller is in it: `"owner"` or `"member"`.
    ///
    /// Free, in the sense that costs a read nothing: the membership
    /// row the query already reads is where it lives.
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
