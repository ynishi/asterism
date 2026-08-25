//! The team's forge over HTTP — the local surface mirrored under
//! `/teams/{team_id}/forge/*`, plus the verbs hosting adds (#148
//! decisions 5 and 19).
//!
//! ## What "mirror" means here
//!
//! Same paths below the prefix, same DTOs, same handler form: the
//! request bodies are `asterism-contract`'s forge DTOs, the responses
//! are the same DTOs built by the same mappers, and a refusal comes
//! back through `asterism-server`'s status table down to the `reason`
//! token on a conflict.
//!
//! Three things differ, and each of them is what hosting *is*:
//!
//! 1. **The services are built per request** rather than held in the
//!    context, because each is built over
//!    [`TeamForge`](teams_infra::sqlite::forge::TeamForge), which
//!    carries the team and the capacity this request acts under
//!    (decision 20 and revision 6). A context-held service could carry
//!    neither.
//! 2. **The author is the authenticated member**, not a field of the
//!    command. See [`acting`].
//! 3. **Every route is behind the membership gate** — the merge in
//!    [`crate::http::router`] puts them there — and one write also
//!    asks the authority table. See below.
//!
//! ## Who may do what (#148 revision 5)
//!
//! Membership is the whole of the answer for every verb here but one.
//! Opening a line, renaming it, re-pointing its rule, moving its
//! standing, opening work, pushing a round, resolving, closing,
//! everything said in a thread and bringing content in are a member's
//! acts: they all leave a record, and anyone who can read the line can
//! recover from any of them. Discarding a line does not — it takes the
//! log with it — so that one verb consults the authority table and
//! wants an owner ([`TeamVerb::ForgeDiscard`]).
//!
//! Reads ask for nothing beyond the gate: a member reads their team's
//! forge, which is what having one hosted for them means.
//!
//! An **admin** standing outside the roster has no verb here at all,
//! not even the discard. #83 §1 hands an admin the destructive pair on
//! the team's own substrate — delete and purge — and nothing implicit
//! inside a team they are not in; working somebody's forge is further
//! inside than either.
//!
//! ## What is not here
//!
//! A projection *write* verb. Decision 12 rides a projection on the
//! push rather than giving it one, so that no second editing surface
//! grows beside the verbs. The read is here (`get_entry_projection`),
//! and so is the capture, on the push.
//!
//! A line's own reads are unpaged, exactly as the local surface leaves
//! them. `GET /lines/{id}` grows with the line's history and
//! `/states` folds it — the split the local surface already makes for
//! that reason — and putting a cursor on one of them here and not
//! there would be the first place the mirror stopped being one.

use std::sync::Arc;

use asterism_contract::forge::{
    AmendForgeMessageCommand, CloseForgePursuitCommand, ForgeCollisionDto, ForgeDiscardedDto,
    ForgeEntryStateDto, ForgeLineActCommand, ForgeLineDto, ForgeLineHistoryDto, ForgeMessageDto,
    ForgeOpDto, ForgePursuitActCommand, ForgePursuitDto, ForgeResolvedDto, ForgeRevisionDto,
    ForgeStrategyDto, ForgeThreadDto, OpenForgeLineCommand, OpenForgePursuitCommand,
    OpenForgeThreadCommand, PushForgeRoundCommand, RenameForgeLineCommand,
    RenameForgeThreadCommand, SayInForgeThreadCommand, SetForgeLineStrategyCommand,
};
use asterism_core::application::forge::{Anchored, LineService, PursuitService, ThreadService};
use asterism_core::application::mapping::{
    forge_anchored, forge_body, forge_collisions_to_dto, forge_discarded_to_dto,
    forge_history_to_dto, forge_line_id, forge_line_to_dto, forge_message_id, forge_message_to_dto,
    forge_name, forge_op, forge_outcome, forge_pursuit_id, forge_pursuit_to_dto,
    forge_revision_to_dto, forge_round_to_dto, forge_states_to_dto, forge_strategy_id,
    forge_strategy_to_dto, forge_thread_id, forge_thread_to_dto,
};
use asterism_core::domain::attribution::{AttributionContext, Author, OperatorRef};
use asterism_core::domain::forge::boundary::StoreClient;
use asterism_core::domain::forge::clock::SystemClock;
use asterism_core::domain::forge::model::pursuit::Intent;
use asterism_core::domain::forge::model::value::{LineId, PursuitId, ThreadId};
use asterism_core::domain::forge::strategies::Builtin;
use asterism_core::domain::value::AssetId;
use asterism_teams_wire::command::{
    EnterContentCommand, HaveContentCommand, ResolveContentCommand,
};
use asterism_teams_wire::dto::{
    ContentEnteredDto, HeldAssetDto, HeldContentDto, ResolvedContentDto,
};
use asterism_teams_wire::projection::{
    EntryProjectionDto, EntryProjectionEnvelope, WithProjections,
};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use http_body_util::BodyExt as _;
use teams_core::DomainError;
use teams_core::domain::identity::{LedgerActor, TeamVerb};
use teams_core::domain::projection::ProjectionBody;
use teams_core::domain::store::DeclaredDigest;
use teams_infra::auth::password::AccountRecord;
use teams_infra::sqlite::forge::TeamForge;
use uuid::Uuid;

use crate::http::{
    ApiError, AuthedAccount, Capacity, EVENTS_PAGE_MAX, TeamAccess, decide, event_dto, stamp,
};
use crate::state::{TeamsCtx, now_ms};

/// A write's result, or a refusal in the forge's own vocabulary.
type ForgeResult<T> = Result<Json<T>, ApiError>;

