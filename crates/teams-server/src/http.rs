//! HTTP transport — the axum `/teams/*` router (#83 §5, the #91
//! slice).
//!
//! ## Route table
//!
//! | Method | Path | Authority |
//! |---|---|---|
//! | POST | `/teams/auth/login` | none (rate-limited) |
//! | POST | `/teams/auth/logout` | bearer token (rate-limited) |
//! | POST | `/teams/create` | any authenticated user; admin-only under closed registration |
//! | POST | `/teams/{team_id}/delete` | owner, or an admin (admin-stamped) |
//! | GET | `/teams/{team_id}/roster` | member, or an admin |
//! | GET | `/teams/{team_id}/events` | member, or an admin — paged, see [`events`] |
//! | POST | `/teams/{team_id}/members/invite` | owner |
//! | POST | `/teams/{team_id}/members/remove` | owner |
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
//!
//! ## The gate (#83 §5: every route, no exceptions)
//!
//! Two middleware layers, in request order:
//!
//! 1. [`auth_gate`] — `Authorization: Bearer` token →
//!    [`PasswordAuth::resolve_session`] → [`AccountRecord`], inserted
//!    as an extension. Missing, malformed, unknown and **expired**
//!    tokens are all the same `401` (an expired row is deleted on
//!    touch).
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

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use http_body_util::BodyExt as _;
use teams_contract::command::{
    CreateTeamCommand, GrantOwnerCommand, InviteMemberCommand, LoginCommand, RemoveMemberCommand,
    RevokeOwnerCommand, UploadBlobCommand,
};
use teams_contract::dto::{
    BlobUploadedDto, HeadPublishedDto, LedgerEventDto, LedgerPageDto, MarkedBlobLinkDto,
    MarkedBlobsDto, PurgeReclaimedDto, RosterDto, RosterMemberDto, SessionDto, SubjectRefDto,
    TeamCreatedDto,
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
use teams_infra::auth::password::AccountRecord;
use teams_infra::gc::sweep_zero_link_blobs;
use teams_infra::sqlite::map::subject_to_ref;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::state::{TeamsCtx, now_ms};

/// HTTP-boundary error type. Same tagged body as `asterism-server`'s,
/// with the auth/authority outcomes this surface adds.
enum ApiError {
    /// A domain refusal, mapped by variant.
    Domain(DomainError),
    /// No token, a malformed header, an unknown token, or an expired
    /// one — deliberately indistinguishable.
    Unauthorized,
    /// Authenticated, but this verb is not yours here.
    Forbidden(String),
    /// The `{team_id}` names no team on this instance.
    TeamNotFound,
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
}

impl From<DomainError> for ApiError {
    fn from(err: DomainError) -> Self {
        Self::Domain(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, kind, message) = match self {
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
            Self::BlobNotFound => (
                StatusCode::NOT_FOUND,
                "NotFound",
                "no such blob in this team".to_string(),
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
struct AuthedAccount(AccountRecord);

/// What [`team_gate`] established about the caller in the `{team_id}`
/// team: their current role (from state), and whether the admin
/// capacity is available as a fallback.
#[derive(Clone)]
struct TeamAccess {
    team_id: Uuid,
    role: Option<Role>,
    admin: bool,
}

/// Which capacity a verb was granted under — what decides the ledger
/// stamp.
#[derive(Clone, Copy)]
enum Capacity {
    Member,
    Admin,
}

/// Builds the router; the caller binds a listener and calls
/// `axum::serve` (with connect-info, so the limiter sees client IPs).
pub fn router(ctx: Arc<TeamsCtx>) -> Router {
    let auth = Router::new()
        .route("/teams/auth/login", post(login))
        .route("/teams/auth/logout", post(logout))
        // One limiter over ALL auth endpoints (#83 §5) — the layer
        // wraps every route above it, and new auth routes belong in
        // this block so they inherit it.
        .layer(middleware::from_fn_with_state(ctx.clone(), auth_rate_limit))
        .with_state(ctx.clone());

    let team_scoped = Router::new()
        .route("/teams/{team_id}/delete", post(delete_team))
        .route("/teams/{team_id}/roster", get(roster))
        .route("/teams/{team_id}/events", get(events))
        .route("/teams/{team_id}/members/invite", post(invite_member))
        .route("/teams/{team_id}/members/remove", post(remove_member))
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
        .layer(middleware::from_fn_with_state(ctx.clone(), team_gate));

    let authed = Router::new()
        .route("/teams/create", post(create_team))
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
        .merge(team_scoped)
        .layer(middleware::from_fn_with_state(ctx.clone(), auth_gate))
        .with_state(ctx);

    Router::new().merge(auth).merge(authed)
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
fn decide(access: &TeamAccess, verb: TeamVerb) -> Result<Capacity, ApiError> {
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
    }
}

/// The ledger stamp for `account` acting under `capacity` — the one
/// place the member/admin variant is chosen (#83 §1: never disguised).
fn stamp(account: &AccountRecord, capacity: Capacity) -> Result<LedgerActor, ApiError> {
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
    Ok(Json(SessionDto {
        token,
        user_id: user_id.to_string(),
        display_name: account.display_name,
        admin: account.admin,
        expires_at_ms: now.saturating_add(ctx.session_ttl_ms),
    }))
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

/// `GET /teams/{team_id}/roster` — the current membership state.
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
    }))
}

/// Events per page when the caller does not say, and the most any
/// caller may ask for.
///
/// The ceiling is the load-bearing one: without it `?limit=` is a
/// request for the whole stream spelled differently, and the response
/// this route pages in order to bound comes back anyway.
const EVENTS_PAGE_DEFAULT: u32 = 100;
const EVENTS_PAGE_MAX: u32 = 500;

/// Query parameters of the events read.
#[derive(serde::Deserialize)]
struct EventsQuery {
    /// Resume above this seq. Absent means from the beginning.
    after: Option<i64>,
    /// How many events at most. Absent means
    /// [`EVENTS_PAGE_DEFAULT`]; anything above [`EVENTS_PAGE_MAX`] is
    /// clamped to it rather than refused, because a caller asking for
    /// more than the ceiling wants as much as it can have.
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
    let limit = query
        .limit
        .unwrap_or(EVENTS_PAGE_DEFAULT)
        .min(EVENTS_PAGE_MAX);
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
fn event_dto(event: &LedgerEvent) -> Result<LedgerEventDto, ApiError> {
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
