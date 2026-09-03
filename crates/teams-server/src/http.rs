//! HTTP transport — the axum `/teams/*` router (#83 §5, the #91
//! slice).
//!
//! ## Route table
//!
//! | Method | Path | Authority |
//! |---|---|---|
//! | POST | `/teams/auth/login` | none (rate-limited) |
//! | POST | `/teams/auth/logout` | bearer token (rate-limited) |
//! | POST | `/teams/auth/device/login` | a device token (rate-limited) — answers with an ordinary session |
//! | GET | `/teams/auth/providers` | none — what this instance offers besides a password (#163) |
//! | POST | `/teams/auth/oidc/attempts` | none (rate-limited) — starts a sign-in through the provider (#163) |
//! | GET | `/teams/auth/oidc/attempts/{id}` | none — the page a browser lands on, HTML |
//! | POST | `/teams/auth/oidc/attempts/{id}/authorize` | none — the button, `303` to the provider |
//! | GET | `/teams/auth/oidc/callback` | a code from the provider (rate-limited) — `303` to the app's loopback listener |
//! | GET | `/teams/auth/oidc/attempts/{id}/done` | none — the page the listener sends the browser on to, HTML |
//! | POST | `/teams/auth/oidc/attempts/{id}/collect` | the attempt's secret and the grant the browser delivered (rate-limited) — answers with an ordinary session, once |
//! | POST | `/teams/auth/device` | any live session — mints a device token (#204) |
//! | GET | `/teams/auth/device` | any live session — the caller's own tokens, never their values |
//! | DELETE | `/teams/auth/device/{id}` | any live session — owner-scoped, `204` |
//! | POST | `/teams/create` | any authenticated user; admin-only under closed registration |
//! | POST | `/teams/{team_id}/delete` | owner, or an admin (admin-stamped) |
//! | GET | `/teams/{team_id}/roster` | member, or an admin |
//! | GET | `/teams/{team_id}/events` | member, or an admin — paged, see [`events`] |
//! | GET | `/teams/{team_id}/events/subject` | member, or an admin — one subject's events, same page contract |
//! | — | `/teams/{team_id}/forge/*` | member for every write but one; see the `forge` module |
//! | POST | `/teams/{team_id}/members/invite` | owner |
//! | POST | `/teams/{team_id}/members/remove` | owner |
//! | POST | `/teams/{team_id}/members/leave` | any caller holding a row, of themself |
//! | POST | `/teams/{team_id}/owners/grant` | owner |
//! | POST | `/teams/{team_id}/owners/revoke` | owner |
//! | PUT | `/teams/{team_id}/blobs?digest=…` | member (a roster row; an admin has no implicit upload) |
//! | GET | `/teams/{team_id}/blobs/{digest}` | member, or an admin — every failure is the same `404`, see below |
//! | POST | `/teams/{team_id}/blobs/{digest}/purge/mark` | owner, or an admin (admin-stamped — #95, the §1 delete row's reclaim sibling) |
//! | POST | `/teams/{team_id}/blobs/{digest}/purge/unmark` | owner, or an admin (admin-stamped) |
//! | POST | `/teams/{team_id}/blobs/purge/reclaim` | owner, or an admin (admin-stamped); refused while every mark is inside its grace window |
//! | GET | `/teams/{team_id}/blobs/purge/marked` | owner, or an admin — the marked set, same authority as the mark |
//! | PUT | `/teams/heads/registry` | admins only — instance scope (#132), no team gate |
//! | GET | `/teams/heads/registry` | any authenticated account — the live head artifact's bytes, verbatim |
//! | GET | `/teams/admin/accounts/{user_id}/devices` | admins only — another account's devices, never their values (#213) |
//! | DELETE | `/teams/admin/accounts/{user_id}/devices` | admins only — takes back every live device token the account holds (the ones its listing shows), `204`; the next presentation of any of them inside its window is `401 revoked_by_instance`; sessions already held run to their TTL |
//! | POST | `/teams/admin/accounts/{user_id}/lock` | admins only — the account resolves no credential until unlocked, `204`; its rows and stamps stay; `400` for the caller's own account and for the last unlocked admin |
//! | DELETE | `/teams/admin/accounts/{user_id}/lock` | admins only — lifts the lock, `204` |
//! | GET | `/teams/admin/accounts/{user_id}/events` | admins only — what was done to that account and by whom, and whether it is locked |
//! | DELETE | `/teams/admin/devices` | admins only — takes back every live device token on the instance, `204`; sessions run to their TTL |
//! | GET | `/teams/admin/events` | admins only — the instance's whole record of acts on accounts |
//!
//! ## The gate (#83 §5: every route, no exceptions)
//!
//! Two middleware layers, in request order:
//!
//! 1. [`auth_gate`] — `Authorization: Bearer` token →
//!    [`PasswordAuth::resolve_session`] → [`AccountRecord`], inserted
//!    as an extension. Missing, malformed, unknown and **expired**
//!    tokens, and a live token whose account an admin has locked
//!    (#213), are all the same `401` (an expired row is deleted on
//!    touch; a locked account's row is kept).
//! 2. [`team_gate`] (team-scoped routes only) — the `{team_id}` path
//!    segment → team existence (`404`) → the caller's current role in
//!    *this* team, read from state, never from the ledger (#83 §1).
//!    A caller with neither a membership row nor the admin flag is
//!    `403` before any handler runs.
//!
//! The handler then asks `teams-core`'s decision functions
//! ([`verb_allowed`] / [`may_create_team`]) with the capacity the gate
//! established. When both capacities could act, the membership row
//! wins and the ledger stamp is the member's — the admin variant is
//! reserved for an admin acting *from outside* the membership set,
//! which is exactly when §1 demands the stamp say so.
//!
//! ## Which of the device-token routes the limiter covers (#204)
//!
//! The login arm alone, and the split is the decision. #83 §5 puts new
//! auth routes in the limited router, and what that limiter is for is
//! an unauthenticated caller presenting a *credential*: its budget is
//! what bounds guessing. `POST /teams/auth/device/login` is exactly
//! that — a token arrives from nobody in particular and either
//! resolves or does not — so it sits beside the password arm and
//! shares its bucket.
//!
//! Every other device-token route presents no credential; each
//! presents a session [`auth_gate`] has already resolved. Putting them
//! under the same bucket would spend a login's budget on a caller who
//! is already inside, so a person who minted a token would find
//! themselves unable to log in again — while protecting a guessing
//! surface that does not exist, because there is nothing to guess past
//! a session that already resolved. They sit behind the gate instead.
//!
//! ## Minting asks for a live session and nothing more (#204)
//!
//! Not the password arm specifically, and this is the other question
//! #204 leaves open. Any-session is what makes the provider path
//! (#163) free: a sign-in through the provider ends in a session the
//! same way a password does, and the minting path never learns which
//! way in was taken — which is the property the whole issue turns on.
//! Requiring a re-auth would put a password back in front of a flow
//! whose point is that a password is not always what happened.
//!
//! What that costs is written down rather than waved at: a stolen live
//! session can mint a device token, which outlives the session by
//! design. The bound on it is that the owner can see every token
//! (`GET`) and revoke any of it (`DELETE`), and that the tokens the
//! disk holds end on a day fixed at the mint and earlier when unused
//! ([`TeamsCtx::device_token_ttl_ms`] and
//! [`TeamsCtx::device_token_idle_ms`], #163). A re-auth requirement
//! can be added later without moving the table or changing a single
//! row shape.
//!
//! ## The blob read is the one deliberate exception to [`team_gate`]
//!
//! `GET /teams/{team_id}/blobs/{digest}` sits behind [`auth_gate`]
//! only, and answers **one indistinguishable `404`** for every miss:
//! unknown team, caller neither a member nor an admin, digest
//! never uploaded, digest linked only in a team the caller cannot
//! read — and, since #95, a link **marked for purge**, whose grace
//! window hides it behind the very same answer. The gate's
//! usual 403/404 split would confirm which part of the probe was
//! right; on the byte-serving surface that is exactly the existence
//! oracle the link boundary exists to close (#83 §3 — a digest
//! "exists" for a caller iff a link row sits in a team they belong
//! to), the same conflation `asterism-server`'s asset-file route
//! documents. Uploads stay behind the full gate: mutations answer
//! 403 to outsiders on every other route, and a 403 on `PUT` reveals
//! nothing about any digest.
//!
//! ## Error mapping
//!
//! Same body shape as `asterism-server` (`{"kind", "message"}`).
//! Domain refusals surface as client errors, never `500`:
//! `Validation` → 400, [`DomainError::LastOwner`] and
//! `DigestMismatch` → 409 (the mismatch body carries declared and
//! computed, both), `Infra` → 500; the gate adds 401/403/404 and the
//! limiter 429.
//!
//! The forge's routes answer on a second table, because their
//! refusals come from the other plane's `DomainError` and carry a
//! field this one has no column for: `reason`, on a conflict, which
//! is what tells a caller whether retrying is worth anything. Those
//! bodies are `asterism-server`'s to the letter — see `ApiError::Forge`
//! and `forge_response` below.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
// The other plane's refusal type, named for what it is here: the
// hosted forge's services are `asterism-core`'s and speak its
// `DomainError` (#148 decision 20), and two types called `DomainError`
// in one module would read as one.
use asterism_core::DomainError as ForgeError;
use http_body_util::BodyExt as _;
use teams_contract::command::UploadBlobCommand;
use teams_contract::dto::{
    BlobUploadedDto, HeadPublishedDto, MarkedBlobLinkDto, MarkedBlobsDto, PurgeReclaimedDto,
};
use teams_core::DomainError;
use teams_core::domain::head_registry::TagHeadEntry;
use teams_core::domain::identity::{
    ActorStamp, CreationActor, InstanceAdmin, LedgerActor, Membership, Role, TeamAuthority,
    TeamVerb, may_create_team, verb_allowed,
};
use teams_core::domain::ledger::LedgerEvent;
use teams_core::domain::store::{DeclaredDigest, TeamBlobLink, parse_digest};
use teams_core::port::auth::CredentialVerifier;
use teams_infra::auth::password::{AccountRecord, DeviceTokenResolution, LockOutcome};
use teams_infra::gc::sweep_zero_link_blobs;
use teams_infra::sqlite::map::{subject_from_ref, subject_to_ref};
// The shapes a member's client also reads, from the leaf both planes
// depend on (#148 decision 15). Which crate a shape comes from is the
// answer to "does a client say this", and nothing else changed about
// any of them.
use asterism_teams_wire::command::{
    CreateTeamCommand, DeviceLoginCommand, GrantOwnerCommand, InviteMemberCommand, LoginCommand,
    MintDeviceTokenCommand, RemoveMemberCommand, RevokeOwnerCommand,
};
use asterism_teams_wire::dto::{
    AccountEventDto, AccountEventsDto, DeviceTokenDto, DeviceTokenMintedDto, DeviceTokensDto,
    LedgerEventDto, LedgerPageDto, MyTeamDto, MyTeamsDto, RosterDto, RosterMemberDto, SessionDto,
    SubjectRefDto, TeamCreatedDto, ViewerDto,
};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::state::{TeamsCtx, now_ms};