/// The forge's routes, without the gate — [`crate::http::router`]
/// merges them inside it.
///
/// The mirrored paths below the prefix are the local surface's,
/// verbatim. Read them against `asterism-server`'s router: a
/// difference between the two lists is a difference in the mirror,
/// which is the whole thing this route table promises not to have.
///
/// Four routes here are **not** mirrored, because hosting is what adds
/// them and the local plane has nowhere to put them: the three content
/// verbs at the bottom, and the projection read. Each says so where it
/// sits. A route added here without such a note is claiming to be a
/// mirror of something, and that claim is checkable.
pub(crate) fn routes() -> Router<Arc<TeamsCtx>> {
    Router::new()
        // A line.
        .route(
            "/teams/{team_id}/forge/lines",
            get(list_lines).post(open_line),
        )
        .route("/teams/{team_id}/forge/lines/{id}", get(get_line))
        .route(
            "/teams/{team_id}/forge/lines/{id}/states",
            get(get_line_states),
        )
        .route(
            "/teams/{team_id}/forge/lines/{id}/rename",
            post(rename_line),
        )
        .route(
            "/teams/{team_id}/forge/lines/{id}/strategy",
            post(set_line_strategy),
        )
        .route(
            "/teams/{team_id}/forge/lines/{id}/archive",
            post(archive_line),
        )
        .route(
            "/teams/{team_id}/forge/lines/{id}/reopen",
            post(reopen_line),
        )
        .route(
            "/teams/{team_id}/forge/lines/{id}/discard",
            post(discard_line),
        )
        // What a promoter said about an entry (#148 decision 12).
        // **Not a mirrored path** — the local plane stores no
        // projections, so there is nothing for this to mirror. Read
        // only: the write rides on the push, and `entries` is a static
        // segment under the line rather than a surface of its own, so
        // the shape says that a projection hangs off `(line, entry)`
        // and nothing else.
        .route(
            "/teams/{team_id}/forge/lines/{id}/entries/{entry}/projection",
            get(get_entry_projection),
        )
        .route("/teams/{team_id}/forge/strategies", get(list_strategies))
        // Work against a line.
        .route(
            "/teams/{team_id}/forge/lines/{id}/pursuits",
            get(list_pursuits_of_line),
        )
        .route("/teams/{team_id}/forge/pursuits", post(open_pursuit))
        .route("/teams/{team_id}/forge/pursuits/{id}", get(get_pursuit))
        .route(
            "/teams/{team_id}/forge/pursuits/{id}/push",
            post(push_round),
        )
        .route(
            "/teams/{team_id}/forge/pursuits/{id}/resolve",
            post(resolve_pursuit),
        )
        .route(
            "/teams/{team_id}/forge/pursuits/{id}/close",
            post(close_pursuit),
        )
        .route(
            "/teams/{team_id}/forge/pursuits/{id}/collisions",
            get(get_pursuit_collisions),
        )
        .route(
            "/teams/{team_id}/forge/pursuits/{id}/behind",
            get(get_pursuit_behind),
        )
        .route(
            "/teams/{team_id}/forge/pursuits/{id}/children",
            get(list_pursuit_children),
        )
        // What was said about work.
        .route("/teams/{team_id}/forge/threads", post(open_thread))
        .route("/teams/{team_id}/forge/threads/{id}", get(get_thread))
        .route("/teams/{team_id}/forge/threads/{id}/say", post(say))
        .route("/teams/{team_id}/forge/threads/{id}/amend", post(amend))
        .route(
            "/teams/{team_id}/forge/threads/{id}/rename",
            post(rename_thread),
        )
        .route(
            "/teams/{team_id}/forge/pursuits/{id}/threads",
            get(threads_about_pursuit),
        )
        .route(
            "/teams/{team_id}/forge/pursuits/{id}/rounds/{node}/threads",
            get(threads_about_round),
        )
        .route(
            "/teams/{team_id}/forge/pursuits/{id}/rounds/{node}/entries/{entry}/threads",
            get(threads_about_entry),
        )
        .route(
            "/teams/{team_id}/forge/lines/{id}/points/{point}/threads",
            get(threads_about_change),
        )
        // The verbs hosting adds (decision 19). `content` is a static
        // segment under `pursuits/{id}/`, and the two below it hang off
        // the prefix rather than off any one pursuit — they are asked
        // about a team's holdings, which is not a question one piece of
        // work has an answer to.
        .route(
            "/teams/{team_id}/forge/pursuits/{id}/content",
            put(enter_content),
        )
        .route(
            "/teams/{team_id}/forge/content/resolve",
            post(resolve_content),
        )
        .route("/teams/{team_id}/forge/content/have", post(have_content))
}

// ----------------------------------------------------------------------
// Wiring.
// ----------------------------------------------------------------------

/// The three services, over one team's forge, for one request.
struct Wired {
    lines: LineService,
    work: PursuitService,
    said: ThreadService,
}

/// Builds them.
///
/// Cheap, and per request because it has to be: the handle carries the
/// team and the [`LedgerActor`] whose capacity this request's events
/// are stamped with, and both are properties of the request rather
/// than of the store (#148 revision 6). The isle is the repository's
/// own, which is what puts a forge write and its ledger event in one
/// transaction (decision 17).
fn wire(ctx: &TeamsCtx, team_id: Uuid, actor: LedgerActor) -> Wired {
    let forge = TeamForge::for_request(ctx.repo.isle(), team_id, actor);
    let clock = Arc::new(SystemClock);
    let rules = Arc::new(Builtin::default());
    let lines: Arc<dyn asterism_core::domain::forge::lines::Lines> = Arc::new(forge.clone());
    let pursuits: Arc<dyn asterism_core::domain::forge::pursuits::Pursuits> =
        Arc::new(forge.clone());
    let closings: Arc<dyn asterism_core::domain::forge::closings::Closings> =
        Arc::new(forge.clone());
    let threads: Arc<dyn asterism_core::domain::forge::threads::Threads> = Arc::new(forge.clone());
    let actors: Arc<dyn asterism_core::domain::forge::boundary::Actors> = Arc::new(forge.clone());
    Wired {
        lines: LineService::new(
            lines.clone(),
            pursuits.clone(),
            rules.clone(),
            actors.clone(),
            clock.clone(),
        ),
        work: PursuitService::new(
            pursuits.clone(),
            lines.clone(),
            closings,
            rules,
            StoreClient::new(Arc::new(forge)),
            actors.clone(),
            clock.clone(),
        ),
        said: ThreadService::new(threads, pursuits, lines, actors, clock),
    }
}

/// What a member is granted here, and the pair the write needs: the
/// ledger's stamp and the forge's attribution.
///
/// Every write goes through this, which is why the two records
/// revision 6 separates cannot come apart: the **event** gets the
/// capacity the gate established, and the **forge node** gets who —
/// resolved from the same authenticated account, never from the
/// command.
fn writing(
    ctx: &TeamsCtx,
    account: &AccountRecord,
    access: &TeamAccess,
    verb: TeamVerb,
    stated_operator: Option<&str>,
    stated_author: (Option<&str>, Option<&str>),
) -> Result<(Wired, AttributionContext), ApiError> {
    let capacity = decide(access, verb)?;
    let actor = stamp(account, capacity)?;
    let by = acting(account, stated_operator, stated_author)?;
    Ok((wire(ctx, access.team_id, actor), by))
}

/// The forge handle for a read: no verb to ask about, and nothing this
/// stamp will ever be written into.
///
/// The handle carries a [`LedgerActor`] because the type does, so it
/// gets the one a write from this request would have used — the
/// membership row when there is one, the admin capacity when the
/// caller is reading from outside the roster (#83 §1's general read
/// boundary). No read path here appends, so the value is inert; what
/// it must not be is a *member* stamp for somebody who is not one,
/// which is why the choice is made rather than defaulted.
fn read_handle(
    ctx: &TeamsCtx,
    account: &AccountRecord,
    access: &TeamAccess,
) -> Result<TeamForge, ApiError> {
    Ok(TeamForge::for_request(
        ctx.repo.isle(),
        access.team_id,
        stamp(account, read_capacity(access))?,
    ))
}

/// The same for the three services, which is what most reads want.
fn reading(
    ctx: &TeamsCtx,
    account: &AccountRecord,
    access: &TeamAccess,
) -> Result<Wired, ApiError> {
    Ok(wire(
        ctx,
        access.team_id,
        stamp(account, read_capacity(access))?,
    ))
}

/// Which capacity a read is happening under.
///
/// Spelled once because the two read paths above must not drift: the
/// membership row when there is one, and the admin capacity only for
/// the caller who has no row — which is the same order [`decide`]
/// applies to a write, and the reason it is that order is #83 §1's
/// ("an admin who is also a member acts through the row like anyone
/// else").
const fn read_capacity(access: &TeamAccess) -> Capacity {
    if access.role.is_some() {
        Capacity::Member
    } else {
        Capacity::Admin
    }
}

/// The attribution a forge write on this plane records.
///
/// **The author is the authenticated member, and a command may not say
/// otherwise.** On the local surface the three attribution fields are
/// the caller's own statement about itself, believed and labelled as
/// such (`asterism-server`'s `asserted`). Here the gate has already
/// resolved who is asking, so a caller-stated author would be a second
/// answer to a question that is settled — and the one shape #83 §1
/// forbids outright is a write that says it was somebody else's. So
/// `author_kind` and `author_subject` are **refused** rather than
/// ignored: a request that carries them was written against the wrong
/// plane, and silently overwriting them would let a client believe a
/// name it chose had landed.
///
/// The subject is the account's user id rather than its display name.
/// Decision 6 puts author subjects and viewer subjects in one
/// namespace, and a display name moves; the ledger event beside this
/// write is where the name at write time is kept (revision 9).
///
/// `operator_ai` **is** taken from the command, and it is the one that
/// has to be: which agent drove the request is something only the
/// caller knows.
fn acting(
    account: &AccountRecord,
    stated_operator: Option<&str>,
    stated_author: (Option<&str>, Option<&str>),
) -> Result<AttributionContext, ApiError> {
    if stated_author.0.is_some() || stated_author.1.is_some() {
        return Err(ApiError::Domain(DomainError::Validation(
            "author_kind and author_subject are not read on a team's forge: the author is \
             the authenticated member (#148 revision 6). Send operator_ai alone."
                .to_string(),
        )));
    }
    let operator = stated_operator
        .map(OperatorRef::new)
        .transpose()
        .map_err(ApiError::Forge)?;
    AttributionContext::asserted(Some(Author::Subject(account.user_id.to_string())), operator)
        .map_err(ApiError::Forge)
}

/// Reads a line id out of a path segment. The service takes typed ids
/// and the wire carries strings, so the parsing is the adapter's job —
/// the local surface's note applies here unchanged.
fn line_id(raw: &str) -> Result<LineId, ApiError> {
    Ok(forge_line_id(raw, "line id")?)
}

/// Reads a pursuit id out of the path.
fn pursuit_id(raw: &str) -> Result<PursuitId, ApiError> {
    Ok(forge_pursuit_id(raw, "pursuit id")?)
}

/// Reads a thread id out of the path.
fn thread_id(raw: &str) -> Result<ThreadId, ApiError> {
    Ok(forge_thread_id(raw, "thread id")?)
}

// ----------------------------------------------------------------------
// A line.
// ----------------------------------------------------------------------