/// HTTP-boundary error type. Same tagged body as `asterism-server`'s,
/// with the auth/authority outcomes this surface adds.
pub(crate) enum ApiError {
    /// A domain refusal, mapped by variant.
    Domain(DomainError),
    /// A refusal from the hosted forge, which speaks the other plane's
    /// `DomainError` (#148 decision 20 — the services are
    /// `asterism-core`'s, unchanged).
    ///
    /// Mapped by `asterism-server`'s table rather than by this one, to
    /// the letter including the `reason` token on a conflict: the
    /// mirrored routes answer with the same paths and the same DTOs
    /// (decision 19), and a client that had to branch differently on a
    /// refusal depending on which prefix it asked would not be talking
    /// to a mirror.
    Forge(ForgeError),
    /// No token, a malformed header, an unknown token, an expired one,
    /// or a live one whose account an admin has locked (#213) —
    /// deliberately indistinguishable.
    Unauthorized,
    /// Authenticated, but this verb is not yours here.
    Forbidden(String),
    /// The `{team_id}` names no team on this instance.
    TeamNotFound,
    /// The `{user_id}` an admin named has no account on this instance
    /// (#213). Sayable, because the routes that answer it are an
    /// admin's, and which accounts exist is an admin's to know.
    AccountNotFound,
    /// Nothing has been said about this entry (#148 decision 12) — an
    /// ordinary absence rather than a fault.
    ///
    /// The same `404` also covers an entry on another team's line,
    /// which the read scopes out. Conflating the two is not the blob
    /// read's evasion: the line's own reads enumerate its entries to
    /// any member, so nothing is being hidden from a caller who can
    /// already see it, and a caller who cannot see the line learns
    /// only that this instance will not answer them about it.
    ProjectionNotFound,
    /// The blob read's one answer for every miss — unknown team,
    /// non-member, unlinked digest, foreign digest — deliberately a
    /// single variant so the four cannot drift into distinguishable
    /// bodies (see the module doc).
    BlobNotFound,
    /// `GET /teams/heads/registry` while nothing has been published
    /// (#132) — a plain absence, sayable to any authenticated account:
    /// which head an instance endorses is exactly what the registry
    /// exists to tell its members.
    HeadRegistryEmpty,
    /// The auth limiter refused the attempt.
    RateLimited,
    /// A sign-in through a provider was asked of an instance that has
    /// none configured (#163).
    NoProvider,
    /// The attempt id names no live attempt — or the secret presented
    /// is not the one it was started with, which is deliberately the
    /// same answer (the `oidc` module doc says why).
    AttemptNotFound,
    /// A device token refused, and why: the `401` an app acts on
    /// rather than the one it can only show a password form for (#163,
    /// #213). The token is `reason` on the body, and the message is
    /// worded per reason, because "sign in again" is the right advice
    /// for a credential that ended and the wrong one for an account
    /// that is locked.
    ReauthRequired(&'static str),
}

impl From<DomainError> for ApiError {
    fn from(err: DomainError) -> Self {
        Self::Domain(err)
    }
}

impl From<ForgeError> for ApiError {
    fn from(err: ForgeError) -> Self {
        Self::Forge(err)
    }
}

/// A forge refusal as `asterism-server` answers it, to the letter.
///
/// Its own function rather than an arm of the table below, because
/// this body has a field that one has no column for: on a conflict,
/// the `reason` token that tells a caller whether retrying is worth
/// anything. Four of them exist and `asterism-server`'s own error type
/// documents what each asks of the caller; a mirrored route that
/// dropped the token would leave a client with a 409 it cannot act on.
fn forge_response(err: &ForgeError) -> Response {
    let (status, kind) = match err {
        ForgeError::PersonaNotFound(_)
        | ForgeError::AssetNotFound(_)
        | ForgeError::NotFound { .. } => (StatusCode::NOT_FOUND, "NotFound"),
        ForgeError::Validation(_) => (StatusCode::BAD_REQUEST, "Validation"),
        ForgeError::DuplicatePersona(_) | ForgeError::Conflict { .. } => {
            (StatusCode::CONFLICT, "Conflict")
        }
        ForgeError::Infra(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal"),
    };
    let mut body = serde_json::json!({ "kind": kind, "message": err.to_string() });
    if let (Some(reason), Some(object)) = (err.reason(), body.as_object_mut()) {
        object.insert("reason".into(), serde_json::Value::from(reason));
    }
    (status, Json(body)).into_response()
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, kind, message) = match self {
            Self::Forge(err) => return forge_response(&err),
            Self::ReauthRequired(reason) => {
                let message = match reason {
                    "locked" => {
                        "the account is locked on this instance; ask whoever runs it".to_string()
                    }
                    "revoked_by_instance" => {
                        "this device was signed out by the instance; sign in again".to_string()
                    }
                    _ => format!("the stored credential is {reason}; sign in again"),
                };
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "kind": "Unauthorized",
                        "message": message,
                        "reason": reason,
                    })),
                )
                    .into_response();
            }
            Self::Domain(err) => {
                let (status, kind) = match &err {
                    DomainError::Validation(_) => (StatusCode::BAD_REQUEST, "Validation"),
                    // The request was well-formed; the team's *state*
                    // refuses it — the distinction the domain carves
                    // this variant out for.
                    DomainError::LastOwner { .. } | DomainError::DigestMismatch { .. } => {
                        (StatusCode::CONFLICT, "Conflict")
                    }
                    // Its own kind, not the generic conflict: the
                    // caller's remedy (unmark, or wait for reclaim) is
                    // the point of the refusal, and a client can only
                    // branch on what the body distinguishes (#95).
                    DomainError::MarkedForPurge { .. } => {
                        (StatusCode::CONFLICT, "marked_for_purge")
                    }
                    DomainError::Infra(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal"),
                };
                (status, kind, err.to_string())
            }
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "Unauthorized",
                "missing, invalid, or expired session token".to_string(),
            ),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, "Forbidden", message),
            Self::TeamNotFound => (
                StatusCode::NOT_FOUND,
                "NotFound",
                "no such team on this instance".to_string(),
            ),
            Self::AccountNotFound => (
                StatusCode::NOT_FOUND,
                "NotFound",
                "no such account on this instance".to_string(),
            ),
            Self::BlobNotFound => (
                StatusCode::NOT_FOUND,
                "NotFound",
                "no such blob in this team".to_string(),
            ),
            Self::ProjectionNotFound => (
                StatusCode::NOT_FOUND,
                "NotFound",
                "nothing has been said about this entry".to_string(),
            ),
            Self::HeadRegistryEmpty => (
                StatusCode::NOT_FOUND,
                "NotFound",
                "no head has been published on this instance".to_string(),
            ),
            Self::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "RateLimited",
                "too many authentication attempts; retry later".to_string(),
            ),
            Self::NoProvider => (
                StatusCode::NOT_FOUND,
                "NotFound",
                "this instance signs people in with passwords only".to_string(),
            ),
            Self::AttemptNotFound => (
                StatusCode::NOT_FOUND,
                "NotFound",
                "no such sign-in attempt".to_string(),
            ),
        };
        (
            status,
            Json(serde_json::json!({ "kind": kind, "message": message })),
        )
            .into_response()
    }
}

/// The account [`auth_gate`] resolved — present on every request past
/// it.
#[derive(Clone)]
pub(crate) struct AuthedAccount(pub(crate) AccountRecord);

/// What [`team_gate`] established about the caller in the `{team_id}`
/// team.
///
/// **Two axes, not one value.** A role is a membership row in this
/// team; being an instance admin is a standing that belongs to no
/// roster. A caller may hold both — an admin who founded a team holds
/// a row in it like anybody else — so neither field is derivable from
/// the other, and [`decide`] needs the pair rather than either alone.
/// Which capacity a given verb is granted under is that function's
/// question, under the rule the module doc above states.
///
/// **This is the sentence the rest of the surface leaves to this
/// site.** A route refusing a caller who holds no row says *that* —
/// the absence of a row — rather than naming an instance admin, who is
/// only the commonest caller in that state and not the condition
/// anything tests.
#[derive(Clone)]
pub(crate) struct TeamAccess {
    /// The team the gate resolved from the path.
    pub(crate) team_id: Uuid,
    /// The caller's role in it, or nothing when they hold no row.
    pub(crate) role: Option<Role>,
    /// Whether the caller is an instance admin. Independent of `role`.
    pub(crate) admin: bool,
}