/// `GET /teams/{team_id}/forge/lines` — every line this team hosts,
/// without its history.
///
/// Scoped by the adapter and by nothing in this signature, which is
/// the seat `Lines::list` reserves for a hosting layer (#148 decision
/// 1): the forge does not know what a person is, this plane does.
async fn list_lines(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
) -> ForgeResult<Vec<ForgeLineDto>> {
    let wired = reading(&ctx, &account, &access)?;
    let lines = wired.lines.list().await?;
    Ok(Json(lines.iter().map(forge_line_to_dto).collect()))
}

/// `POST /teams/{team_id}/forge/lines` — opens a line.
///
/// A member's act (revision 5). The name is unique within the team,
/// which is the question the forge's `Name` leaves to whoever owns the
/// namespace — a second line by the same name is the adapter's `409`.
async fn open_line(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Json(command): Json<OpenForgeLineCommand>,
) -> ForgeResult<ForgeLineDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let line = wired
        .lines
        .open(
            forge_name(command.name)?,
            forge_strategy_id(command.strategy_id)?,
            &by,
        )
        .await?;
    Ok(Json(forge_line_to_dto(&line)))
}

/// `GET /teams/{team_id}/forge/lines/{id}` — the line and its whole
/// history.
async fn get_line(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
) -> ForgeResult<ForgeLineHistoryDto> {
    let wired = reading(&ctx, &account, &access)?;
    let line = wired.lines.get(&line_id(&id)?).await?;
    Ok(Json(forge_history_to_dto(&line)))
}

/// `GET /teams/{team_id}/forge/lines/{id}/states` — what is on the
/// line, folded from the chain.
async fn get_line_states(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
) -> ForgeResult<Vec<ForgeEntryStateDto>> {
    let wired = reading(&ctx, &account, &access)?;
    let states = wired.lines.states(&line_id(&id)?).await?;
    Ok(Json(forge_states_to_dto(&states)))
}

/// Reads a line back after a write, so the caller sees what it now is
/// — the local surface's `line_now`, and its reasoning applies
/// unchanged.
async fn line_now(wired: &Wired, id: &LineId) -> ForgeResult<ForgeLineDto> {
    Ok(Json(forge_line_to_dto(&wired.lines.get(id).await?)))
}

/// `POST /teams/{team_id}/forge/lines/{id}/rename` — moves the line's
/// own description. Not a landing: nothing goes on the chain.
async fn rename_line(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Json(command): Json<RenameForgeLineCommand>,
) -> ForgeResult<ForgeLineDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let id = line_id(&id)?;
    wired
        .lines
        .rename(&id, &forge_name(command.name)?, &by)
        .await?;
    line_now(&wired, &id).await
}

/// `POST /teams/{team_id}/forge/lines/{id}/strategy` — points the line
/// at a different rule, from here on.
async fn set_line_strategy(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Json(command): Json<SetForgeLineStrategyCommand>,
) -> ForgeResult<ForgeLineDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let id = line_id(&id)?;
    wired
        .lines
        .set_strategy(&id, &forge_strategy_id(command.strategy_id)?, &by)
        .await?;
    line_now(&wired, &id).await
}

/// `POST /teams/{team_id}/forge/lines/{id}/archive` — finished with.
async fn archive_line(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Json(command): Json<ForgeLineActCommand>,
) -> ForgeResult<ForgeLineDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let id = line_id(&id)?;
    wired.lines.archive(&id, &by).await?;
    line_now(&wired, &id).await
}

/// `POST /teams/{team_id}/forge/lines/{id}/reopen` — takes it back
/// out.
async fn reopen_line(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Json(command): Json<ForgeLineActCommand>,
) -> ForgeResult<ForgeLineDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let id = line_id(&id)?;
    wired.lines.reopen(&id, &by).await?;
    line_now(&wired, &id).await
}

/// `POST /teams/{team_id}/forge/lines/{id}/discard` — takes the line,
/// its history and every piece of work against it.
///
/// **The one verb on this surface that asks more than membership**
/// (revision 5): it is the verb that takes the log with it, so an
/// owner is what it wants. The response is the point for the local
/// surface's reason — it names the assets the forge was holding and is
/// not holding any more, and after this write there is no record left
/// to derive them from.
///
/// What it does *not* do is release the team's copies of those bytes.
/// The assets it names stop being held by any line, and the `team_asset`
/// rows and the blobs under them stay: reclaiming a team's storage is
/// the purge two-step (#95), a separate verb with a grace window of its
/// own. Releasing is not deleting, which is the order the local plane
/// keeps too.
async fn discard_line(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Json(command): Json<ForgeLineActCommand>,
) -> ForgeResult<ForgeDiscardedDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeDiscard,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let id = line_id(&id)?;
    let released = wired.lines.discard(&id, &by).await?;
    Ok(Json(forge_discarded_to_dto(id, &released)))
}

/// `GET /teams/{team_id}/forge/strategies` — every rule a line can be
/// pointed at.
///
/// The rules this deployment carries, which are `asterism-core`'s
/// built-ins: a collision rule is not storage, so hosting the forge
/// did not give the team a set of its own (#148 decision 20).
async fn list_strategies(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
) -> ForgeResult<Vec<ForgeStrategyDto>> {
    let wired = reading(&ctx, &account, &access)?;
    let rules = wired.lines.strategies().await;
    Ok(Json(
        rules
            .iter()
            .map(|(id, about)| forge_strategy_to_dto(id, about))
            .collect(),
    ))
}

// ----------------------------------------------------------------------
// Work against a line.
// ----------------------------------------------------------------------

/// Reads work back after a write — the local surface's `pursuit_now`.
async fn pursuit_now(wired: &Wired, id: &PursuitId) -> ForgeResult<ForgePursuitDto> {
    Ok(Json(forge_pursuit_to_dto(&wired.work.get(id).await?)))
}

/// `POST /teams/{team_id}/forge/pursuits` — opens work against a line.
///
/// The line is in the body rather than the path, because this is the
/// one verb here with no pursuit to name yet.
async fn open_pursuit(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Json(command): Json<OpenForgePursuitCommand>,
) -> ForgeResult<ForgePursuitDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let line = line_id(&command.line_id)?;
    let parent = command.parent_id.as_deref().map(pursuit_id).transpose()?;
    let intent = Intent {
        title: command.title.map(forge_name).transpose()?,
        note: command.note,
    };
    let pursuit = wired.work.open(&line, parent, intent, &by).await?;
    Ok(Json(forge_pursuit_to_dto(&pursuit)))
}

/// `GET /teams/{team_id}/forge/pursuits/{id}` — the work, whole.
async fn get_pursuit(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
) -> ForgeResult<ForgePursuitDto> {
    let wired = reading(&ctx, &account, &access)?;
    pursuit_now(&wired, &pursuit_id(&id)?).await
}

/// `POST /teams/{team_id}/forge/pursuits/{id}/push` — writes a round,
/// and captures whatever descriptions rode in with it.
///
/// What it checks is that the content each operation names exists —
/// and on this plane "exists" means *this team has it*
/// (`TeamForge`'s `Store`). So a round naming content another team
/// holds is refused before it is written, and the foreign key under
/// the row is the backstop rather than the answer. That is also the
/// ordering decision 5 keeps: content is there before the round that
/// names it, because a round must not name what the instance does not
/// hold.
///
/// ## The projection rides here rather than getting a verb
///
/// Decision 19 says so, and decision 12 says why: only a forge op
/// replaces a projection, so no second editing surface grows beside
/// the verbs. The body is a
/// [`WithProjections`] wrapper over the mirror's own command, which
/// flattens — a push sent by a caller that knows nothing about
/// projections is byte-for-byte the request it always was, so the
/// route's shape below the prefix still matches the local surface's.
///
/// What an envelope is checked for is in [`describing`], and none of
/// it is inside the body (decision 14). The line and the team are not
/// checked at all — they are taken from the pursuit and from the gate,
/// because a client-stated one would be a second answer to a question
/// those already settle.
///
/// **The capture happens after the push succeeded, in its own
/// transaction, and cannot fail the push.** Decision 12 makes a
/// projection something that can be lost without the line lying, which
/// is what licenses the second write. The ordering forecloses the
/// failure that is *not* permitted — a description of a round that was
/// refused — and the swallowed error below forecloses the other one, a
/// caller told that a round it can see on the line did not land.
async fn push_round(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Json(body): Json<WithProjections<PushForgeRoundCommand>>,
) -> ForgeResult<ForgePursuitDto> {
    let WithProjections {
        push: command,
        projections,
    } = body;
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let id = pursuit_id(&id)?;
    let described = describing(&projections, &command.ops)?;
    let ops = command
        .ops
        .iter()
        .map(forge_op)
        .collect::<Result<Vec<_>, _>>()?;
    wired.work.push(&id, ops, command.note, &by).await?;

    if !described.is_empty() {
        let pursuit = wired.work.get(&id).await?;
        let line = *pursuit.of().as_uuid();
        // Deliberately not `?`. The round is committed by the time
        // this runs, and a `?` here would answer a landed push with a
        // failure — which is the one thing the caller must not be
        // told, because it would retry and mint a second TeamAsset and
        // push a second round for content that is already on the line.
        // What is actually lost when this fails is the description,
        // and decision 12 makes that a loss the line survives. So the
        // push answers for the push, and the failure goes to stderr
        // where this binary already writes.
        if let Err(err) = ctx
            .projections
            .capture(access.team_id, line, account.user_id, now_ms(), described)
            .await
        {
            eprintln!(
                "teams-server: the round on line {line} landed and its projections did not: \
                 {err}"
            );
        }
    }
    pursuit_now(&wired, &id).await
}

/// Reads the envelopes a push carried, refusing any that names an
/// entry the push does not operate on.
///
/// The refusal is about the *key*, not the contents: a projection is
/// keyed `(line, entry)`, the line is the pursuit's, and the entry has
/// to be one this round actually touched or the push becomes a way to
/// write over any entry on the line. Nothing here opens a body — the
/// only thing asked of one is that it is not empty and not past the
/// ceiling, which
/// [`ProjectionBody::parse`](teams_core::domain::projection::ProjectionBody::parse)
/// answers without reading it.
fn describing(
    envelopes: &[EntryProjectionEnvelope],
    ops: &[ForgeOpDto],
) -> Result<Vec<(Uuid, u32, ProjectionBody)>, ApiError> {
    envelopes
        .iter()
        .map(|envelope| {
            if !ops.iter().any(|op| op.entry_id == envelope.entry_id) {
                return Err(ApiError::Domain(DomainError::Validation(format!(
                    "a projection describes an entry the round it rides on operates on, \
                     and no operation here names {:?}",
                    envelope.entry_id
                ))));
            }
            let entry = Uuid::parse_str(&envelope.entry_id).map_err(|_| {
                ApiError::Domain(DomainError::Validation(format!(
                    "entry id {:?} is not a UUID",
                    envelope.entry_id
                )))
            })?;
            Ok((
                entry,
                envelope.version,
                ProjectionBody::parse(envelope.body.clone())?,
            ))
        })
        .collect()
}