/// Which capacity a verb was granted under — what decides the ledger
/// stamp.
#[derive(Clone, Copy)]
pub(crate) enum Capacity {
    Member,
    Admin,
}

/// Builds the router; the caller binds a listener and calls
/// `axum::serve` (with connect-info, so the limiter sees client IPs).
pub fn router(ctx: Arc<TeamsCtx>) -> Router {
    let auth = Router::new()
        .route("/teams/auth/login", post(login))
        .route("/teams/auth/logout", post(logout))
        // The device arm that presents a credential (#204). Its
        // siblings present a session instead and live below the gate —
        // the module doc's "which of the device-token routes the
        // limiter covers".
        .route("/teams/auth/device/login", post(device_login))
        // The provider path's routes under the limiter (#163): the
        // provider's callback (presents a code) and the collect
        // (presents the attempt's secret and grant) belong here by the
        // rule below; starting an attempt presents nothing and is here
        // for a different reason — each start holds ten minutes of
        // memory, and the budget is what bounds how many. The pages a
        // browser walks between them sit in the public router below.
        .route(
            "/teams/auth/oidc/attempts",
            post(crate::oidc::attempt_start),
        )
        .route("/teams/auth/oidc/callback", get(crate::oidc::callback))
        .route(
            "/teams/auth/oidc/attempts/{id}/collect",
            post(crate::oidc::attempt_collect),
        )
        // One limiter over every route above it (#83 §5). What decides
        // whether a new auth route belongs in this block is whether it
        // *presents a credential*: the budget is what bounds guessing,
        // so an arm somebody can guess at goes here. The device
        // verbs below present a session the gate already resolved and
        // sit outside — the module doc's "which of the device-token
        // routes the limiter covers" is the whole argument.
        .layer(middleware::from_fn_with_state(ctx.clone(), auth_rate_limit))
        .with_state(ctx.clone());

    // Public and unlimited: what a connect form reads before anybody
    // has a credential to present (#163), and the pages a browser
    // walks around a sign-in. The page shows a label and the done page
    // says what happened; the button does take the page's token, and
    // it is outside the limiter because the token is 256 random bits
    // with nothing to guess at — a budget bounds guessing, and there is
    // none. None of them answers anything a caller could not learn by
    // starting an attempt of their own.
    let public = Router::new()
        .route("/teams/auth/providers", get(crate::oidc::providers))
        .route(
            "/teams/auth/oidc/attempts/{id}",
            get(crate::oidc::attempt_page),
        )
        .route(
            "/teams/auth/oidc/attempts/{id}/authorize",
            post(crate::oidc::attempt_authorize),
        )
        .route(
            "/teams/auth/oidc/attempts/{id}/done",
            get(crate::oidc::attempt_done),
        )
        .with_state(ctx.clone());

    let team_scoped = Router::new()
        .route("/teams/{team_id}/delete", post(delete_team))
        .route("/teams/{team_id}/roster", get(roster))
        .route("/teams/{team_id}/events", get(events))
        .route("/teams/{team_id}/members/invite", post(invite_member))
        .route("/teams/{team_id}/members/remove", post(remove_member))
        .route("/teams/{team_id}/members/leave", post(leave_team))
        .route("/teams/{team_id}/owners/grant", post(grant_owner))
        .route("/teams/{team_id}/owners/revoke", post(revoke_owner))
        .route("/teams/{team_id}/blobs", put(upload_blob))
        // The purge two-step (#95). `purge` is a static segment, which
        // axum prefers over the `{digest}` capture — and no digest can
        // collide with it anyway, the grammar admits `sha256:` forms
        // only.
        .route(
            "/teams/{team_id}/blobs/{digest}/purge/mark",
            post(purge_mark),
        )
        .route(
            "/teams/{team_id}/blobs/{digest}/purge/unmark",
            post(purge_unmark),
        )
        .route("/teams/{team_id}/blobs/purge/reclaim", post(purge_reclaim))
        .route(
            "/teams/{team_id}/blobs/purge/marked",
            get(purge_marked_list),
        )
        .route("/teams/{team_id}/events/subject", get(events_for_subject))
        // The hosted forge, mirrored under this team's prefix (#148
        // decision 19). Merged *inside* the gate rather than beside
        // it: "every route sits behind the membership gate" is the
        // whole of the answer for all but the discard, and a forge
        // route added later inherits that by where it is written.
        .merge(crate::forge::routes())
        .layer(middleware::from_fn_with_state(ctx.clone(), team_gate));

    let authed = Router::new()
        .route("/teams/create", post(create_team))
        // A team read that names no team, because the question is
        // about the caller rather than about a team: everything under
        // `team_scoped` needs an id to gate on and this is what a
        // caller asks *before* they have one.
        //
        // One segment, so the grammar note the routes below carry does
        // not apply here — there is no capture at this position for a
        // static segment to be preferred over.
        .route("/teams", get(my_teams))
        // Managing the caller's own device tokens (#204). No team to
        // gate on: these answer about the account the session resolved
        // to and nobody else; the admin's reach over another account's
        // is under `/teams/admin` below (#213). `auth` is a static
        // segment, preferred over the `{team_id}` capture — the same
        // grammar note `heads` carries below.
        .route(
            "/teams/auth/device",
            post(mint_device_token).get(device_tokens),
        )
        .route("/teams/auth/device/{id}", delete(revoke_device_token))
        // Deliberately outside `team_gate`: the blob read answers one
        // `404` for every miss instead of the gate's 403/404 split —
        // the module doc's "one deliberate exception".
        .route("/teams/{team_id}/blobs/{digest}", get(read_blob))
        // Instance scope (#132): no team to gate on. `heads` is a
        // static segment, which axum prefers over the `{team_id}`
        // capture — the same grammar note as `purge` above.
        .route(
            "/teams/heads/registry",
            get(head_registry).put(publish_head_registry),
        )
        // An admin's reach over somebody else's sign-in (#213): the
        // account verbs the instance admin had no route for. Instance
        // scope, admin-only inside each handler, no team gate — the
        // same shape as the head registry. `admin` is a static
        // segment, preferred over the `{team_id}` capture.
        .route(
            "/teams/admin/accounts/{user_id}/devices",
            get(admin_device_tokens).delete(admin_revoke_device_tokens),
        )
        .route(
            "/teams/admin/accounts/{user_id}/lock",
            post(admin_lock_account).delete(admin_unlock_account),
        )
        .route(
            "/teams/admin/accounts/{user_id}/events",
            get(admin_account_events),
        )
        .route(
            "/teams/admin/devices",
            delete(admin_revoke_every_device_token),
        )
        .route("/teams/admin/events", get(admin_events))
        .merge(team_scoped)
        .layer(middleware::from_fn_with_state(ctx.clone(), auth_gate))
        .with_state(ctx);

    Router::new().merge(auth).merge(public).merge(authed)
}

// ----------------------------------------------------------------------
// Middleware.
// ----------------------------------------------------------------------

/// Per-key limiter over the auth endpoints. Keyed by client IP when
/// the connection carries one, else a fixed key (see
/// [`crate::rate_limit`] for the choice).
async fn auth_rate_limit(
    State(ctx): State<Arc<TeamsCtx>>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let key = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip().to_string())
        .unwrap_or_else(|| "local".to_string());
    if !ctx.auth_limiter.check(&key) {
        return Err(ApiError::RateLimited);
    }
    Ok(next.run(req).await)
}

/// Session token → account. The first half of the #83 §5 gate.
async fn auth_gate(
    State(ctx): State<Arc<TeamsCtx>>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = bearer_token(req.headers())
        .ok_or(ApiError::Unauthorized)?
        .to_string();
    let account = ctx
        .auth
        .resolve_session(&token, now_ms())
        .await?
        .ok_or(ApiError::Unauthorized)?;
    req.extensions_mut().insert(AuthedAccount(account));
    Ok(next.run(req).await)
}

/// Account → standing in the `{team_id}` team. The second half of the
/// gate, on every team-scoped route.
///
/// The path lands as a name→value map rather than a typed `Path<Uuid>`
/// because the gate spans routes with different arities — the purge
/// routes carry `{digest}` beside `{team_id}` (#95), and a
/// single-value extractor refuses any route with a second capture.
async fn team_gate(
    State(ctx): State<Arc<TeamsCtx>>,
    Path(params): Path<std::collections::HashMap<String, String>>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let team_id = parse_uuid(
        params.get("team_id").map(String::as_str).unwrap_or(""),
        "team_id",
    )?;
    let account = req
        .extensions()
        .get::<AuthedAccount>()
        .cloned()
        .ok_or(ApiError::Unauthorized)?;
    if !ctx.repo.team_exists(team_id).await? {
        return Err(ApiError::TeamNotFound);
    }
    // Current membership state — never the ledger (#83 §1).
    let roster = ctx.repo.roster(team_id).await?;
    let role = roster.role_of(account.0.user_id);
    if role.is_none() && !account.0.admin {
        return Err(ApiError::Forbidden(
            "you are not a member of this team".to_string(),
        ));
    }
    req.extensions_mut().insert(TeamAccess {
        team_id,
        role,
        admin: account.0.admin,
    });
    Ok(next.run(req).await)
}

// ----------------------------------------------------------------------
// Authority helpers.
// ----------------------------------------------------------------------