/// `GET /teams/{team_id}/forge/lines/{id}/entries/{entry}/projection`
/// — what a promoter said about one entry.
///
/// **The gate proves membership of the team in the path, and the read
/// is scoped to that same team.** Both halves are needed and only the
/// first is automatic: a line id is unique across teams, so
/// `(line, entry)` alone finds whichever team's row exists, and a
/// member of any team who learned another team's ids would read its
/// promoter's description. So `access.team_id` goes to the store, the
/// way every other line-scoped read here reaches
/// `TeamForge::for_request`, and a row belonging to another team
/// answers as absent.
///
/// **The one read this projection has**, and there is deliberately no
/// list: a description is looked up while looking at an entry, and a
/// route returning every projection on a line would be a second shape
/// for reading what the line's own reads already enumerate.
///
/// A missing projection is a `404` and an ordinary answer (decision
/// 12). The body comes back verbatim — this handler does not parse it,
/// and the DTO it travels in names nothing inside it (decision 14).
async fn get_entry_projection(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, line, entry)): Path<(Uuid, String, String)>,
) -> ForgeResult<EntryProjectionDto> {
    let line = *line_id(&line)?.as_uuid();
    let entry = Uuid::parse_str(&entry).map_err(|_| {
        ApiError::Domain(DomainError::Validation(format!(
            "entry id {entry:?} is not a UUID"
        )))
    })?;
    let found = ctx
        .projections
        .find(access.team_id, line, entry)
        .await?
        .ok_or(ApiError::ProjectionNotFound)?;
    Ok(Json(EntryProjectionDto {
        line_id: found.line_id.to_string(),
        entry_id: found.entry_id.to_string(),
        version: found.version,
        body: found.body.as_str().to_string(),
        promoted_by: found.promoted_by.to_string(),
        pushed_at_ms: found.pushed_at_ms,
    }))
}

/// `POST /teams/{team_id}/forge/pursuits/{id}/resolve` — lets the
/// line's rule answer whatever this work collides with. 200 whether or
/// not a round was written, and the body says which.
async fn resolve_pursuit(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Json(command): Json<ForgePursuitActCommand>,
) -> ForgeResult<ForgeResolvedDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let id = pursuit_id(&id)?;
    let round = wired.work.resolve(&id, &by).await?;
    let collisions = wired.work.collisions(&id).await?;
    Ok(Json(ForgeResolvedDto {
        round: round.as_ref().map(forge_round_to_dto),
        collisions: forge_collisions_to_dto(&collisions),
    }))
}

/// `POST /teams/{team_id}/forge/pursuits/{id}/close` — ends the work,
/// and puts what it says on the line if it says anything.
///
/// The four conflict reasons are the local surface's and mean the same
/// things here: `blocked`, `raced`, `settled`, `clashes`.
async fn close_pursuit(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Json(command): Json<CloseForgePursuitCommand>,
) -> ForgeResult<ForgePursuitDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let id = pursuit_id(&id)?;
    let outcome = forge_outcome(&command.outcome)?;
    wired.work.close(&id, outcome, command.note, &by).await?;
    pursuit_now(&wired, &id).await
}

/// `GET /teams/{team_id}/forge/pursuits/{id}/collisions` — what this
/// work still asks for that the line has moved since.
async fn get_pursuit_collisions(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
) -> ForgeResult<Vec<ForgeCollisionDto>> {
    let wired = reading(&ctx, &account, &access)?;
    let found = wired.work.collisions(&pursuit_id(&id)?).await?;
    Ok(Json(forge_collisions_to_dto(&found)))
}

/// `GET /teams/{team_id}/forge/pursuits/{id}/behind` — the landings
/// this work has not seen, oldest first.
async fn get_pursuit_behind(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
) -> ForgeResult<Vec<String>> {
    let wired = reading(&ctx, &account, &access)?;
    let behind = wired.work.behind(&pursuit_id(&id)?).await?;
    Ok(Json(behind.iter().map(ToString::to_string).collect()))
}

/// `GET /teams/{team_id}/forge/pursuits/{id}/children` — work opened
/// from this work.
async fn list_pursuit_children(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
) -> ForgeResult<Vec<ForgePursuitDto>> {
    let wired = reading(&ctx, &account, &access)?;
    let found = wired.work.children(&pursuit_id(&id)?).await?;
    Ok(Json(found.iter().map(forge_pursuit_to_dto).collect()))
}

/// `GET /teams/{team_id}/forge/lines/{id}/pursuits` — every piece of
/// work against a line, open and ended alike.
async fn list_pursuits_of_line(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
) -> ForgeResult<Vec<ForgePursuitDto>> {
    let wired = reading(&ctx, &account, &access)?;
    let found = wired.work.of_line(&line_id(&id)?).await?;
    Ok(Json(found.iter().map(forge_pursuit_to_dto).collect()))
}

// ----------------------------------------------------------------------
// What was said about work.
// ----------------------------------------------------------------------

/// Reads a conversation back after a write — the local surface's
/// `thread_now`, and `say` and `amend` do not use it there either.
async fn thread_now(wired: &Wired, id: &ThreadId) -> ForgeResult<ForgeThreadDto> {
    Ok(Json(forge_thread_to_dto(&wired.said.get(id).await?)))
}

/// `POST /teams/{team_id}/forge/threads` — opens a conversation about
/// something in the forge. The anchor is resolved rather than
/// accepted, and an id the kind has no use for is refused.
async fn open_thread(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Json(command): Json<OpenForgeThreadCommand>,
) -> ForgeResult<ForgeThreadDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let about = forge_anchored(
        &command.anchor_kind,
        command.pursuit_id.as_deref(),
        command.line_id.as_deref(),
        command.node_id.as_deref(),
        command.entry_id.as_deref(),
        command.change_point_id.as_deref(),
    )?;
    let title = command.title.map(forge_name).transpose()?;
    let thread = wired
        .said
        .open(about, title, forge_body(command.said)?, &by)
        .await?;
    Ok(Json(forge_thread_to_dto(&thread)))
}

/// `GET /teams/{team_id}/forge/threads/{id}` — the conversation,
/// whole: every message and every correction to each.
async fn get_thread(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
) -> ForgeResult<ForgeThreadDto> {
    let wired = reading(&ctx, &account, &access)?;
    thread_now(&wired, &thread_id(&id)?).await
}

/// `POST /teams/{team_id}/forge/threads/{id}/say` — says something,
/// and answers with the message it wrote.
async fn say(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Json(command): Json<SayInForgeThreadCommand>,
) -> ForgeResult<ForgeMessageDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let id = thread_id(&id)?;
    let replying_to = command
        .replying_to
        .as_deref()
        .map(|raw| forge_message_id(raw, "replying_to"))
        .transpose()?;
    let said = wired
        .said
        .say(&id, replying_to, forge_body(command.said)?, &by)
        .await?;
    Ok(Json(forge_message_to_dto(&said)))
}

/// `POST /teams/{team_id}/forge/threads/{id}/amend` — corrects
/// something said, and answers with the correction rather than with
/// the message as it now reads.
async fn amend(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Json(command): Json<AmendForgeMessageCommand>,
) -> ForgeResult<ForgeRevisionDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let id = thread_id(&id)?;
    let message = forge_message_id(&command.message_id, "message id")?;
    let revision = wired
        .said
        .amend(&id, &message, forge_body(command.said)?, &by)
        .await?;
    Ok(Json(forge_revision_to_dto(&revision)))
}

/// `POST /teams/{team_id}/forge/threads/{id}/rename` — names the
/// conversation, or takes its name off. Writes no message.
async fn rename_thread(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Json(command): Json<RenameForgeThreadCommand>,
) -> ForgeResult<ForgeThreadDto> {
    let (wired, by) = writing(
        &ctx,
        &account,
        &access,
        TeamVerb::ForgeWork,
        command.operator_ai.as_deref(),
        (
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
        ),
    )?;
    let id = thread_id(&id)?;
    let title = command.title.map(forge_name).transpose()?;
    wired.said.rename(&id, title.as_ref(), &by).await?;
    thread_now(&wired, &id).await
}

/// Answers one of the four `about` reads. More than one conversation
/// can hang off the same thing, so every one of these answers a list.
async fn threads_about(wired: &Wired, about: Anchored) -> ForgeResult<Vec<ForgeThreadDto>> {
    let found = wired.said.about(about).await?;
    Ok(Json(found.iter().map(forge_thread_to_dto).collect()))
}

/// `GET /teams/{team_id}/forge/pursuits/{id}/threads` — about the work
/// as a whole.
async fn threads_about_pursuit(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
) -> ForgeResult<Vec<ForgeThreadDto>> {
    let wired = reading(&ctx, &account, &access)?;
    threads_about(&wired, Anchored::Pursuit(pursuit_id(&id)?)).await
}

/// `GET /teams/{team_id}/forge/pursuits/{id}/rounds/{node}/threads` —
/// about one round of it.
async fn threads_about_round(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id, node)): Path<(Uuid, String, String)>,
) -> ForgeResult<Vec<ForgeThreadDto>> {
    let wired = reading(&ctx, &account, &access)?;
    let about = forge_anchored("round", Some(&id), None, Some(&node), None, None)?;
    threads_about(&wired, about).await
}

/// `GET
/// /teams/{team_id}/forge/pursuits/{id}/rounds/{node}/entries/{entry}/threads`
/// — about one entry, as that round had it.
async fn threads_about_entry(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id, node, entry)): Path<(Uuid, String, String, String)>,
) -> ForgeResult<Vec<ForgeThreadDto>> {
    let wired = reading(&ctx, &account, &access)?;
    let about = forge_anchored("entry", Some(&id), None, Some(&node), Some(&entry), None)?;
    threads_about(&wired, about).await
}

/// `GET /teams/{team_id}/forge/lines/{id}/points/{point}/threads` —
/// about what landed on the line.
async fn threads_about_change(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id, point)): Path<(Uuid, String, String)>,
) -> ForgeResult<Vec<ForgeThreadDto>> {
    let wired = reading(&ctx, &account, &access)?;
    let about = forge_anchored("change", None, Some(&id), None, None, Some(&point))?;
    threads_about(&wired, about).await
}

// ----------------------------------------------------------------------
// Content — the verbs hosting adds (#148 decisions 5 and 19).
// ----------------------------------------------------------------------