/// Asks the #83 §1 authority table which capacity (if any) grants
/// `verb`. The membership row is asked first: an admin who is also a
/// member acts through the row like anyone else, and the admin
/// capacity only answers when the caller stands outside the roster —
/// which is exactly the action §1 wants admin-stamped.
pub(crate) fn decide(access: &TeamAccess, verb: TeamVerb) -> Result<Capacity, ApiError> {
    if let Some(role) = access.role
        && verb_allowed(TeamAuthority::Member(role), verb)
    {
        return Ok(Capacity::Member);
    }
    if access.admin && verb_allowed(TeamAuthority::Admin, verb) {
        return Ok(Capacity::Admin);
    }
    Err(ApiError::Forbidden(format!(
        "your role does not permit {}",
        verb_name(verb)
    )))
}

const fn verb_name(verb: TeamVerb) -> &'static str {
    match verb {
        TeamVerb::Delete => "deleting the team",
        TeamVerb::Invite => "inviting a member",
        TeamVerb::Remove => "removing a member",
        TeamVerb::GrantOwner => "granting the owner role",
        TeamVerb::RevokeOwner => "revoking the owner role",
        TeamVerb::Purge => "purging the team's storage",
        TeamVerb::ForgeWork => "working on this team's forge",
        TeamVerb::ForgeDiscard => "discarding a line",
    }
}

/// The ledger stamp for `account` acting under `capacity` — the one
/// place the member/admin variant is chosen (#83 §1: never disguised).
pub(crate) fn stamp(account: &AccountRecord, capacity: Capacity) -> Result<LedgerActor, ApiError> {
    Ok(match capacity {
        Capacity::Member => LedgerActor::member(ActorStamp {
            user_id: account.user_id,
            display_name: account.display_name.clone(),
        }),
        Capacity::Admin => {
            let admin = InstanceAdmin::new(account.user_id, account.display_name.clone())?;
            LedgerActor::admin(&admin)
        }
    })
}

// ----------------------------------------------------------------------
// Handlers — auth.
// ----------------------------------------------------------------------