/// `PUT /teams/{team_id}/forge/pursuits/{id}/content?digest=sha256:<hex>`
/// — brings content into the team against open work.
///
/// **The entry point is a forge op, and there is exactly one**
/// (decision 5). The standalone `PUT /teams/{team_id}/blobs` stays
/// where it is as the substrate's own upload; what a member promoting
/// into a line uses is this, and the difference is what the write
/// leaves behind: a `TeamAsset` a round can name, attached to the work
/// it arrived against.
///
/// **The byte path is the upload's, unchanged**, down to the streaming,
/// the always-hash-the-whole-body rule and the gc guard's span:
/// [`crate::http::upload_blob`] is where those are argued (#83 §3,
/// #93), and this route re-derives none of it so that changing them
/// there changes them here. What this route adds after the bytes are
/// durable is the rows below.
///
/// A membership row, not the admin capacity, for the upload's reason:
/// bringing content into a team is a member's act.
async fn enter_content(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Path((_team, id)): Path<(Uuid, String)>,
    Query(command): Query<EnterContentCommand>,
    body: Body,
) -> ForgeResult<ContentEnteredDto> {
    let Some(raw_digest) = command.digest.as_deref() else {
        return Err(ApiError::Domain(DomainError::Validation(
            "the declared digest is mandatory: \
             PUT /teams/{team_id}/forge/pursuits/{id}/content?digest=sha256:<hex> \
             (#83 §3 — promotion asserts \"content X\", so the claim travels with the bytes)"
                .to_string(),
        )));
    };
    let declared = DeclaredDigest::parse(raw_digest)?;
    let capacity = decide(&access, TeamVerb::ForgeWork)?;
    let pursuit = pursuit_id(&id)?;

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
    let _link_phase = ctx.gc_guard.link_phase().await;
    let verified = staged.commit(&declared).await?;

    let forge = TeamForge::for_request(ctx.repo.isle(), access.team_id, stamp(&account, capacity)?);
    let (asset, event) = forge
        .enter_content(pursuit, verified.digest().to_string(), now_ms())
        .await?;
    Ok(Json(ContentEnteredDto {
        asset_id: asset.as_uuid().to_string(),
        digest: verified.digest().to_string(),
        pursuit_id: pursuit.as_uuid().to_string(),
        event: event_dto(&event)?,
    }))
}

/// The most ids or digests one bulk read may be asked about.
///
/// **The bound is the load-bearing part, not the number.** Both bulk
/// verbs walk their input one statement at a time on the isle, which
/// is one connection on one thread — so an unbounded list is a caller
/// deciding how long this server stops answering anybody. That is
/// decision 18's argument about the ledger's read, made about a read
/// the caller sizes rather than one the data does, and it lands the
/// same way. The events page answers it with a ceiling; these two
/// answer it with the same ceiling, so there is one number here rather
/// than a convention per verb.
const CONTENT_BATCH_MAX: usize = EVENTS_PAGE_MAX as usize;

/// Refuses a bulk read that asked for more than [`CONTENT_BATCH_MAX`].
///
/// Refused rather than clamped, which is where this parts company with
/// the events page. A clamped page is still a true answer to a smaller
/// question, and the cursor says where to resume. A truncated bulk read
/// has no cursor and its answer is *wrong*: the resolve would report
/// ids as unknown that it never looked at, and the have-check would
/// tell a client to send bytes the team already holds. So the caller is
/// told to split the list, which it can do without losing anything —
/// neither verb has any state between calls.
fn within_batch(asked: usize, what: &str) -> Result<(), ApiError> {
    if asked > CONTENT_BATCH_MAX {
        return Err(ApiError::Domain(DomainError::Validation(format!(
            "{asked} {what} is more than one request may ask about; the most is \
             {CONTENT_BATCH_MAX}, and this read keeps no state between calls, so \
             the list splits into as many requests as it needs"
        ))));
    }
    Ok(())
}

/// `POST /teams/{team_id}/forge/content/resolve` — which of these
/// asset ids the team holds, and what each was converted from.
///
/// A read, so it asks nothing beyond the gate. An id this team did not
/// mint comes back under `unknown` rather than as a refusal: a caller
/// reconciling a list wants to know which of *its* ids resolve here,
/// and one that does not is an ordinary answer.
async fn resolve_content(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(AuthedAccount(account)): Extension<AuthedAccount>,
    Extension(access): Extension<TeamAccess>,
    Json(command): Json<ResolveContentCommand>,
) -> ForgeResult<ResolvedContentDto> {
    within_batch(command.asset_ids.len(), "asset ids")?;
    let asked = command
        .asset_ids
        .iter()
        .map(|raw| {
            Uuid::parse_str(raw).map(AssetId::from_uuid).map_err(|_| {
                ApiError::Domain(DomainError::Validation(format!(
                    "asset id {raw:?} is not a UUID"
                )))
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let held = read_handle(&ctx, &account, &access)?
        .resolve_assets(asked.clone())
        .await?;
    let unknown = asked
        .iter()
        .filter(|asset| !held.iter().any(|found| &&found.asset == asset))
        .map(|asset| asset.as_uuid().to_string())
        .collect();
    Ok(Json(ResolvedContentDto {
        held: held
            .into_iter()
            .map(|found| HeldAssetDto {
                asset_id: found.asset.as_uuid().to_string(),
                digest: found.digest,
                entered_for_pursuit_id: found
                    .entered_for
                    .map(|pursuit| pursuit.as_uuid().to_string()),
                created_at_ms: found.created_at_ms,
            })
            .collect(),
        unknown,
    }))
}

/// `POST /teams/{team_id}/forge/content/have` — which of these digests
/// the team already holds.
///
/// **Its only purpose is to let a client skip a send**, and the design
/// note that matters is where the answer's boundary sits: inside one
/// team, to that team's members, about digests the asker is already
/// holding. What it can reveal is therefore what the asker could learn
/// by uploading — the same answer, one round trip earlier — which is
/// why it is safe here and would not be a route away. Asked across
/// teams or by anyone outside one, it is the deduplication side
/// channel #83 §3 closes by making the link row the visibility
/// boundary rather than the digest, the attack Harnik et al. (2010)
/// sets out.
///
/// A digest marked for purge answers as **not** held: skipping a send
/// for bytes a reclaim is about to take is exactly the wrong skip
/// (#95).
async fn have_content(
    State(ctx): State<Arc<TeamsCtx>>,
    Extension(access): Extension<TeamAccess>,
    Json(command): Json<HaveContentCommand>,
) -> ForgeResult<HeldContentDto> {
    within_batch(command.digests.len(), "digests")?;
    let held = ctx
        .repo
        .held_digests(access.team_id, command.digests)
        .await?;
    Ok(Json(HeldContentDto {
        held: held.into_iter().collect(),
    }))
}