/// `POST /teams/auth/login`. A wrong password and an unknown login are
/// the same `401` (the port's one-arm contract). Each login also runs
/// the bulk expiry sweep, so the session table is cleaned at least as
/// often as it grows.
async fn login(
    State(ctx): State<Arc<TeamsCtx>>,
    Json(cmd): Json<LoginCommand>,
) -> Result<Json<SessionDto>, ApiError> {
    let now = now_ms();
    ctx.auth.cleanup_expired(now).await?;
    let verifier: &dyn CredentialVerifier = &ctx.auth;
    let user_id = verifier
        .verify(&cmd.login, &cmd.password)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let account = ctx
        .auth
        .account(user_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let token = ctx
        .auth
        .create_session(user_id, now, ctx.session_ttl_ms)
        .await?;
    Ok(Json(session_dto(&ctx, token, &account, now).await?))
}

/// A session as the wire says it — one function for the three arms
/// that mint one (password, device token, provider), so that what a
/// session says about its account cannot differ by how it was got.
pub(crate) async fn session_dto(
    ctx: &TeamsCtx,
    token: String,
    account: &AccountRecord,
    now: i64,
) -> Result<SessionDto, ApiError> {
    let instance_id = ctx.auth.instance_id().await?;
    Ok(SessionDto {
        token,
        user_id: account.user_id.to_string(),
        login: account.login.clone(),
        display_name: account.display_name.clone(),
        admin: account.admin,
        expires_at_ms: now.saturating_add(ctx.session_ttl_ms),
        // One tenant per instance today, so the tenant is the
        // instance; the field exists so that stops being true without
        // a client noticing.
        tenant_id: instance_id.clone(),
        instance_id,
    })
}

/// `POST /teams/auth/logout`. Destroys the presented session;
/// idempotent, because the caller's goal — this token resolves to
/// nothing — is already true for a token that never resolved.
async fn logout(
    State(ctx): State<Arc<TeamsCtx>>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let token = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    ctx.auth.destroy_session(token).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----------------------------------------------------------------------
// Handlers — device tokens (#204).
// ----------------------------------------------------------------------

/// `POST /teams/auth/device/login`. A device token in, an **ordinary
/// session** out — same table, same TTL, same shape on the wire as the
/// password arm's.
///
/// That sameness is the design (#204): device tokens sit in front of
/// sessions rather than beside them, so nothing downstream of
/// [`auth_gate`] can tell how a session was obtained, and nothing
/// downstream needs to. A token that does not resolve is a `401`
/// whose body carries the reason the adapter's
/// [`DeviceTokenResolution`] gave, because the caller is an app
/// holding a credential it was given, and which end it met is what it
/// tells the person (#163). Unlike the password arm, there is nothing
/// to enumerate: a device token is 256 random bits, and "revoked" is
/// also the answer for one this instance never minted.
///
/// Sweeps both tables, and the two sweeps are here for different
/// reasons — which is the adapter's rule rather than this route's. A
/// session table is swept where it fills, and this route fills it. A
/// device-token table fills too rarely for that to bound it, so it is
/// swept where one is read and where one is presented
/// (`PasswordAuth::cleanup_expired_device_tokens`), and this is the
/// surface where one is presented. The device sweep runs **after** the
/// resolve, not before: a sweep first would delete the very row an
/// expired token names and the resolve would then answer `revoked`
/// for a token that expired, which is the distinction the reason
/// exists to draw. The table is bounded either way.
async fn device_login(
    State(ctx): State<Arc<TeamsCtx>>,
    Json(cmd): Json<DeviceLoginCommand>,
) -> Result<Json<SessionDto>, ApiError> {
    let now = now_ms();
    ctx.auth.cleanup_expired(now).await?;
    let resolution = ctx
        .auth
        .resolve_device_token(&cmd.token, now, ctx.device_token_idle_ms)
        .await?;
    ctx.auth
        .cleanup_expired_device_tokens(now, ctx.device_token_idle_ms)
        .await?;
    let account = match resolution {
        DeviceTokenResolution::Resolved(account) => account,
        DeviceTokenResolution::Expired => return Err(ApiError::ReauthRequired("expired")),
        DeviceTokenResolution::Idle => return Err(ApiError::ReauthRequired("idle")),
        // The two #213 ends: the instance took the token back, or
        // locked the account. Named apart from `revoked` because the
        // person holding the device did neither, and what they can do
        // about it differs — sign in again, or ask whoever runs the
        // instance.
        DeviceTokenResolution::RevokedByInstance => {
            return Err(ApiError::ReauthRequired("revoked_by_instance"));
        }
        DeviceTokenResolution::Locked => return Err(ApiError::ReauthRequired("locked")),
        DeviceTokenResolution::Unknown => return Err(ApiError::ReauthRequired("revoked")),
    };
    let token = ctx
        .auth
        .create_session(account.user_id, now, ctx.session_ttl_ms)
        .await?;
    Ok(Json(session_dto(&ctx, token, &account, now).await?))
}

/// `POST /teams/auth/device` — mints a device token for the caller's
/// own account.
///
/// The account comes from the session the gate resolved and cannot
/// come from the body, which is what keeps this from being a way to
/// issue a credential for somebody else. Why a live session is enough,
/// and what that costs, is the module doc's.
///
/// **The token is in this response and never in another one.** The
/// listing answers with handles, and the instance holds only a
/// SHA-256 — so a client that loses the value mints a second token
/// rather than asking for this one again.
async fn mint_device_token(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Json(cmd): Json<MintDeviceTokenCommand>,
) -> Result<Json<DeviceTokenMintedDto>, ApiError> {
    let minted = ctx
        .auth
        .mint_device_token(
            account.user_id,
            &cmd.label,
            now_ms(),
            ctx.device_token_ttl_ms,
        )
        .await?;
    Ok(Json(DeviceTokenMintedDto {
        token: minted.token,
        id: minted.id.to_string(),
        expires_at_ms: minted.expires_at_ms,
    }))
}

/// `GET /teams/auth/device` — the caller's own device tokens.
///
/// Owner-scoped: this route answers about the caller and nobody else.
/// It once argued that no admin widening existed because a person's
/// machines are their own; #213 widened it, on a route of the admin's
/// (`GET /teams/admin/accounts/{user_id}/devices`) rather than here,
/// so that the owner's route still takes no account to ask about and
/// the admin's act is the admin's, recorded as such.
///
/// Sweeps first to keep the table bounded where it is read; what is
/// listed is decided by the listing's own predicate, the one the
/// sweep and the instance's revoke share, so a row that stopped
/// resolving is not shown whether or not the sweep has reached it.
async fn device_tokens(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
) -> Result<Json<DeviceTokensDto>, ApiError> {
    let now = now_ms();
    ctx.auth
        .cleanup_expired_device_tokens(now, ctx.device_token_idle_ms)
        .await?;
    let tokens = ctx
        .auth
        .list_device_tokens(account.user_id, now, ctx.device_token_idle_ms)
        .await?;
    Ok(Json(DeviceTokensDto {
        tokens: tokens.into_iter().map(device_token_dto).collect(),
    }))
}

/// `DELETE /teams/auth/device/{id}` — revokes one of the caller's own
/// device tokens.
///
/// `204`, and the same `204` for a handle that named nothing, one
/// already revoked, and one belonging to another account — the
/// adapter's idempotence, kept on the wire because distinguishing them
/// would answer "does this id exist" about ids the caller has no
/// business knowing. Logout answers the same way for the same reason.
///
/// The sessions this token minted are not revoked with it, for the
/// reason `PasswordAuth::revoke_device_token` gives.
async fn revoke_device_token(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = parse_uuid(&id, "id")?;
    ctx.auth.revoke_device_token(account.user_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----------------------------------------------------------------------
// Handlers — an admin's reach over accounts (#213).
// ----------------------------------------------------------------------
//
// The capacity is the one #83 §1 gives an admin — a flag on the
// account, outside every roster — reaching an *account* through it
// rather than a team. Each is admin-only in the handler, as the head
// registry is, and each act on an account is written to the
// instance's record (`account_event`, V13) with the admin stamped the
// way a ledger actor is, for the reason the migration gives.

/// Call this before `subject_account`, not after: the order is what
/// keeps a non-admin from learning which accounts exist.
fn require_admin(account: &AccountRecord, act: &str) -> Result<(), ApiError> {
    if account.admin {
        return Ok(());
    }
    Err(ApiError::Forbidden(format!(
        "{act} is an instance admin's act (#213)"
    )))
}

/// The account a `{user_id}` names, or [`ApiError::AccountNotFound`]
/// for one this instance has no row for.
async fn subject_account(ctx: &TeamsCtx, raw: &str) -> Result<AccountRecord, ApiError> {
    let user_id = parse_uuid(raw, "user_id")?;
    ctx.auth
        .account(user_id)
        .await?
        .ok_or(ApiError::AccountNotFound)
}

/// One device token as a listing shows it: the same row, and the same
/// absence, whichever route asked.
fn device_token_dto(row: teams_infra::auth::password::DeviceTokenRecord) -> DeviceTokenDto {
    DeviceTokenDto {
        id: row.id.to_string(),
        label: row.label,
        created_at_ms: row.created_at_ms,
        last_used_at_ms: row.last_used_at_ms,
        expires_at_ms: row.expires_at_ms,
    }
}

fn account_event_dto(event: teams_infra::auth::password::AccountEvent) -> AccountEventDto {
    AccountEventDto {
        seq: event.seq,
        occurred_at_ms: event.occurred_at_ms,
        actor_user_id: event.actor_user_id.to_string(),
        actor_name: event.actor_name,
        subject_user_id: event.subject_user_id.map(|id| id.to_string()),
        kind: event.kind,
    }
}

/// `GET /teams/admin/accounts/{user_id}/devices` — admins only: the
/// devices another account has stored a token on, in the shape the
/// owner's own listing uses and with the same absence — no value, no
/// digest — and the same sweep first, for the reason the owner's read
/// gives. What this shows is decided by the same predicate the
/// admin's revoke uses, so it is what that revoke would take back, and
/// nothing else.
async fn admin_device_tokens(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Path(user_id): Path<String>,
) -> Result<Json<DeviceTokensDto>, ApiError> {
    require_admin(&account, "reading another account's devices")?;
    let subject = subject_account(&ctx, &user_id).await?;
    let now = now_ms();
    ctx.auth
        .cleanup_expired_device_tokens(now, ctx.device_token_idle_ms)
        .await?;
    let tokens = ctx
        .auth
        .list_device_tokens(subject.user_id, now, ctx.device_token_idle_ms)
        .await?
        .into_iter()
        .map(device_token_dto)
        .collect();
    Ok(Json(DeviceTokensDto { tokens }))
}

/// `DELETE /teams/admin/accounts/{user_id}/devices` — admins only:
/// takes back every live device token the account holds — the ones
/// its listing shows — so no device of theirs signs in silently
/// again. Each becomes a tombstone and answers `revoked_by_instance`
/// once, on `DeviceTokenResolution`'s terms. Recorded as
/// `devices_revoked` on the account. Sessions are left as
/// `PasswordAuth::revoke_device_token` leaves them, for the reason
/// given there; the lock is the verb that stops them resolving, which
/// is why offboarding is this and then the lock.
async fn admin_revoke_device_tokens(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Path(user_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_admin(&account, "taking another account's devices back")?;
    let subject = subject_account(&ctx, &user_id).await?;
    let now = now_ms();
    ctx.auth
        .revoke_device_tokens_of(subject.user_id, now, ctx.device_token_idle_ms)
        .await?;
    ctx.auth
        .record_account_event(&account, Some(subject.user_id), "devices_revoked", now)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /teams/admin/devices` — admins only: takes back every live
/// device token on the instance, the admin's own included, which is
/// the honest reading of "every". Sessions are left as the per-account
/// verb leaves them. Recorded once, on no account.
async fn admin_revoke_every_device_token(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
) -> Result<StatusCode, ApiError> {
    require_admin(&account, "taking every device back")?;
    let now = now_ms();
    ctx.auth
        .revoke_every_device_token(now, ctx.device_token_idle_ms)
        .await?;
    ctx.auth
        .record_account_event(&account, None, "devices_revoked", now)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /teams/admin/accounts/{user_id}/lock` — admins only: the
/// account keeps its rows and resolves no credential from now, on the
/// terms `PasswordAuth::lock_account` defines. Its ledger stamps keep
/// resolving, which is the difference from deleting it. An admin
/// cannot lock themself, and the last admin who can authenticate is
/// locked by nobody — the adapter decides that inside its own
/// statement, so two admins locking each other in the same instant
/// cannot leave the instance with none; the way back from a lock
/// applied out of band is `bootstrap-admin`, whose doc says why.
/// Recorded as `locked`; locking an account already locked records
/// nothing and answers the same, which the adapter decides in its
/// statement rather than this handler in a read before it.
async fn admin_lock_account(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Path(user_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_admin(&account, "locking an account")?;
    let subject = subject_account(&ctx, &user_id).await?;
    if subject.user_id == account.user_id {
        return Err(ApiError::Domain(DomainError::Validation(
            "an admin cannot lock their own account; another admin can".to_string(),
        )));
    }
    let now = now_ms();
    match ctx.auth.lock_account(subject.user_id, now).await? {
        LockOutcome::Locked => {
            ctx.auth
                .record_account_event(&account, Some(subject.user_id), "locked", now)
                .await?;
        }
        LockOutcome::Unchanged => {}
        LockOutcome::LastAdmin => {
            return Err(ApiError::Domain(DomainError::Validation(
                "this is the last unlocked admin; an instance with none has no way back to \
                 its own admin verbs, so provision or unlock another admin first"
                    .to_string(),
            )));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /teams/admin/accounts/{user_id}/lock` — admins only: lifts
/// the lock. The lock refused and took nothing away, and it stopped
/// no clock either, so what the server still holds for the account —
/// device tokens and sessions inside whatever life each has left, its
/// provider binding — resolves again; what the admin's revoke took
/// back before the lock stays taken. Whether a device presents the
/// token again is the client's business — the desktop's rule is on
/// its `was_unauthorized` — and the row is here either way. Recorded
/// as `unlocked`; an account not locked records nothing, decided as
/// the lock is.
async fn admin_unlock_account(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Path(user_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_admin(&account, "unlocking an account")?;
    let subject = subject_account(&ctx, &user_id).await?;
    let now = now_ms();
    if ctx.auth.unlock_account(subject.user_id).await? {
        ctx.auth
            .record_account_event(&account, Some(subject.user_id), "unlocked", now)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /teams/admin/accounts/{user_id}/events` — admins only: what
/// was done to this account and by whom, oldest first, including the
/// acts on every account that reached it too.
async fn admin_account_events(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Path(user_id): Path<String>,
) -> Result<Json<AccountEventsDto>, ApiError> {
    require_admin(&account, "reading an account's record")?;
    let subject = subject_account(&ctx, &user_id).await?;
    // One call, for the reason `account_page` gives.
    let (locked_at_ms, events) = ctx.auth.account_page(subject.user_id).await?;
    Ok(Json(AccountEventsDto {
        locked_at_ms,
        events: events.into_iter().map(account_event_dto).collect(),
    }))
}

/// `GET /teams/admin/events` — admins only: the instance's whole
/// record of acts on accounts, oldest first.
async fn admin_events(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
) -> Result<Json<AccountEventsDto>, ApiError> {
    require_admin(&account, "reading the instance's record")?;
    let events = ctx
        .auth
        .account_events()
        .await?
        .into_iter()
        .map(account_event_dto)
        .collect();
    Ok(Json(AccountEventsDto {
        locked_at_ms: None,
        events,
    }))
}

// ----------------------------------------------------------------------
// Handlers — team lifecycle.
// ----------------------------------------------------------------------

/// `POST /teams/create`. Registration policy first (#83 §1), then the
/// founding-owner rules: a user founds their own team; an admin —
/// never implicitly a member — must name the owner explicitly.
async fn create_team(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Json(cmd): Json<CreateTeamCommand>,
) -> Result<Json<TeamCreatedDto>, ApiError> {
    let creation_actor = if account.admin {
        CreationActor::Admin
    } else {
        CreationActor::AuthenticatedUser
    };
    if !may_create_team(creation_actor, ctx.registration) {
        return Err(ApiError::Forbidden(
            "registration is closed on this instance; only an admin may create teams".to_string(),
        ));
    }
    let owner_user_id = match (&cmd.owner_user_id, account.admin) {
        (Some(raw), true) => parse_uuid(raw, "owner_user_id")?,
        (None, true) => {
            return Err(ApiError::Domain(DomainError::Validation(
                "an admin is never implicitly a member (#83 §1); \
                 name the founding owner via owner_user_id"
                    .to_string(),
            )));
        }
        (Some(raw), false) => {
            let named = parse_uuid(raw, "owner_user_id")?;
            if named != account.user_id {
                return Err(ApiError::Domain(DomainError::Validation(
                    "a user founds their own team; owner_user_id may only name yourself"
                        .to_string(),
                )));
            }
            named
        }
        (None, false) => account.user_id,
    };
    if ctx.auth.account(owner_user_id).await?.is_none() {
        return Err(ApiError::Domain(DomainError::Validation(format!(
            "user {owner_user_id} has no account on this instance"
        ))));
    }
    let team_id = Uuid::now_v7();
    let founding_owner = Membership {
        user_id: owner_user_id,
        team_id,
        role: Role::Owner,
    };
    let capacity = if account.admin {
        Capacity::Admin
    } else {
        Capacity::Member
    };
    let event = ctx
        .repo
        .create_team(
            team_id,
            founding_owner,
            stamp(&account, capacity)?,
            now_ms(),
        )
        .await?;
    Ok(Json(TeamCreatedDto {
        team_id: team_id.to_string(),
        event: event_dto(&event)?,
    }))
}

/// `POST /teams/{team_id}/delete` — owner, or an admin
/// (admin-stamped, #83 §1).
async fn delete_team(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
) -> Result<Json<LedgerEventDto>, ApiError> {
    let capacity = decide(&access, TeamVerb::Delete)?;
    let event = ctx
        .repo
        .delete_team(access.team_id, stamp(&account, capacity)?, now_ms())
        .await?;
    Ok(Json(event_dto(&event)?))
}

// ----------------------------------------------------------------------
// Handlers — membership.
// ----------------------------------------------------------------------

/// `POST /teams/{team_id}/members/invite` — owner only.
async fn invite_member(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Json(cmd): Json<InviteMemberCommand>,
) -> Result<Json<LedgerEventDto>, ApiError> {
    let capacity = decide(&access, TeamVerb::Invite)?;
    let user_id = parse_uuid(&cmd.user_id, "user_id")?;
    let role = Role::parse(&cmd.role)?;
    if ctx.auth.account(user_id).await?.is_none() {
        return Err(ApiError::Domain(DomainError::Validation(format!(
            "user {user_id} has no account on this instance"
        ))));
    }
    let membership = Membership {
        user_id,
        team_id: access.team_id,
        role,
    };
    let event = ctx
        .repo
        .add_member(membership, stamp(&account, capacity)?, now_ms())
        .await?;
    Ok(Json(event_dto(&event)?))
}

/// `POST /teams/{team_id}/members/remove` — owner only. The last-owner
/// refusal comes back as `409` with state and stream untouched.
async fn remove_member(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Json(cmd): Json<RemoveMemberCommand>,
) -> Result<Json<LedgerEventDto>, ApiError> {
    let capacity = decide(&access, TeamVerb::Remove)?;
    let user_id = parse_uuid(&cmd.user_id, "user_id")?;
    let event = ctx
        .repo
        .remove_member(
            access.team_id,
            user_id,
            stamp(&account, capacity)?,
            now_ms(),
        )
        .await?;
    Ok(Json(event_dto(&event)?))
}

/// `POST /teams/{team_id}/members/leave` — any member, of themself
/// (#210).
///
/// Not in the authority table, and that is the point of it. The verbs
/// that act on somebody else's row ask whether the caller may; this
/// one asks nothing, because a member acting on their own membership
/// needs no authority over anyone. What it does need is a row to act
/// on, so a caller holding none is refused — see [`TeamAccess`] for
/// why that is the condition rather than any statement about who the
/// caller is.
///
/// The last owner cannot go, the same refusal removing them raises,
/// and the ledger appends [`MEMBERSHIP_REMOVED`], whose doc says how
/// an entry reads as a departure rather than a removal.
async fn leave_team(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
) -> Result<Json<LedgerEventDto>, ApiError> {
    if access.role.is_none() {
        return Err(ApiError::Forbidden(
            "you hold no membership in this team to leave".to_string(),
        ));
    }
    let event = ctx
        .repo
        .leave_team(
            access.team_id,
            account.user_id,
            stamp(&account, Capacity::Member)?,
            now_ms(),
        )
        .await?;
    Ok(Json(event_dto(&event)?))
}

/// `POST /teams/{team_id}/owners/grant` — owner only. The event
/// payload carries old + new (#83 §1).
async fn grant_owner(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Json(cmd): Json<GrantOwnerCommand>,
) -> Result<Json<LedgerEventDto>, ApiError> {
    change_role(
        &ctx,
        &account,
        &access,
        &cmd.user_id,
        Role::Owner,
        TeamVerb::GrantOwner,
    )
    .await
}

/// `POST /teams/{team_id}/owners/revoke` — owner only. Revoking the
/// last owner (including yourself) is the `409` the domain reserves
/// for it.
async fn revoke_owner(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Json(cmd): Json<RevokeOwnerCommand>,
) -> Result<Json<LedgerEventDto>, ApiError> {
    change_role(
        &ctx,
        &account,
        &access,
        &cmd.user_id,
        Role::Member,
        TeamVerb::RevokeOwner,
    )
    .await
}

/// Grant and revoke are one repository verb with different targets —
/// spelled once so the two routes cannot drift.
async fn change_role(
    ctx: &TeamsCtx,
    account: &AccountRecord,
    access: &TeamAccess,
    raw_user_id: &str,
    new_role: Role,
    verb: TeamVerb,
) -> Result<Json<LedgerEventDto>, ApiError> {
    let capacity = decide(access, verb)?;
    let user_id = parse_uuid(raw_user_id, "user_id")?;
    let event = ctx
        .repo
        .change_role(
            access.team_id,
            user_id,
            new_role,
            stamp(account, capacity)?,
            now_ms(),
        )
        .await?;
    Ok(Json(event_dto(&event)?))
}

// ----------------------------------------------------------------------
// Handlers — reads.
// ----------------------------------------------------------------------

/// `GET /teams` — the teams the caller is a member of.
///
/// The roster read turned around, and it sits outside [`team_gate`]
/// because it answers before a caller has a team to be gated on —
/// which is the whole reason it exists. Until it did, an app had no
/// way to offer a choice of team and made somebody type an id.
///
/// **Membership, not reach.** An admin acts inside any team without a
/// membership row (#83 §1), and this route does not widen for one: an
/// admin who is a member of nothing gets an empty list while retaining
/// every capacity they had. That is the honest answer to the question
/// as named, and the alternative — folding "teams I may act in" into
/// the same list — would make the route mean two things and match
/// neither. A surface that wants the admin's reach is asking a
/// different question and needs its own read.
async fn my_teams(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
) -> Result<Json<MyTeamsDto>, ApiError> {
    let teams = ctx.repo.teams_of_user(account.user_id).await?;
    Ok(Json(MyTeamsDto {
        teams: teams
            .into_iter()
            .map(|row| MyTeamDto {
                team_id: row.team_id.to_string(),
                role: row.role.as_str().to_string(),
                created_at_ms: row.created_at,
            })
            .collect(),
    }))
}

/// `GET /teams/{team_id}/roster` — the current membership state, and
/// what the caller may do in it.
async fn roster(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(access): Extension<TeamAccess>,
) -> Result<Json<RosterDto>, ApiError> {
    let roster = ctx.repo.roster(access.team_id).await?;
    Ok(Json(RosterDto {
        team_id: access.team_id.to_string(),
        members: roster
            .members()
            .iter()
            .map(|row| RosterMemberDto {
                user_id: row.user_id.to_string(),
                role: row.role.as_str().to_string(),
            })
            .collect(),
        // Said rather than left to be derived. The gate worked this
        // out to let the request through, and a caller with no row —
        // an admin — cannot be found in the rows at all.
        viewer: ViewerDto {
            role: access.role.map(|role| role.as_str().to_string()),
            admin: access.admin,
        },
    }))
}

/// Events per page when the caller does not say, and the most any
/// caller may ask for.
///
/// The ceiling is the load-bearing one: without it `?limit=` is a
/// request for the whole stream spelled differently, and the response
/// this route pages in order to bound comes back anyway.
const EVENTS_PAGE_DEFAULT: u32 = 100;
/// `pub(crate)` because this ceiling is not the events page's alone:
/// the forge's two bulk reads derive theirs from it rather than
/// picking their own, so changing this number moves them too. Why a
/// caller-sized read is bounded at all is argued where those two
/// define their bound, not here.
pub(crate) const EVENTS_PAGE_MAX: u32 = 500;

/// Query parameters of the events read.
#[derive(serde::Deserialize)]
struct EventsQuery {
    /// Resume above this seq. Absent means from the beginning.
    after: Option<i64>,
    /// How many events at most. Absent means
    /// [`EVENTS_PAGE_DEFAULT`]; anything outside
    /// `1..=`[`EVENTS_PAGE_MAX`] is clamped into it rather than
    /// refused, because a caller asking for more than the ceiling
    /// wants as much as it can have, and one asking for none is
    /// asking for a page it can do nothing with.
    limit: Option<u32>,
}

/// `GET /teams/{team_id}/events?after=<seq>&limit=<n>` — a page of the
/// team's stream in order. The per-member "who brought what" view
/// starts here (#91).
///
/// Paged, and a call with no parameters returns the first page rather
/// than the whole stream — a change of contract for anyone who was
/// reading the array this used to answer with. The reason it is worth
/// one is that the previous shape had no bound at all: a ledger only
/// grows, every team-scoped mutation appends to it, and the response
/// size was therefore a function of how long the team had existed.
/// Paging it before `forge.*` events start landing (#63) is cheaper
/// than paging it afterwards.
///
/// The cursor is a keyset over `seq`, which the storage's primary key
/// already orders, so a page costs the same wherever in the stream it
/// falls. What `next_after` means is
/// [`LedgerPageDto`]'s to say; what this handler owes it is the short
/// page, which is the only thing here that can end a walk.
async fn events(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(access): Extension<TeamAccess>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<LedgerPageDto>, ApiError> {
    // Clamped into `1..=MAX`, and the floor is load-bearing rather
    // than tidiness: at zero the repository answers with an empty page
    // that is nevertheless *full* — `len() == limit` holds at zero —
    // so the walk below would read a cursorless page off a stream with
    // events still in it, and say `next_after: null` about a position
    // nothing had been read up to. A caller asking for no events is
    // asking for a page it can do nothing with; it gets the smallest
    // one that keeps the cursor contract true.
    let limit = query
        .limit
        .unwrap_or(EVENTS_PAGE_DEFAULT)
        .clamp(1, EVENTS_PAGE_MAX);
    let events = ctx
        .repo
        .events_page(access.team_id, query.after, limit)
        .await?;
    // A full page may still be the last one, and the only way to know
    // is to ask again — so "shorter than asked for" is what ends the
    // walk, and a full page always carries a cursor.
    let next_after = (events.len() as u32 == limit)
        .then(|| events.last().map(|event| event.seq.get()))
        .flatten();
    let events = events
        .iter()
        .map(event_dto)
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(LedgerPageDto { events, next_after }))
}

/// Query parameters of the subject-filtered events read: the pair that
/// names a subject, plus [`EventsQuery`]'s cursor.
#[derive(serde::Deserialize)]
struct SubjectEventsQuery {
    /// The subject's kind, spelled as the ledger's index spells it —
    /// `forge_line`, `forge_pursuit`, `forge_thread`,
    /// `forge_identity`, `user`, `digest`, `blob`.
    #[serde(rename = "type")]
    ref_type: String,
    /// Its value in the same encoding: a hyphenated uuid, a
    /// `sha256:`-prefixed digest, or a forge handle's canonical form.
    value: String,
    /// Resume above this seq. Absent means from the beginning.
    after: Option<i64>,
    /// How many at most — clamped exactly as [`events`] clamps it.
    limit: Option<u32>,
}

/// `GET /teams/{team_id}/events/subject?type=<kind>&value=<v>&after=&limit=`
/// — the events that reference one subject, in order.
///
/// The trace query the repository has always been able to answer and
/// nothing could ask for (#83 §2). What made it worth exposing now is
/// what it can be asked *about*: the ledger carries forge subjects
/// since #150, so "everything that happened to this line" and
/// "everything that happened to this piece of work" are one request
/// each instead of a walk of the whole stream with a filter on the
/// client.
///
/// Its own route rather than a parameter on [`events`], because the
/// two are different questions with different costs, and a caller that
/// forgot the filter would get the whole stream back believing it had
/// asked for one subject's.
///
/// Same page contract as [`events`] — keyset over `seq`, a short page
/// ends the walk — and the same authority: a member may read their
/// team's stream, and reading a slice of it is not a different
/// permission.
///
/// **What is judged is the pair, and how much of it depends on the
/// kind.** A kind the ledger has no subject for is a `400`. The value
/// is then whatever the subject vocabulary makes of it, which is that
/// vocabulary's business rather than this route's and differs by
/// kind: a forge handle outside #102's set and an id that is not a
/// uuid are refused, because those kinds have a grammar, while a
/// digest that is not one is carried through and matches nothing.
/// Both endings are ones a caller can act on — a refusal, or an empty
/// page — and an empty page is the true answer for a well-formed
/// subject nothing references, which is why nothing here is a `404`.
///
/// This is deliberately *not* the have-check's rule for the same
/// notation, where a malformed digest is a `400`
/// ([`SqliteTeamsRepository::held_digests`](teams_infra::sqlite::repo::SqliteTeamsRepository::held_digests)).
/// There the answer decides whether a client skips a send, and a
/// nonsense digest silently reading as "not held" would be answered
/// wrongly; here it decides which events come back, and the honest
/// answer about a subject nothing wrote is none of them.
async fn events_for_subject(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(access): Extension<TeamAccess>,
    Query(query): Query<SubjectEventsQuery>,
) -> Result<Json<LedgerPageDto>, ApiError> {
    let subject = subject_from_ref(&query.ref_type, &query.value).map_err(|_| {
        // The storage helper's own message names the columns it was
        // built from, which is the wrong vocabulary out here: on this
        // side the pair is what the caller wrote.
        //
        // The pair, and not the kind alone: this refusal covers a kind
        // the ledger has no subject for *and* a value the kind's own
        // grammar rejects (a forge handle outside #102's set, an id
        // that is not a uuid). Naming only the kind would tell a
        // caller who sent `forge_identity` with a bad handle that
        // `forge_identity` is not a kind, in a sentence that goes on
        // to list it.
        ApiError::Domain(DomainError::Validation(format!(
            "no subject of this ledger's is written {:?} = {:?} — the kinds are \
             forge_line, forge_pursuit, forge_thread, forge_identity, user, digest \
             and blob, and each spells its value its own way: a hyphenated uuid, a \
             sha256: digest, or one of #102's forge handles",
            query.ref_type, query.value
        )))
    })?;
    let limit = query
        .limit
        .unwrap_or(EVENTS_PAGE_DEFAULT)
        .clamp(1, EVENTS_PAGE_MAX);
    let events = ctx
        .repo
        .events_for_subject_page(access.team_id, &subject, query.after, limit)
        .await?;
    let next_after = (events.len() as u32 == limit)
        .then(|| events.last().map(|event| event.seq.get()))
        .flatten();
    let events = events
        .iter()
        .map(event_dto)
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(LedgerPageDto { events, next_after }))
}

// ----------------------------------------------------------------------
// Handlers — blobs (#93, the #83 §3 mechanics).
// ----------------------------------------------------------------------

/// `PUT /teams/{team_id}/blobs?digest=sha256:<hex>` — members only.
///
/// The declared digest is mandatory (the OCI `PUT ?digest=` contract);
/// the body is the raw bytes, streamed frame by frame into the
/// adapter's staging write — never buffered whole. The full body is
/// **always** consumed and hashed, even when the CAS already holds the
/// digest: dedupe is server-side only, and a response (or a timing
/// difference) that skipped work would be the Harnik-2010 side channel
/// (#83 §3).
///
/// Ordering (#83 §3): the bytes are durable in the CAS *before* the
/// link row + `blob-copy completed` event commit in one transaction
/// (the #89 write API). A failure between the two leaves an orphan
/// blob — harmless, swept later — and never a dangling link. A digest
/// mismatch is a `409` carrying declared and computed, with no blob,
/// no link and no event behind it. A duplicate link (this team already
/// holds the digest) is the #89 repository's refusal, surfaced as the
/// `400` it already is — by then the body has been read in full, so
/// the refusal reveals only what the member could read anyway.
async fn upload_blob(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Query(cmd): Query<UploadBlobCommand>,
    body: Body,
) -> Result<Json<BlobUploadedDto>, ApiError> {
    let Some(raw_digest) = cmd.digest.as_deref() else {
        return Err(ApiError::Domain(DomainError::Validation(
            "the declared digest is mandatory: PUT /teams/{team_id}/blobs?digest=sha256:<hex> \
             (#83 §3 — promotion asserts \"content X\", so the claim travels with the bytes)"
                .to_string(),
        )));
    };
    let declared = DeclaredDigest::parse(raw_digest)?;
    // A membership row, not the admin capacity: §83 §1 gives an admin
    // delete and closed-registration create, nothing else implicit —
    // bringing content into a team's store is a member's act, stamped
    // as one.
    if access.role.is_none() {
        return Err(ApiError::Forbidden(
            "uploading into a team's store requires membership; an admin has no implicit upload"
                .to_string(),
        ));
    }
    let mut staged = ctx.blobs.begin_put().await?;
    let mut body = body;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|err| {
            // The client's stream broke mid-body; the staging write is
            // dropped (and removes its temp) on this return path.
            ApiError::Domain(DomainError::Validation(format!(
                "request body ended early: {err}"
            )))
        })?;
        if let Ok(data) = frame.into_data() {
            staged.write_chunk(&data).await?;
        }
    }
    // The gc guard's link phase spans the rename that makes the bytes
    // durable and the link row's commit: while any upload sits between
    // those two, the zero-link sweep must not decide (the racing
    // same-digest case — `teams_infra::gc`'s module doc). Shared, so
    // uploads never wait on each other.
    let _link_phase = ctx.gc_guard.link_phase().await;
    let verified = staged.commit(&declared).await?;
    let link = TeamBlobLink::new(access.team_id, verified.digest())?;
    let event = ctx
        .repo
        .add_blob_link(link, stamp(&account, Capacity::Member)?, now_ms())
        .await?;
    Ok(Json(BlobUploadedDto {
        digest: verified.digest().to_string(),
        event: event_dto(&event)?,
    }))
}

/// `GET /teams/{team_id}/blobs/{digest}` — the blob's bytes, streamed.
///
/// Visibility is the link boundary (#83 §3): the digest exists for
/// this caller iff a link row sits in this team and the caller may
/// read this team — a membership row, or the admin capacity. §1's
/// read boundary is general: an admin may read what a member may read
/// (roster, events, and blob bytes alike), because whoever holds that
/// capacity reaches the instance's disk and an HTTP-layer read denial
/// would be theater, not a boundary. Every way of not clearing the
/// bar — unknown team, no read capacity, unlinked digest, digest
/// linked elsewhere — is the same `404` (see the module doc's
/// exception note). A digest that fails to parse at all is the usual
/// `400`: the refusal is about the request's grammar and confirms
/// nothing.
///
/// Hits stream through the house `ReaderStream` pattern — 64 KiB
/// chunks, length from the open handle, `application/octet-stream` +
/// `nosniff` — never a whole-blob allocation.
async fn read_blob(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Path((team_id, raw_digest)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    let digest = parse_digest(&raw_digest)?;
    // The admin capacity, or a membership row (§1's general read
    // boundary). An unknown team reads as an empty roster, so the
    // membership probe covers "no such team" and "not a member" in
    // one motion — and the link table is consulted only for callers
    // who may read at all.
    let may_read = account.admin
        || ctx
            .repo
            .roster(team_id)
            .await?
            .role_of(account.user_id)
            .is_some();
    if !may_read || !ctx.repo.blob_link_exists(team_id, &digest).await? {
        return Err(ApiError::BlobNotFound);
    }
    let (file, length) = ctx.blobs.open_blob(&digest).await?.ok_or_else(|| {
        // By the #83 §3 ordering this cannot happen — the link row
        // commits only after the bytes are durable — so a miss here is
        // an invariant breach, not a 404.
        ApiError::Domain(DomainError::Infra(anyhow::anyhow!(
            "link row exists for {digest} but the CAS holds no bytes"
        )))
    })?;
    // 64 KiB chunks, the house choice (asterism-server's asset-file
    // route): a blob can be huge, and the 4 KiB default is a syscall
    // per page.
    let mut response = Response::new(Body::from_stream(ReaderStream::with_capacity(
        file,
        64 * 1024,
    )));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    // The CAS stores bytes, not types; `nosniff` keeps a browser from
    // promoting untyped bytes into something executable.
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    Ok(response)
}

// ----------------------------------------------------------------------
// Handlers — head registry (#132 phase 3).
// ----------------------------------------------------------------------

/// `PUT /teams/heads/registry` — admins only.
///
/// The body is a training run's head artifact, exactly as the member
/// app wrote it; the domain validates the envelope (one JSON object,
/// the `-v1` schema tag, a non-empty label, the encoder identity
/// fields) and keeps the bytes verbatim — the instance is a carrier,
/// not an authority, so nothing deeper is read here. Publishing
/// supersedes the live entry in the same transaction: one current
/// head per instance.
///
/// Admin, not owner: what scores for a whole team is an instance
/// concern — the same authority shape the model entry had, and
/// closed-registration create has.
async fn publish_head_registry(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    body: String,
) -> Result<Json<HeadPublishedDto>, ApiError> {
    if !account.admin {
        return Err(ApiError::Forbidden(
            "publishing a head is an admin's act; what scores for a team is an \
             instance concern (#132), not any one member's"
                .to_string(),
        ));
    }
    let entry = TagHeadEntry::parse(&body)?;
    let label = entry.label().to_string();
    let published_at_ms = now_ms();
    ctx.repo.publish_head_entry(entry, published_at_ms).await?;
    Ok(Json(HeadPublishedDto {
        label,
        published_at_ms,
    }))
}

/// `GET /teams/heads/registry` — any authenticated account; `404`
/// while nothing has been published.
///
/// Serves the publisher's bytes verbatim. The member app re-runs the
/// same verification its startup bind runs — encoder identity, row
/// widths, key shapes — before a pulled head may score; this route is
/// transport, which is why re-serialization has no place here: the
/// bytes a member checks must be bytes the publisher authored.
async fn head_registry(State(ctx): State<Arc<TeamsCtx>>) -> Result<Response, ApiError> {
    let entry = ctx
        .repo
        .current_head_entry()
        .await?
        .ok_or(ApiError::HeadRegistryEmpty)?;
    // A sized body, so hyper sets the length itself — unlike the blob
    // route's stream, which has to say it.
    let mut response = Response::new(Body::from(entry.raw().to_string()));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Ok(response)
}

// ----------------------------------------------------------------------
// Handlers — purge (#95, the #83 §3 lifecycle).
// ----------------------------------------------------------------------

/// `POST /teams/{team_id}/blobs/{digest}/purge/mark` — owner, or an
/// admin (admin-stamped).
///
/// The first half of the trash→purge two-step: from here the link is
/// hidden from normal reads — the blob route answers its one `404` for
/// it — but restorable via unmark until a reclaim takes it after the
/// grace window. The refusals ("not linked", "already marked") are the
/// repository's; they answer to an owner or an admin, who could read
/// the link state anyway.
async fn purge_mark(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team_id, digest)): Path<(Uuid, String)>,
) -> Result<Json<LedgerEventDto>, ApiError> {
    let capacity = decide(&access, TeamVerb::Purge)?;
    let event = ctx
        .repo
        .mark_blob_link_for_purge(
            access.team_id,
            &digest,
            stamp(&account, capacity)?,
            now_ms(),
        )
        .await?;
    Ok(Json(event_dto(&event)?))
}

/// `POST /teams/{team_id}/blobs/{digest}/purge/unmark` — owner, or an
/// admin (admin-stamped). Restores the marked link intact; the
/// grace window bounds reclaim, not restoration.
async fn purge_unmark(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team_id, digest)): Path<(Uuid, String)>,
) -> Result<Json<LedgerEventDto>, ApiError> {
    let capacity = decide(&access, TeamVerb::Purge)?;
    let event = ctx
        .repo
        .unmark_blob_link(
            access.team_id,
            &digest,
            stamp(&account, capacity)?,
            now_ms(),
        )
        .await?;
    Ok(Json(event_dto(&event)?))
}

/// `POST /teams/{team_id}/blobs/purge/reclaim` — owner, or an admin
/// (admin-stamped). The explicit second verb of the
/// two-step (#83 §3: reclaim is the only path that removes links for
/// reclaim's sake).
///
/// Removes the team's marked links whose grace window
/// ([`TeamsCtx::purge_grace_ms`]) has elapsed and appends the one
/// reclaim event; refused (`400`) while nothing is marked or every
/// mark is still inside its window. The zero-link sweep runs right
/// after, inside the same request — single-process — and the response
/// carries **how many** blobs it deleted, never which: the sweep is
/// instance-wide, so its digest list can name blobs the caller's team
/// never linked (orphans, another team's leftovers), and digest values
/// must not cross the team boundary on a surface that otherwise treats
/// digest existence as protected (the caller's own removals are named
/// in `removed_digests`). A sweep failure after the commit surfaces as
/// `500` — the reclaim itself is durable by then (the events route
/// shows it), and `teams-server gc` retries the sweep.
async fn purge_reclaim(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
) -> Result<Json<PurgeReclaimedDto>, ApiError> {
    let capacity = decide(&access, TeamVerb::Purge)?;
    let (removed_digests, event) = ctx
        .repo
        .reclaim_marked_links(
            access.team_id,
            ctx.purge_grace_ms,
            stamp(&account, capacity)?,
            now_ms(),
        )
        .await?;
    let swept = sweep_zero_link_blobs(&ctx.gc_guard, &ctx.repo, &ctx.blobs).await?;
    Ok(Json(PurgeReclaimedDto {
        removed_digests,
        swept: swept.len() as u64,
        event: event_dto(&event)?,
    }))
}

/// `GET /teams/{team_id}/blobs/purge/marked` — owner, or an admin
/// (admin-stamped authority, though a read stamps nothing): the
/// team's marked-for-purge set, with each mark's instant and when it
/// becomes reclaimable.
///
/// Same authority as the mark itself ([`TeamVerb::Purge`], so a plain
/// member's ask is the usual `403` — the convention every owner-only
/// verb follows): whoever may unmark must be able to see what is
/// marked. This is the surface the grace-visibility boundary (#83 §3)
/// *grants* — the mark hides a link from the normal reads, but inside
/// the team it is sayable state, and this route is where it is said.
async fn purge_marked_list(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(access): Extension<TeamAccess>,
) -> Result<Json<MarkedBlobsDto>, ApiError> {
    decide(&access, TeamVerb::Purge)?;
    let marked = ctx
        .repo
        .marked_blob_links(access.team_id)
        .await?
        .into_iter()
        .map(|(link, marked_at_ms)| MarkedBlobLinkDto {
            digest: link.digest().to_string(),
            marked_at_ms,
            reclaimable_at_ms: marked_at_ms.saturating_add(ctx.purge_grace_ms),
        })
        .collect();
    Ok(Json(MarkedBlobsDto {
        team_id: access.team_id.to_string(),
        marked,
    }))
}

// ----------------------------------------------------------------------
// Small helpers.
// ----------------------------------------------------------------------

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn parse_uuid(raw: &str, field: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| {
        ApiError::Domain(DomainError::Validation(format!(
            "{field} {raw:?} is not a UUID"
        )))
    })
}

/// Projects a domain envelope onto the wire shape. The subject columns
/// reuse the storage encoding (`teams-infra`'s `subject_to_ref`), so
/// the wire, the index table and the domain cannot spell a reference
/// three ways.
pub(crate) fn event_dto(event: &LedgerEvent) -> Result<LedgerEventDto, ApiError> {
    let (actor_kind, actor) = match &event.actor {
        LedgerActor::Member(stamp) => ("member", stamp),
        LedgerActor::Admin(stamp) => ("admin", stamp),
    };
    let subjects = event
        .subjects
        .iter()
        .map(|subject| {
            let (ref_type, value) = subject_to_ref(subject)?;
            Ok(SubjectRefDto { ref_type, value })
        })
        .collect::<Result<Vec<_>, DomainError>>()?;
    Ok(LedgerEventDto {
        seq: event.seq.get(),
        event_id: event.event_id.to_string(),
        team_id: event.team_id.to_string(),
        actor_kind: actor_kind.to_string(),
        actor_user_id: actor.user_id.to_string(),
        actor_display_name: actor.display_name.clone(),
        occurred_at_ms: event.occurred_at_ms,
        kind: event.kind.as_str().to_string(),
        subjects,
        payload_json: event.payload.to_string(),
    })
}
