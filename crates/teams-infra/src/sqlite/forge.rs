//! The team's forge — adapters behind `asterism-core`'s forge ports,
//! over the team's own database (#148 decision 20).
//!
//! One type for `Lines`, `Pursuits`, `Closings`, `Threads`, `Actors`
//! and `Store`. The close is why the first four are one object: it
//! writes a change point, its rows and an ending together, and two
//! adapters sharing one transaction is a shape that only reads as
//! sharing when they are the same object.
//!
//! `Actors` and `Store` join them for a smaller reason: they need the
//! same three things this type holds — the isle, the team, and nothing
//! else — and every question they answer is scoped by the team the way
//! every other read here is. Neither appends to the stream. `Store` is
//! a pure read, and minting a handle deliberately records nothing (see
//! [`TeamForge::handle`]), so what they share with the write ports is
//! the scope rather than the transaction.
//!
//! `Strategies` needs no adapter — `Builtin` is in `asterism-core`, and
//! a collision rule is not storage.
//!
//! # The team is here and in no signature
//!
//! [`TeamForge`] holds a `team_id`, every statement carries it, and no
//! port method mentions it. That is the seat `Lines::list` reserves
//! when it says that scoping a listing belongs to whoever knows what a
//! person is: the forge does not, this plane does, and the answer lands
//! here rather than in a trait the local plane also implements.
//!
//! Reads are scoped as tightly as writes. A `LineId` from another team
//! reads back as nothing rather than as somebody else's line, so a
//! caller holding an id it should not have learns nothing from it.
//!
//! # Every write is one transaction, and the ledger is in it
//!
//! Decision 17: a forge write and its ledger event commit together or
//! neither lands. So every write-port method here is one isle call
//! opening one transaction, and inside it the rows change *and*
//! [`append_event_in_tx`] runs — the same allocation of `seq`, the same
//! registry check and the same subject index rows the repository's own
//! gestures go through. A half-written pair is not a state this store
//! can be in, which is what the e2e asks it for.
//!
//! # Two records, two fields
//!
//! Revision 6. The **event** records the capacity — `LedgerActor`,
//! member or admin, with the display name stamped at write time. The
//! **forge node** records who — an `ActorId`, which is a handle in
//! `forge_actor` and carries no capacity at all. Neither is derived
//! from the other, and an event carries the handle in its payload so
//! the two can be read against each other without a join.
//!
//! Where the handle is one this team's `forge_actor` already holds,
//! the event also carries it as a
//! [`SubjectRef::ForgeIdentity`](teams_core::domain::ledger::SubjectRef::ForgeIdentity),
//! which is what lets a trace query cross from a person to their forge
//! writes without reading payloads. A handle with no row is not an
//! error — the model mints `ActorId`s freely and a caller may carry one
//! this store never saw — so the subject is added when it can be
//! resolved and the payload's `by` answers either way.
//!
//! # What a payload does not carry
//!
//! Anything somebody wrote. Names of lines and titles of threads are
//! here, because a name is how a team refers to a line and an event
//! about a rename that did not say the names would not read on its
//! own. Message bodies are not, and neither is content: the ledger is
//! append-only in the schema and there is no path that rewrites a row,
//! so a body copied into it is a second copy nothing can ever act on
//! when somebody asks to be erased.
//!
//! # The head is never read to be compared
//!
//! Nothing here selects a head and checks it against what a caller
//! decided: `UNIQUE (line_id, parent_id)` and `UNIQUE (pursuit_id,
//! parent_id)` refuse a fork as part of the insert. What that costs is
//! telling one constraint violation from another, so the exact column
//! list SQLite reports is what is matched — see `is_unique_violation`.

use std::collections::BTreeSet;
use std::sync::Arc;

use asterism_core::domain::attribution::{AttributionContext, Author};
use asterism_core::domain::forge::boundary::{Actors, Store};
use asterism_core::domain::forge::closings::{Closings, Deciding};
use asterism_core::domain::forge::lines::Lines;
use asterism_core::domain::forge::model::act::Act;
use asterism_core::domain::forge::model::closing::Closing;
use asterism_core::domain::forge::model::line::{Line, Standing};
use asterism_core::domain::forge::model::pursuit::{Outcome, Pursuit, Round};
use asterism_core::domain::forge::model::thread::{Anchor, Message, Revision, Thread};
use asterism_core::domain::forge::model::value::{
    ActorId, ChangePointId, Content, EntryId, Existence, LineId, MessageId, Name, NodeId,
    PursuitId, StrategyId, ThreadId,
};
use asterism_core::domain::forge::pursuits::Pursuits;
use asterism_core::domain::forge::threads::Threads;
use asterism_core::domain::value::AssetId;
use asterism_core::error::{ConflictKind, DomainError};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use rusqlite_isle::AsyncIsle;
use teams_core::domain::identity::LedgerActor;
use teams_core::domain::ledger::{
    FORGE_CONTENT_ENTERED, FORGE_LINE_DISCARDED, FORGE_LINE_OPENED, FORGE_LINE_RENAMED,
    FORGE_LINE_STANDING_SET, FORGE_LINE_STRATEGY_SET, FORGE_PURSUIT_CLOSED, FORGE_PURSUIT_OPENED,
    FORGE_ROUND_PUSHED, FORGE_THREAD_AMENDED, FORGE_THREAD_OPENED, FORGE_THREAD_RENAMED,
    FORGE_THREAD_SAID, ForgeIdentityRef, LedgerEvent, SubjectRef,
};
use uuid::Uuid;

use crate::forge::rows::{
    self, ActRow, ChangePointRow, ChangeRowRow, LineRow, PursuitNodeRow, PursuitOpRow, PursuitRow,
    ThreadMessageRow, ThreadRevisionRow, ThreadRow,
};
use crate::sqlite::map::{datetime_to_ms, ms_to_datetime};
use crate::sqlite::repo::{append_event_in_tx, link_mark_in_tx};

/// The forge's ports over one team's rows in the teams database.
///
/// Built per request and cheap to build: an isle handle, a uuid and the
/// request's actor stamp. The services above it take five `Arc`s, and
/// the five are clones of one of these.
#[derive(Clone)]
pub struct TeamForge {
    isle: AsyncIsle,
    team_id: Uuid,
    /// Whose capacity every event this handle appends is stamped with.
    /// Per request, because capacity is a property of the request and
    /// not of the store.
    actor: LedgerActor,
}

impl TeamForge {
    /// The forge as one request sees it: one team, and the actor whose
    /// capacity its writes are recorded under.
    pub fn for_request(isle: AsyncIsle, team_id: Uuid, actor: LedgerActor) -> Self {
        Self {
            isle,
            team_id,
            actor,
        }
    }

    /// Which team this handle reads and writes.
    pub const fn team_id(&self) -> Uuid {
        self.team_id
    }
}

/// Wraps an isle failure as the forge's infrastructure error.
fn infra_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> DomainError {
    DomainError::Infra(anyhow::Error::new(e))
}

/// A line's history does not fork.
const ONE_POINT_PER_PARENT: &str = "change_point.line_id, change_point.parent_id";
/// Neither does a pursuit.
const ONE_NODE_PER_PARENT: &str = "pursuit_node.pursuit_id, pursuit_node.parent_id";
/// And work ends once.
const ONE_ENDING_PER_PURSUIT: &str = "pursuit_node.pursuit_id";
/// A team's line names are its own.
const ONE_LINE_PER_NAME: &str = "line.team_id, line.name";

/// Whether this error is that unique constraint, and not another.
///
/// Matched on the exact column list SQLite reports, because one of them
/// is a prefix of another: a violation of `pursuit_node.pursuit_id` is a
/// second ending, and `pursuit_node.pursuit_id, pursuit_node.parent_id`
/// is a fork. `contains` reads the first out of the second — a fork
/// answering to the ending's column list — and the ending is the one
/// refusal here that is final. So the misreading turns "a round
/// arrived, decide again" into "this work is over", and the close that
/// deciding again would have landed is refused instead.
fn is_unique_violation(error: &rusqlite::Error, columns: &str) -> bool {
    let rusqlite::Error::SqliteFailure(inner, Some(message)) = error else {
        return false;
    };
    inner.code == rusqlite::ErrorCode::ConstraintViolation
        && message
            .strip_prefix("UNIQUE constraint failed: ")
            .is_some_and(|named| named == columns)
}

/// Whether this error is a foreign key that was not satisfied.
///
/// Its own function rather than a match at the one site, because a
/// deferred violation arrives from `COMMIT` rather than from the
/// statement that caused it, and reading that as an ordinary failure to
/// commit would report a transport problem for a rule.
fn is_foreign_key_violation(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::ConstraintViolation
                && inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY
    )
}

/// Refuses a stored value this model does not have a name for.
///
/// The alternative is a wildcard arm, and the arm has to pick
/// something: `_ => Outcome::Satisfied` reads a row nobody could write
/// as one somebody did, and turns work that gave up into work that
/// landed.
fn unknown<T>(column: &str, value: &str) -> rusqlite::Result<T> {
    Err(rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("a stored `{column}` says `{value}`, which this model has no name for").into(),
    ))
}

// ----------------------------------------------------------------------
// The ledger half.
// ----------------------------------------------------------------------

/// A refusal from the ledger append, in the vocabulary the forge's
/// callers speak.
///
/// The two planes have a `DomainError` each and they are different
/// types, so something has to cross. Everything that can refuse here is
/// this adapter's own doing — it chooses the kind, builds the payload
/// and never mints a `seq` — so a refusal is a fault in the plumbing
/// rather than anything the caller can restate. `Validation` keeps its
/// text because that text names which part disagreed; the rest arrive
/// as infrastructure, which is what they are.
fn ledger_refusal(kind: &str, refused: teams_core::DomainError) -> DomainError {
    match refused {
        teams_core::DomainError::Validation(said) => DomainError::Infra(anyhow::anyhow!(
            "the ledger refused a {kind} event this store built: {said}"
        )),
        other => DomainError::Infra(anyhow::anyhow!(
            "the ledger could not record a {kind} event: {other}"
        )),
    }
}

/// The handle a forge `ActorId` stands for on this team, when this
/// team's `forge_actor` holds it.
///
/// `None` when it does not, and that is not a failure: the model mints
/// `ActorId`s without asking anybody, so a write can carry one this
/// store never saw. What the subject index gains is the handles it can
/// resolve; what it never does is refuse a write over one it cannot.
fn forge_identity_of(
    conn: &Connection,
    team_id: Uuid,
    actor: ActorId,
) -> rusqlite::Result<Option<ForgeIdentityRef>> {
    let found: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT stands_for, subject FROM forge_actor WHERE id = ?1 AND team_id = ?2",
            params![actor.as_uuid(), team_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((stands_for, subject)) = found else {
        return Ok(None);
    };
    Ok(match (stands_for.as_str(), subject) {
        ("owner", _) => Some(ForgeIdentityRef::owner()),
        ("unrecorded", _) => Some(ForgeIdentityRef::unrecorded()),
        ("server", _) => Some(ForgeIdentityRef::server()),
        ("subject", Some(token)) => ForgeIdentityRef::subject(token).ok(),
        _ => None,
    })
}

/// One event, appended inside the transaction the forge write is in.
///
/// The `by` handle goes into the payload and — when this team knows it
/// — into the subject index as well.
#[allow(clippy::too_many_arguments)] // Six of these are the envelope's own fields (#83 §2); a struct here would name the envelope a second time.
fn append(
    tx: &Transaction<'_>,
    team_id: Uuid,
    actor: &LedgerActor,
    at: DateTime<Utc>,
    by: Option<ActorId>,
    kind: &str,
    mut subjects: Vec<SubjectRef>,
    mut payload: serde_json::Value,
) -> rusqlite::Result<Result<(), DomainError>> {
    if let Some(by) = by {
        if let Some(object) = payload.as_object_mut() {
            object.insert("by".into(), serde_json::json!(by.as_uuid().to_string()));
        }
        if let Some(handle) = forge_identity_of(tx, team_id, by)? {
            subjects.push(SubjectRef::forge_identity(handle));
        }
    }
    match append_event_in_tx(
        tx,
        team_id,
        actor,
        datetime_to_ms(&at),
        kind,
        subjects,
        payload,
    )? {
        Ok(_) => Ok(Ok(())),
        Err(refused) => Ok(Err(ledger_refusal(kind, refused))),
    }
}

// ----------------------------------------------------------------------
// Rows in, rows out.
// ----------------------------------------------------------------------

/// Reads an act out of the four columns that carry one, named by prefix
/// so the same three lines serve every table that has a stamp.
fn act_at(row: &Row<'_>, at: &str, by: &str, kind: &str) -> rusqlite::Result<ActRow> {
    Ok(ActRow {
        at: ms_to_datetime(row.get::<_, i64>(at)?).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                "a stored timestamp is out of range".into(),
            )
        })?,
        actor: ActorId::from_uuid(row.get::<_, Uuid>(by)?),
        kind: match row.get::<_, String>(kind)?.as_str() {
            "user" => "user",
            "system" => "system",
            other => return unknown(kind, other),
        },
    })
}

fn line_row(row: &Row<'_>) -> rusqlite::Result<LineRow> {
    Ok(LineRow {
        id: LineId::from_uuid(row.get("id")?),
        name: Name::new(row.get::<_, String>("name")?).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                "a stored line name is blank".into(),
            )
        })?,
        strategy: StrategyId::new(row.get::<_, String>("strategy")?).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                "a stored strategy id is blank".into(),
            )
        })?,
        standing: match row.get::<_, String>("standing")?.as_str() {
            "open" => Standing::Open,
            "archived" => Standing::Archived,
            other => return unknown("standing", other),
        },
        genesis: ChangePointId::from_uuid(row.get("genesis_id")?),
        genesis_act: act_at(row, "genesis_at", "genesis_by", "genesis_kind")?,
        created: act_at(row, "created_at", "created_by", "created_kind")?,
        updated: act_at(row, "updated_at", "updated_by", "updated_kind")?,
    })
}

fn change_point_row(row: &Row<'_>) -> rusqlite::Result<ChangePointRow> {
    Ok(ChangePointRow {
        id: ChangePointId::from_uuid(row.get("id")?),
        line: LineId::from_uuid(row.get("line_id")?),
        parent: ChangePointId::from_uuid(row.get("parent_id")?),
        from: PursuitId::from_uuid(row.get("from_work")?),
        by: NodeId::from_uuid(row.get("by_node")?),
        act: act_at(row, "at", "actor_id", "actor_kind")?,
    })
}

fn change_row_row(row: &Row<'_>) -> rusqlite::Result<ChangeRowRow> {
    Ok(ChangeRowRow {
        point: ChangePointId::from_uuid(row.get("point_id")?),
        entry: EntryId::from_uuid(row.get("entry_id")?),
        existence: match row.get::<_, Option<String>>("existence")?.as_deref() {
            None => None,
            Some("present") => Some(Existence::Present),
            Some("absent") => Some(Existence::Absent),
            Some(other) => return unknown("existence", other),
        },
        content: row
            .get::<_, Option<Uuid>>("content")?
            .map(Content::from_uuid),
        name: row
            .get::<_, Option<String>>("name")?
            .map(Name::new)
            .transpose()
            .map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    "a stored row name is blank".into(),
                )
            })?,
    })
}

fn pursuit_row(row: &Row<'_>) -> rusqlite::Result<PursuitRow> {
    Ok(PursuitRow {
        id: PursuitId::from_uuid(row.get("id")?),
        of: LineId::from_uuid(row.get("line_id")?),
        parent: row
            .get::<_, Option<Uuid>>("parent_id")?
            .map(PursuitId::from_uuid),
        created: act_at(row, "created_at", "created_by", "created_kind")?,
        updated: act_at(row, "updated_at", "updated_by", "updated_kind")?,
        open: NodeId::from_uuid(row.get("open_node")?),
        base: ChangePointId::from_uuid(row.get("base_id")?),
        title: row
            .get::<_, Option<String>>("title")?
            .map(Name::new)
            .transpose()
            .map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    "a stored work title is blank".into(),
                )
            })?,
        note: row.get("note")?,
        open_act: act_at(row, "open_at", "open_by", "open_kind")?,
    })
}

fn pursuit_node_row(row: &Row<'_>) -> rusqlite::Result<PursuitNodeRow> {
    Ok(PursuitNodeRow {
        pursuit: PursuitId::from_uuid(row.get("pursuit_id")?),
        id: NodeId::from_uuid(row.get("id")?),
        parent: NodeId::from_uuid(row.get("parent_id")?),
        kind: match row.get::<_, String>("kind")?.as_str() {
            "round" => "round",
            "close" => "close",
            other => return unknown("kind", other),
        },
        note: row.get("note")?,
        act: act_at(row, "at", "actor_id", "actor_kind")?,
        outcome: match row.get::<_, Option<String>>("outcome")?.as_deref() {
            None => None,
            Some("satisfied") => Some(Outcome::Satisfied),
            Some("abandoned") => Some(Outcome::Abandoned),
            Some(other) => return unknown("outcome", other),
        },
    })
}

fn pursuit_op_row(row: &Row<'_>) -> rusqlite::Result<PursuitOpRow> {
    Ok(PursuitOpRow {
        node: NodeId::from_uuid(row.get("node_id")?),
        position: row.get::<_, i64>("position")? as usize,
        entry: EntryId::from_uuid(row.get("entry_id")?),
        verb: match row.get::<_, String>("verb")?.as_str() {
            "add" => "add",
            "replace" => "replace",
            "rename" => "rename",
            "remove" => "remove",
            other => return unknown("verb", other),
        },
        content: row
            .get::<_, Option<Uuid>>("content")?
            .map(Content::from_uuid),
        name: row
            .get::<_, Option<String>>("name")?
            .map(Name::new)
            .transpose()
            .map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    "a stored operation name is blank".into(),
                )
            })?,
    })
}

fn thread_row(row: &Row<'_>) -> rusqlite::Result<ThreadRow> {
    Ok(ThreadRow {
        id: ThreadId::from_uuid(row.get("id")?),
        kind: match row.get::<_, String>("anchor_kind")?.as_str() {
            "pursuit" => "pursuit",
            "round" => "round",
            "entry" => "entry",
            "change_point" => "change_point",
            other => return unknown("anchor_kind", other),
        },
        pursuit: row
            .get::<_, Option<Uuid>>("anchor_pursuit")?
            .map(PursuitId::from_uuid),
        node: row
            .get::<_, Option<Uuid>>("anchor_node")?
            .map(NodeId::from_uuid),
        entry: row
            .get::<_, Option<Uuid>>("anchor_entry")?
            .map(EntryId::from_uuid),
        point: row
            .get::<_, Option<Uuid>>("anchor_change_point")?
            .map(ChangePointId::from_uuid),
        title: row
            .get::<_, Option<String>>("title")?
            .map(|said| {
                Name::new(said).map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        "a stored thread title is blank".into(),
                    )
                })
            })
            .transpose()?,
        created: act_at(row, "created_at", "created_by", "created_kind")?,
        updated: act_at(row, "updated_at", "updated_by", "updated_kind")?,
    })
}

fn thread_message_row(row: &Row<'_>) -> rusqlite::Result<ThreadMessageRow> {
    Ok(ThreadMessageRow {
        id: MessageId::from_uuid(row.get("id")?),
        thread: ThreadId::from_uuid(row.get("thread_id")?),
        parent: row
            .get::<_, Option<Uuid>>("parent_id")?
            .map(MessageId::from_uuid),
        body: row.get("body")?,
        act: act_at(row, "said_at", "said_by", "said_kind")?,
    })
}

fn thread_revision_row(row: &Row<'_>) -> rusqlite::Result<ThreadRevisionRow> {
    Ok(ThreadRevisionRow {
        message: MessageId::from_uuid(row.get("message_id")?),
        position: row.get::<_, i64>("position")? as usize,
        body: row.get("body")?,
        act: act_at(row, "said_at", "said_by", "said_kind")?,
    })
}

fn existence_slug(existence: Option<Existence>) -> Option<&'static str> {
    existence.map(|value| match value {
        Existence::Present => "present",
        Existence::Absent => "absent",
    })
}

fn outcome_slug(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Satisfied => "satisfied",
        Outcome::Abandoned => "abandoned",
    }
}

fn standing_slug(standing: Standing) -> &'static str {
    match standing {
        Standing::Open => "open",
        Standing::Archived => "archived",
    }
}

// ----------------------------------------------------------------------
// Reading whole values.
// ----------------------------------------------------------------------

/// Reads one whole line of this team: its row, its change points, and
/// their rows.
fn read_line(conn: &Connection, team_id: Uuid, id: &LineId) -> rusqlite::Result<Option<Line>> {
    let head = conn
        .query_row(
            "SELECT * FROM line WHERE id = ?1 AND team_id = ?2",
            params![id.as_uuid(), team_id],
            line_row,
        )
        .optional()?;
    let Some(head) = head else {
        return Ok(None);
    };
    build_line(conn, &head).map(Some)
}

/// The half of a line read that is the same whether one was asked for
/// or all of them were.
///
/// No team predicate below the head: the head was found within this
/// team, and a change point names the line it is on, so the scope is
/// already spent.
fn build_line(conn: &Connection, head: &LineRow) -> rusqlite::Result<Line> {
    let uuid = *head.id.as_uuid();
    let points: Vec<ChangePointRow> = conn
        .prepare("SELECT * FROM change_point WHERE line_id = ?1")?
        .query_map(params![uuid], change_point_row)?
        .collect::<rusqlite::Result<_>>()?;
    let rows: Vec<ChangeRowRow> = conn
        .prepare(
            "SELECT r.* FROM change_row r \
             JOIN change_point p ON p.id = r.point_id \
             WHERE p.line_id = ?1",
        )?
        .query_map(params![uuid], change_row_row)?
        .collect::<rusqlite::Result<_>>()?;

    rows::read_line(head, &points, &rows).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!("a stored line cannot be read back: {error}").into(),
        )
    })
}

/// Reads one whole piece of this team's work, or nothing if there is
/// none.
fn read_work(
    conn: &Connection,
    team_id: Uuid,
    id: &PursuitId,
) -> rusqlite::Result<Option<Pursuit>> {
    let head = conn
        .query_row(
            "SELECT * FROM pursuit WHERE id = ?1 AND team_id = ?2",
            params![id.as_uuid(), team_id],
            pursuit_row,
        )
        .optional()?;
    let Some(head) = head else {
        return Ok(None);
    };
    build_pursuit(conn, &head).map(Some)
}

/// Reads one whole pursuit: its row, its nodes, and their operations.
fn build_pursuit(conn: &Connection, head: &PursuitRow) -> rusqlite::Result<Pursuit> {
    let uuid = *head.id.as_uuid();
    let nodes: Vec<PursuitNodeRow> = conn
        .prepare("SELECT * FROM pursuit_node WHERE pursuit_id = ?1")?
        .query_map(params![uuid], pursuit_node_row)?
        .collect::<rusqlite::Result<_>>()?;
    let ops: Vec<PursuitOpRow> = conn
        .prepare(
            "SELECT o.* FROM pursuit_op o \
             JOIN pursuit_node n ON n.id = o.node_id \
             WHERE n.pursuit_id = ?1",
        )?
        .query_map(params![uuid], pursuit_op_row)?
        .collect::<rusqlite::Result<_>>()?;

    rows::read_pursuit(head, &nodes, &ops).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!("stored work cannot be read back: {error}").into(),
        )
    })
}

/// Reads one whole conversation: its row, what was said, and every
/// correction.
fn build_thread(conn: &Connection, head: &ThreadRow) -> rusqlite::Result<Thread> {
    let uuid = *head.id.as_uuid();
    // Ordered here as well as in `read_thread`, which sorts by the
    // stamp and keeps what it is given for two remarks sharing one.
    // Without this that tie is whatever order the scan produced, so one
    // conversation could read two ways.
    let messages: Vec<ThreadMessageRow> = conn
        .prepare(
            "SELECT * FROM forge_thread_message WHERE thread_id = ?1 \
             ORDER BY said_at, id",
        )?
        .query_map(params![uuid], thread_message_row)?
        .collect::<rusqlite::Result<_>>()?;
    let revisions: Vec<ThreadRevisionRow> = conn
        .prepare(
            "SELECT r.* FROM forge_thread_revision r \
             JOIN forge_thread_message m ON m.id = r.message_id \
             WHERE m.thread_id = ?1",
        )?
        .query_map(params![uuid], thread_revision_row)?
        .collect::<rusqlite::Result<_>>()?;

    rows::read_thread(head, &messages, &revisions).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!("a stored conversation cannot be read back: {error}").into(),
        )
    })
}

// ----------------------------------------------------------------------
// Writing rows.
// ----------------------------------------------------------------------

/// Writes a change point and its rows.
///
/// Takes a connection rather than a transaction because a savepoint is
/// not one, and an attempt at a close is written inside a savepoint so
/// that the attempt can come back out on its own.
fn insert_change_point(
    conn: &Connection,
    team_id: Uuid,
    point: &ChangePointRow,
    rows: &[ChangeRowRow],
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO change_point \
             (id, team_id, line_id, parent_id, from_work, by_node, at, actor_id, actor_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            point.id.as_uuid(),
            team_id,
            point.line.as_uuid(),
            point.parent.as_uuid(),
            point.from.as_uuid(),
            point.by.as_uuid(),
            datetime_to_ms(&point.act.at),
            point.act.actor.as_uuid(),
            point.act.kind,
        ],
    )?;
    for row in rows {
        conn.execute(
            "INSERT INTO change_row (point_id, entry_id, team_id, existence, content, name) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                row.point.as_uuid(),
                row.entry.as_uuid(),
                team_id,
                existence_slug(row.existence),
                row.content.map(|c| *c.as_uuid()),
                row.name.as_ref().map(Name::as_str),
            ],
        )?;
    }
    Ok(())
}

/// Writes a node of a pursuit and the operations under it.
fn insert_work_node(
    conn: &Connection,
    team_id: Uuid,
    node: &PursuitNodeRow,
    ops: &[PursuitOpRow],
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO pursuit_node \
             (id, team_id, pursuit_id, parent_id, kind, outcome, note, at, actor_id, actor_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            node.id.as_uuid(),
            team_id,
            node.pursuit.as_uuid(),
            node.parent.as_uuid(),
            node.kind,
            node.outcome.map(outcome_slug),
            node.note.as_deref(),
            datetime_to_ms(&node.act.at),
            node.act.actor.as_uuid(),
            node.act.kind,
        ],
    )?;
    for op in ops {
        conn.execute(
            "INSERT INTO pursuit_op (node_id, position, team_id, entry_id, verb, content, name) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                op.node.as_uuid(),
                op.position as i64,
                team_id,
                op.entry.as_uuid(),
                op.verb,
                op.content.map(|c| *c.as_uuid()),
                op.name.as_ref().map(Name::as_str),
            ],
        )?;
    }
    Ok(())
}

/// Writes a thread's head row.
fn insert_thread(conn: &Connection, team_id: Uuid, head: &ThreadRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO forge_thread \
             (id, team_id, anchor_kind, anchor_pursuit, anchor_node, anchor_entry, \
              anchor_change_point, title, created_at, created_by, created_kind, \
              updated_at, updated_by, updated_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            head.id.as_uuid(),
            team_id,
            head.kind,
            head.pursuit.map(|id| *id.as_uuid()),
            head.node.map(|id| *id.as_uuid()),
            head.entry.map(|id| *id.as_uuid()),
            head.point.map(|id| *id.as_uuid()),
            head.title.as_ref().map(Name::as_str),
            datetime_to_ms(&head.created.at),
            head.created.actor.as_uuid(),
            head.created.kind,
            datetime_to_ms(&head.updated.at),
            head.updated.actor.as_uuid(),
            head.updated.kind,
        ],
    )?;
    Ok(())
}

/// Writes one thing said.
fn insert_message(
    conn: &Connection,
    team_id: Uuid,
    row: &ThreadMessageRow,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO forge_thread_message \
             (id, team_id, thread_id, parent_id, body, said_at, said_by, said_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            row.id.as_uuid(),
            team_id,
            row.thread.as_uuid(),
            row.parent.map(|id| *id.as_uuid()),
            row.body,
            datetime_to_ms(&row.act.at),
            row.act.actor.as_uuid(),
            row.act.kind,
        ],
    )?;
    Ok(())
}

/// Writes one correction.
fn insert_revision(
    conn: &Connection,
    team_id: Uuid,
    row: &ThreadRevisionRow,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO forge_thread_revision \
             (message_id, position, team_id, body, said_at, said_by, said_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            row.message.as_uuid(),
            row.position as i64,
            team_id,
            row.body,
            datetime_to_ms(&row.act.at),
            row.act.actor.as_uuid(),
            row.act.kind,
        ],
    )?;
    Ok(())
}

// ----------------------------------------------------------------------
// The two questions a key cannot ask.
// ----------------------------------------------------------------------

/// Whether this node is one the line has: its genesis, or a change
/// point already on it.
///
/// Not a foreign key, and not for want of trying. A parent is *either*
/// the genesis or a change point, the genesis is a column on `line`
/// rather than a row of its own, and SQLite has no key that points at
/// two tables. So it is a query, asked inside the write's own
/// transaction where the answer cannot go stale.
fn line_has_node(
    conn: &Connection,
    team_id: Uuid,
    line: &LineId,
    node: ChangePointId,
) -> rusqlite::Result<bool> {
    let found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM line WHERE id = ?1 AND team_id = ?2 AND genesis_id = ?3",
        params![line.as_uuid(), team_id, node.as_uuid()],
        |row| row.get(0),
    )?;
    if found > 0 {
        return Ok(true);
    }
    let found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM change_point WHERE line_id = ?1 AND team_id = ?2 AND id = ?3",
        params![line.as_uuid(), team_id, node.as_uuid()],
        |row| row.get(0),
    )?;
    Ok(found > 0)
}

/// Whether this node is one the pursuit has: the node it opened at, or
/// a node already on it. The line's question, asked of the other log
/// and for the same reason.
fn pursuit_has_node(
    conn: &Connection,
    team_id: Uuid,
    pursuit: &PursuitId,
    node: NodeId,
) -> rusqlite::Result<bool> {
    let found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pursuit WHERE id = ?1 AND team_id = ?2 AND open_node = ?3",
        params![pursuit.as_uuid(), team_id, node.as_uuid()],
        |row| row.get(0),
    )?;
    if found > 0 {
        return Ok(true);
    }
    let found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pursuit_node WHERE pursuit_id = ?1 AND team_id = ?2 AND id = ?3",
        params![pursuit.as_uuid(), team_id, node.as_uuid()],
        |row| row.get(0),
    )?;
    Ok(found > 0)
}

// ----------------------------------------------------------------------
// Whose rows these are.
// ----------------------------------------------------------------------

/// Whether this team has a row with that id in `table`.
///
/// The question every write asks about the ids a caller handed it, and
/// it is asked inside the write's own transaction for the reason the
/// two chain checks above are: an answer read outside it is an answer
/// about a moment that has passed.
///
/// A foreign key is not this check. `pursuit.line_id` says the line
/// exists; it does not say the line is *this team's*, because the key
/// spans the whole table and the scope is a column on it. So a caller
/// naming another team's line writes a row that the key is perfectly
/// happy with and that no read of either team can make sense of. That
/// is the hole this closes.
///
/// `table` is a literal from the call sites below — SQLite binds
/// values, not table names.
fn team_has(
    conn: &Connection,
    team_id: Uuid,
    table: &'static str,
    id: &Uuid,
) -> rusqlite::Result<bool> {
    conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id = ?1 AND team_id = ?2)"),
        params![id, team_id],
        |row| row.get(0),
    )
}

// ----------------------------------------------------------------------
// Lines.
// ----------------------------------------------------------------------

#[async_trait]
impl Lines for TeamForge {
    async fn open(&self, line: &Line) -> Result<(), DomainError> {
        if !line.history().changes().is_empty() {
            return Err(DomainError::Validation(
                "this port records a line that has just been opened; a history reaches \
                 the store one close at a time"
                    .into(),
            ));
        }
        let head = rows::take_new_line_apart(line);
        let (team_id, actor) = (self.team_id, self.actor.clone());

        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let landed = tx.execute(
                    "INSERT INTO line \
                         (id, team_id, name, strategy, standing, genesis_id, genesis_at, \
                          genesis_by, genesis_kind, created_at, created_by, created_kind, \
                          updated_at, updated_by, updated_kind) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    params![
                        head.id.as_uuid(),
                        team_id,
                        head.name.as_str(),
                        head.strategy.as_str(),
                        standing_slug(head.standing),
                        head.genesis.as_uuid(),
                        datetime_to_ms(&head.genesis_act.at),
                        head.genesis_act.actor.as_uuid(),
                        head.genesis_act.kind,
                        datetime_to_ms(&head.created.at),
                        head.created.actor.as_uuid(),
                        head.created.kind,
                        datetime_to_ms(&head.updated.at),
                        head.updated.actor.as_uuid(),
                        head.updated.kind,
                    ],
                );
                // The one refusal this plane adds to the port: a team's
                // line names are its own, and the index says so.
                if let Err(error) = landed {
                    if is_unique_violation(&error, ONE_LINE_PER_NAME) {
                        return Ok(Err(DomainError::conflict(
                            ConflictKind::Clashes,
                            format!(
                                "this team already has a line called {:?}",
                                head.name.as_str()
                            ),
                        )));
                    }
                    return Err(error);
                }

                if let Err(refused) = append(
                    &tx,
                    team_id,
                    &actor,
                    head.created.at,
                    Some(head.created.actor),
                    FORGE_LINE_OPENED,
                    vec![SubjectRef::forge_line(*head.id.as_uuid())],
                    serde_json::json!({
                        "line": head.id.as_uuid().to_string(),
                        "name": head.name.as_str(),
                        "strategy": head.strategy.as_str(),
                        "standing": standing_slug(head.standing),
                    }),
                )? {
                    return Ok(Err(refused));
                }
                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?
    }

    async fn get(&self, id: &LineId) -> Result<Option<Line>, DomainError> {
        let (id, team_id) = (*id, self.team_id);
        self.isle
            .call(move |conn| read_line(conn, team_id, &id))
            .await
            .map_err(infra_err)
    }

    async fn list(&self) -> Result<Vec<Line>, DomainError> {
        let team_id = self.team_id;
        self.isle
            .call(move |conn| {
                let heads: Vec<LineRow> = conn
                    .prepare("SELECT * FROM line WHERE team_id = ?1")?
                    .query_map(params![team_id], line_row)?
                    .collect::<rusqlite::Result<_>>()?;
                heads
                    .iter()
                    .map(|head| build_line(conn, head))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .map_err(infra_err)
    }

    async fn rename(&self, id: &LineId, name: &Name, act: &Act) -> Result<(), DomainError> {
        self.move_line_description(
            *id,
            ActRow::of(act),
            FORGE_LINE_RENAMED,
            "name",
            name.as_str().to_owned(),
            "UPDATE line SET name = ?3, updated_at = ?4, updated_by = ?5, updated_kind = ?6 \
              WHERE id = ?1 AND team_id = ?2",
            Some(ONE_LINE_PER_NAME),
        )
        .await
    }

    async fn set_strategy(
        &self,
        id: &LineId,
        strategy: &StrategyId,
        act: &Act,
    ) -> Result<(), DomainError> {
        self.move_line_description(
            *id,
            ActRow::of(act),
            FORGE_LINE_STRATEGY_SET,
            "strategy",
            strategy.as_str().to_owned(),
            "UPDATE line SET strategy = ?3, updated_at = ?4, updated_by = ?5, updated_kind = ?6 \
              WHERE id = ?1 AND team_id = ?2",
            None,
        )
        .await
    }

    async fn set_standing(
        &self,
        id: &LineId,
        standing: Standing,
        act: &Act,
    ) -> Result<(), DomainError> {
        self.move_line_description(
            *id,
            ActRow::of(act),
            FORGE_LINE_STANDING_SET,
            "standing",
            standing_slug(standing).to_owned(),
            "UPDATE line SET standing = ?3, updated_at = ?4, updated_by = ?5, updated_kind = ?6 \
              WHERE id = ?1 AND team_id = ?2",
            None,
        )
        .await
    }

    async fn discard(&self, id: &LineId, covering: &[PursuitId]) -> Result<(), DomainError> {
        let id = *id;
        let covering: Vec<PursuitId> = covering.to_vec();
        let (team_id, actor) = (self.team_id, self.actor.clone());

        let dropped = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;

                let held: Option<(String, String)> = tx
                    .query_row(
                        "SELECT standing, name FROM line WHERE id = ?1 AND team_id = ?2",
                        params![id.as_uuid(), team_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                let Some((standing, name)) = held else {
                    return Ok(Err(DropRefusal::NoSuchLine));
                };

                // The first condition the port states. A drop is
                // decided against an archived line, and the standing it
                // was decided against is read here rather than trusted
                // from the caller's copy: a line taken back out of the
                // archive in between is exactly the race `covering`
                // exists to distrust, one field over.
                if standing != standing_slug(Standing::Archived) {
                    return Ok(Err(DropRefusal::Reopened));
                }

                // The second. Asked inside the write, where the answer
                // cannot go stale — a pursuit opened between the
                // caller's read and this one is the whole case, and it
                // is why the ids come in rather than being looked up
                // here.
                let against: Vec<Uuid> = tx
                    .prepare("SELECT id FROM pursuit WHERE line_id = ?1")?
                    .query_map(params![id.as_uuid()], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                let against: BTreeSet<Uuid> = against.into_iter().collect();
                let named: BTreeSet<Uuid> = covering.iter().map(|one| *one.as_uuid()).collect();
                let opened = against.difference(&named).count();
                let elsewhere = named.difference(&against).count();
                if elsewhere > 0 {
                    return Ok(Err(DropRefusal::WorkOfAnotherLine { elsewhere }));
                }
                if opened > 0 {
                    return Ok(Err(DropRefusal::WorkOpenedSince { opened }));
                }

                // Every foreign key inside the forge is RESTRICT, and
                // `pursuit.parent_id` points at `pursuit` — so no order
                // over these statements is right for every shape a line
                // can hold. Deferring moves the check to COMMIT, by
                // which point the rows that would have violated it are
                // gone too. What survives the deferral is the check
                // that matters: a reference into this line from outside
                // it still fails, and fails the whole drop.
                tx.pragma_update(None, "defer_foreign_keys", 1)?;

                // What was said about any of it goes too. A remark
                // hangs off a pursuit, a round, an entry as a round had
                // it, or a change point, and every one of those is
                // about to stop existing.
                tx.execute(
                    &format!(
                        "DELETE FROM forge_thread_revision WHERE message_id IN \
                             (SELECT id FROM forge_thread_message \
                               WHERE thread_id IN ({THREADS_OF_A_LINE}))"
                    ),
                    params![id.as_uuid()],
                )?;
                tx.execute(
                    &format!(
                        "DELETE FROM forge_thread_message \
                          WHERE thread_id IN ({THREADS_OF_A_LINE})"
                    ),
                    params![id.as_uuid()],
                )?;
                tx.execute(
                    &format!("DELETE FROM forge_thread WHERE id IN ({THREADS_OF_A_LINE})"),
                    params![id.as_uuid()],
                )?;

                tx.execute(
                    "DELETE FROM pursuit_op WHERE node_id IN \
                         (SELECT n.id FROM pursuit_node n \
                           JOIN pursuit p ON p.id = n.pursuit_id \
                          WHERE p.line_id = ?1)",
                    params![id.as_uuid()],
                )?;
                tx.execute(
                    "DELETE FROM pursuit_node WHERE pursuit_id IN \
                         (SELECT id FROM pursuit WHERE line_id = ?1)",
                    params![id.as_uuid()],
                )?;
                tx.execute(
                    "DELETE FROM pursuit WHERE line_id = ?1",
                    params![id.as_uuid()],
                )?;
                tx.execute(
                    "DELETE FROM change_row WHERE point_id IN \
                         (SELECT id FROM change_point WHERE line_id = ?1)",
                    params![id.as_uuid()],
                )?;
                tx.execute(
                    "DELETE FROM change_point WHERE line_id = ?1",
                    params![id.as_uuid()],
                )?;
                tx.execute(
                    "DELETE FROM line WHERE id = ?1 AND team_id = ?2",
                    params![id.as_uuid(), team_id],
                )?;

                // The record outlives the rows, which is the whole
                // reason it is a separate table: the line is gone and
                // the stream still says it was here and what it took
                // with it.
                //
                // No handle goes with it. A drop is decided against a
                // line rather than performed by an act — the port takes
                // no `Act` — so the only "who" available is the
                // request's capacity, which the stamp already carries.
                if let Err(refused) = append(
                    &tx,
                    team_id,
                    &actor,
                    Utc::now(),
                    None,
                    FORGE_LINE_DISCARDED,
                    vec![SubjectRef::forge_line(*id.as_uuid())],
                    serde_json::json!({
                        "line": id.as_uuid().to_string(),
                        "name": name,
                        "covering": covering
                            .iter()
                            .map(|one| one.as_uuid().to_string())
                            .collect::<Vec<_>>(),
                    }),
                )? {
                    return Ok(Err(DropRefusal::Answered(refused)));
                }

                match tx.commit() {
                    Ok(()) => Ok(Ok(())),
                    // The deferred check, arriving where every other
                    // error in this file arrives as a statement
                    // failing. Nothing was written.
                    Err(error) if is_foreign_key_violation(&error) => {
                        Ok(Err(DropRefusal::StillReferenced))
                    }
                    Err(error) => Err(error),
                }
            })
            .await
            .map_err(infra_err)?;

        dropped.map_err(|refusal| match refusal {
            DropRefusal::NoSuchLine => DomainError::not_found("line", id),
            DropRefusal::Reopened => DomainError::conflict(
                ConflictKind::Raced,
                format!(
                    "line {id} is out of the archive again, and a drop is decided against an \
                     archived line"
                ),
            ),
            DropRefusal::WorkOpenedSince { opened } => DomainError::conflict(
                ConflictKind::Raced,
                format!(
                    "{opened} pieces of work have been opened on line {id} since this drop was \
                     decided, and what it releases was decided without them"
                ),
            ),
            DropRefusal::WorkOfAnotherLine { elsewhere } => DomainError::Validation(format!(
                "this drop of line {id} names {elsewhere} pieces of work that are not against \
                 it, and what another line holds is not this drop's to release"
            )),
            DropRefusal::StillReferenced => DomainError::Validation(format!(
                "something outside line {id} points into it and is staying — work on another \
                 line filed under work on this one, or a row this drop does not know to take"
            )),
            DropRefusal::Answered(refused) => refused,
        })
    }
}

/// Every conversation about anything on one line, as a subquery taking
/// the line's id as `?1`.
///
/// Three of the four anchors reach a line by a different road, and the
/// fourth — an entry as a round had it — arrives by the round's, since
/// it is a node id in the same column. Written once because a drop
/// deletes from three tables through it.
const THREADS_OF_A_LINE: &str = "SELECT id FROM forge_thread \
     WHERE anchor_pursuit IN (SELECT id FROM pursuit WHERE line_id = ?1) \
        OR anchor_node IN (SELECT n.id FROM pursuit_node n \
                             JOIN pursuit p ON p.id = n.pursuit_id \
                            WHERE p.line_id = ?1) \
        OR anchor_change_point IN (SELECT id FROM change_point WHERE line_id = ?1)";

impl TeamForge {
    /// The three verbs that move a line's description, which differ by
    /// one column and one kind.
    ///
    /// Written once because what they share is the part worth getting
    /// right: the old value is read inside the transaction that
    /// replaces it, so the event carries both halves and reads on its
    /// own — the discipline `teams.membership.role_changed/1` already
    /// holds. Three copies of that would be three chances to read the
    /// old value outside the write, where it is a guess.
    #[allow(clippy::too_many_arguments)] // Six of the seven differ per verb; naming a struct for them would name nothing the verbs share.
    async fn move_line_description(
        &self,
        id: LineId,
        act: ActRow,
        kind: &'static str,
        column: &'static str,
        new: String,
        sql: &'static str,
        clashes: Option<&'static str>,
    ) -> Result<(), DomainError> {
        let (team_id, actor) = (self.team_id, self.actor.clone());

        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;

                // `column` is a literal from the three call sites
                // below and never anything a caller supplies — the
                // only part of these statements that is interpolated
                // rather than bound, because SQLite binds values and
                // not column names.
                let old: Option<String> = tx
                    .query_row(
                        &format!("SELECT {column} FROM line WHERE id = ?1 AND team_id = ?2"),
                        params![id.as_uuid(), team_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(old) = old else {
                    return Ok(Err(DomainError::not_found("line", id)));
                };

                let landed = tx.execute(
                    sql,
                    params![
                        id.as_uuid(),
                        team_id,
                        new,
                        datetime_to_ms(&act.at),
                        act.actor.as_uuid(),
                        act.kind,
                    ],
                );
                if let Err(error) = landed {
                    if clashes.is_some_and(|columns| is_unique_violation(&error, columns)) {
                        return Ok(Err(DomainError::conflict(
                            ConflictKind::Clashes,
                            format!("this team already has a line called {new:?}"),
                        )));
                    }
                    return Err(error);
                }

                if let Err(refused) = append(
                    &tx,
                    team_id,
                    &actor,
                    act.at,
                    Some(act.actor),
                    kind,
                    vec![SubjectRef::forge_line(*id.as_uuid())],
                    serde_json::json!({
                        "line": id.as_uuid().to_string(),
                        "old": old,
                        "new": new,
                    }),
                )? {
                    return Ok(Err(refused));
                }

                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?
    }
}

/// Why a line was not dropped.
enum DropRefusal {
    /// There is no such line in this team to drop.
    NoSuchLine,
    /// The line is out of the archive, so the standing the drop was
    /// decided against is not the standing it has.
    Reopened,
    /// Work was opened on the line after the caller read the list, so
    /// what they were told the drop releases left it out. A race.
    WorkOpenedSince { opened: usize },
    /// The caller named work that is not against this line, which no
    /// race can produce.
    WorkOfAnotherLine { elsewhere: usize },
    /// A row outside this line names one inside it, and taking the line
    /// would leave that row pointing at nothing.
    StillReferenced,
    /// The ledger refused the record of the drop, so the drop does not
    /// happen — that is what one transaction means.
    Answered(DomainError),
}

// ----------------------------------------------------------------------
// Pursuits.
// ----------------------------------------------------------------------

#[async_trait]
impl Pursuits for TeamForge {
    async fn open(&self, pursuit: &Pursuit) -> Result<(), DomainError> {
        let (head, nodes, ops) = rows::take_pursuit_apart(pursuit);
        let (team_id, actor) = (self.team_id, self.actor.clone());

        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;

                // The two ids this value carries in from outside. The
                // foreign keys below say both rows exist; only this
                // says they are this team's, and without it a caller
                // holding another team's line id opens work against it.
                if !team_has(&tx, team_id, "line", head.of.as_uuid())? {
                    return Ok(Err(DomainError::not_found("line", head.of)));
                }
                if let Some(parent) = head.parent
                    && !team_has(&tx, team_id, "pursuit", parent.as_uuid())?
                {
                    return Ok(Err(DomainError::not_found("pursuit", parent)));
                }

                tx.execute(
                    "INSERT INTO pursuit \
                         (id, team_id, line_id, parent_id, open_node, base_id, title, note, \
                          open_at, open_by, open_kind, created_at, created_by, created_kind, \
                          updated_at, updated_by, updated_kind) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
                             ?15, ?16, ?17)",
                    params![
                        head.id.as_uuid(),
                        team_id,
                        head.of.as_uuid(),
                        head.parent.map(|id| *id.as_uuid()),
                        head.open.as_uuid(),
                        head.base.as_uuid(),
                        head.title.as_ref().map(Name::as_str),
                        head.note.as_deref(),
                        datetime_to_ms(&head.open_act.at),
                        head.open_act.actor.as_uuid(),
                        head.open_act.kind,
                        datetime_to_ms(&head.created.at),
                        head.created.actor.as_uuid(),
                        head.created.kind,
                        datetime_to_ms(&head.updated.at),
                        head.updated.actor.as_uuid(),
                        head.updated.kind,
                    ],
                )?;
                // A pursuit that opens with rounds already on it is not
                // a thing `Pursuit::open` makes, but the port takes a
                // whole value and this writes the whole of what it was
                // given rather than the part it expects.
                for node in &nodes {
                    let its: Vec<PursuitOpRow> = ops
                        .iter()
                        .filter(|op| op.node == node.id)
                        .cloned()
                        .collect();
                    insert_work_node(&tx, team_id, node, &its)?;
                }

                if let Err(refused) = append(
                    &tx,
                    team_id,
                    &actor,
                    head.open_act.at,
                    Some(head.open_act.actor),
                    FORGE_PURSUIT_OPENED,
                    vec![
                        SubjectRef::forge_pursuit(*head.id.as_uuid()),
                        SubjectRef::forge_line(*head.of.as_uuid()),
                    ],
                    serde_json::json!({
                        "pursuit": head.id.as_uuid().to_string(),
                        "line": head.of.as_uuid().to_string(),
                        "parent": head.parent.map(|id| id.as_uuid().to_string()),
                        "base": head.base.as_uuid().to_string(),
                        "title": head.title.as_ref().map(Name::as_str),
                    }),
                )? {
                    return Ok(Err(refused));
                }
                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?
    }

    async fn get(&self, id: &PursuitId) -> Result<Option<Pursuit>, DomainError> {
        let (id, team_id) = (*id, self.team_id);
        self.isle
            .call(move |conn| read_work(conn, team_id, &id))
            .await
            .map_err(infra_err)
    }

    async fn of_line(&self, line: &LineId) -> Result<Vec<Pursuit>, DomainError> {
        let (line, team_id) = (*line, self.team_id);
        self.isle
            .call(move |conn| {
                let heads: Vec<PursuitRow> = conn
                    .prepare("SELECT * FROM pursuit WHERE line_id = ?1 AND team_id = ?2")?
                    .query_map(params![line.as_uuid(), team_id], pursuit_row)?
                    .collect::<rusqlite::Result<_>>()?;
                heads
                    .iter()
                    .map(|head| build_pursuit(conn, head))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .map_err(infra_err)
    }

    async fn children(&self, parent: &PursuitId) -> Result<Vec<Pursuit>, DomainError> {
        let (parent, team_id) = (*parent, self.team_id);
        self.isle
            .call(move |conn| {
                let heads: Vec<PursuitRow> = conn
                    .prepare("SELECT * FROM pursuit WHERE parent_id = ?1 AND team_id = ?2")?
                    .query_map(params![parent.as_uuid(), team_id], pursuit_row)?
                    .collect::<rusqlite::Result<_>>()?;
                heads
                    .iter()
                    .map(|head| build_pursuit(conn, head))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .map_err(infra_err)
    }

    async fn push(&self, id: &PursuitId, on: NodeId, round: &Round) -> Result<(), DomainError> {
        let id = *id;
        let round = round.clone();
        let (team_id, actor) = (self.team_id, self.actor.clone());

        let landed = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let (node, ops) = rows::take_round_apart(id, &round);
                debug_assert_eq!(node.parent, on, "the round names the node it sits on");
                if !pursuit_has_node(&tx, team_id, &id, node.parent)? {
                    return Ok(Err(PushRefusal::NotThisPursuit));
                }
                match insert_work_node(&tx, team_id, &node, &ops) {
                    Ok(()) => {}
                    Err(error) if is_unique_violation(&error, ONE_NODE_PER_PARENT) => {
                        return Ok(Err(PushRefusal::Forked));
                    }
                    Err(error) => return Err(error),
                }

                if let Err(refused) = append(
                    &tx,
                    team_id,
                    &actor,
                    node.act.at,
                    Some(node.act.actor),
                    FORGE_ROUND_PUSHED,
                    vec![SubjectRef::forge_pursuit(*id.as_uuid())],
                    serde_json::json!({
                        "pursuit": id.as_uuid().to_string(),
                        "round": node.id.as_uuid().to_string(),
                        "on": node.parent.as_uuid().to_string(),
                        "operations": ops.len(),
                    }),
                )? {
                    return Ok(Err(PushRefusal::Answered(refused)));
                }
                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?;

        landed.map_err(|refusal| match refusal {
            PushRefusal::Forked => DomainError::conflict(
                ConflictKind::Raced,
                format!(
                    "work {id} has moved: this round sits on {on}, and something is already there"
                ),
            ),
            PushRefusal::NotThisPursuit => DomainError::Validation(format!(
                "this round sits on {on}, which is not a node of work {id}"
            )),
            PushRefusal::Answered(refused) => refused,
        })
    }
}

/// Why a round was not written.
///
/// Apart from [`Refusal`] because the two verbs refuse different
/// things, and folding them into one enum would give each a variant the
/// other can never produce.
enum PushRefusal {
    /// Somebody wrote a round on the node this one sits on.
    Forked,
    /// The round sits on a node this pursuit never had.
    NotThisPursuit,
    /// The ledger refused the record, so the round does not land.
    Answered(DomainError),
}

// ----------------------------------------------------------------------
// Closings.
// ----------------------------------------------------------------------

#[async_trait]
impl Closings for TeamForge {
    async fn commit(
        &self,
        line: &LineId,
        pursuit: &PursuitId,
        closing: &Closing,
        again: Arc<dyn Deciding>,
    ) -> Result<(), DomainError> {
        let (line, pursuit) = (*line, *pursuit);
        let closing = closing.clone();
        let (team_id, actor) = (self.team_id, self.actor.clone());

        let landed = self
            .isle
            .call(move |conn| {
                let mut tx = conn.transaction()?;

                // The caller's decision, made outside this transaction
                // where the line could still move under it. Written
                // inside a savepoint because a refusal can arrive with
                // the ending already in — the change point is the half
                // that loses races, and it goes second.
                let first = {
                    let attempt = tx.savepoint()?;
                    match land(&attempt, team_id, &line, &pursuit, &closing)? {
                        Ok(()) => {
                            attempt.commit()?;
                            Ok(())
                        }
                        // Dropped unreleased, which rolls the ending
                        // back to here — and not the transaction, which
                        // is what keeps the write lock.
                        Err(refusal) => Err(refusal),
                    }
                };

                let recorded = match first {
                    Ok(()) => closing.clone(),
                    // The two ways a log moves under a decision:
                    // somebody landed on the line, or a round arrived
                    // on the work. Both are answered by deciding again
                    // against what is in front of us.
                    Err(Refusal::LineMoved | Refusal::WorkForked) => {
                        // Read inside the transaction, and this is the
                        // whole reason there is one attempt after this
                        // rather than five. Transactions on this
                        // connection begin IMMEDIATE, so the write lock
                        // has been held since before the first attempt,
                        // and rolling a savepoint back does not end the
                        // transaction holding it. Neither log can move
                        // between this read and the write below.
                        let (Some(held), Some(work)) = (
                            read_line(&tx, team_id, &line)?,
                            read_work(&tx, team_id, &pursuit)?,
                        ) else {
                            return Ok(Err(Refusal::Answered(DomainError::not_found(
                                "the logs this close was written against",
                                line,
                            ))));
                        };
                        let decided = match again.close(&held, &work) {
                            Ok(decided) => decided,
                            Err(refused) => return Ok(Err(Refusal::Answered(refused))),
                        };
                        match land(&tx, team_id, &line, &pursuit, &decided)? {
                            Ok(()) => decided,
                            Err(refusal) => return Ok(Err(refusal)),
                        }
                    }
                    Err(settled) => return Ok(Err(settled)),
                };

                // One event for one transaction. The ending and the
                // change point were written together, so recording them
                // as two entries would say two things happened.
                let ending = recorded.close();
                let point = recorded.point();
                let act = ActRow::of(ending.act());
                let mut subjects = vec![SubjectRef::forge_pursuit(*pursuit.as_uuid())];
                if point.is_some() {
                    subjects.push(SubjectRef::forge_line(*line.as_uuid()));
                }
                if let Err(refused) = append(
                    &tx,
                    team_id,
                    &actor,
                    act.at,
                    Some(act.actor),
                    FORGE_PURSUIT_CLOSED,
                    subjects,
                    serde_json::json!({
                        "pursuit": pursuit.as_uuid().to_string(),
                        "line": line.as_uuid().to_string(),
                        "outcome": outcome_slug(ending.outcome()),
                        "ending": ending.id().as_uuid().to_string(),
                        "change_point": point.map(|p| p.id().as_uuid().to_string()),
                    }),
                )? {
                    return Ok(Err(Refusal::Answered(refused)));
                }

                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?;

        landed.map_err(|refusal| match refusal {
            Refusal::NotThisLine => DomainError::Validation(format!(
                "this close puts a change point on a node line {line} does not have"
            )),
            Refusal::NotThisPursuit => DomainError::Validation(format!(
                "this ending sits on a node work {pursuit} does not have"
            )),
            // Reachable only from the second attempt, which decided
            // against the logs this transaction was holding still.
            Refusal::LineMoved => DomainError::conflict(
                ConflictKind::Raced,
                format!("line {line} moved under a close decided against it inside the write"),
            ),
            Refusal::WorkForked => DomainError::conflict(
                ConflictKind::Raced,
                format!("work {pursuit} moved under a close decided against it inside the write"),
            ),
            // The one the message itself already says is not worth
            // asking again: reading it again finds the same ending.
            Refusal::WorkAlreadyEnded => DomainError::conflict(
                ConflictKind::Settled,
                format!(
                    "work {pursuit} has already ended; reading it again will find the same ending"
                ),
            ),
            Refusal::Answered(refused) => refused,
        })
    }
}

/// One attempt at putting a closing on the two logs.
///
/// Writes both halves or refuses, and says which refusal it was. What
/// it does not do is decide anything, undo anything, or record
/// anything: the caller chooses whether an attempt is a savepoint it
/// can roll back or the last thing this transaction does, and the
/// ledger entry belongs to the attempt that stuck.
fn land(
    conn: &Connection,
    team_id: Uuid,
    line: &LineId,
    pursuit: &PursuitId,
    closing: &Closing,
) -> rusqlite::Result<Result<(), Refusal>> {
    let ending = rows::take_close_apart(*pursuit, closing.close());
    let landing = closing
        .point()
        .map(|point| rows::take_change_point_apart(*line, point));

    // Both parents before either write. A node naming something this
    // log never had is a row the read half could not turn back into a
    // value, and the unique indexes do not catch it — they refuse a
    // parent used twice, not one that was never there.
    if !pursuit_has_node(conn, team_id, pursuit, ending.parent)? {
        return Ok(Err(Refusal::NotThisPursuit));
    }
    if let Some((point, _)) = &landing
        && !line_has_node(conn, team_id, line, point.parent)?
    {
        return Ok(Err(Refusal::NotThisLine));
    }

    // The ending first, so a second one is refused by
    // `idx_pursuit_node_one_close` before anything reaches the line —
    // and the whole of the attempt comes back out either way.
    if let Err(error) = insert_work_node(conn, team_id, &ending, &[]) {
        // Kept apart, because what happens next differs. A fork means
        // somebody wrote a round while this close was being decided,
        // and deciding again is what answers it. An ending already
        // there means the work is over, and deciding again finds it
        // over — telling somebody to try that again is telling them to
        // do it forever.
        if is_unique_violation(&error, ONE_NODE_PER_PARENT) {
            return Ok(Err(Refusal::WorkForked));
        }
        if is_unique_violation(&error, ONE_ENDING_PER_PURSUIT) {
            return Ok(Err(Refusal::WorkAlreadyEnded));
        }
        return Err(error);
    }

    if let Some((point, rows)) = &landing
        && let Err(error) = insert_change_point(conn, team_id, point, rows)
    {
        if is_unique_violation(&error, ONE_POINT_PER_PARENT) {
            return Ok(Err(Refusal::LineMoved));
        }
        return Err(error);
    }

    Ok(Ok(()))
}

/// What refused, and what happens next.
///
/// More variants than there are tables, because two of them come from
/// one table and mean opposite things: one is worth deciding again for
/// and one is not.
enum Refusal {
    /// The closing names a node this line never had. Not a race — the
    /// caller decided against something else — so deciding again finds
    /// the same thing.
    NotThisLine,
    /// The ending sits on a node this pursuit never had, for the same
    /// reason and with the same answer.
    NotThisPursuit,
    /// Something already sits on the change point this close aimed at.
    LineMoved,
    /// A round arrived on the node this close sat on.
    WorkForked,
    /// The work already has an ending.
    WorkAlreadyEnded,
    /// Deciding again answered with a refusal of its own, or the ledger
    /// refused the record. That answer is the caller's answer, and it
    /// comes back whole rather than as a race.
    Answered(DomainError),
}

// ----------------------------------------------------------------------
// Threads.
// ----------------------------------------------------------------------

#[async_trait]
impl Threads for TeamForge {
    async fn open(&self, thread: &Thread) -> Result<(), DomainError> {
        let (head, messages, revisions) = rows::take_thread_apart(thread);
        let (team_id, actor) = (self.team_id, self.actor.clone());

        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;

                // What the conversation hangs off, checked against
                // this team before a row is written. Two of the four
                // anchor columns are foreign keys and two are bare, so
                // the keys alone answer for half of it and for none of
                // the scope — a thread anchored to another team's work
                // is a remark filed where nobody can read it.
                if let Err(missing) = anchor_is_this_teams(&tx, team_id, &head)? {
                    return Ok(Err(missing));
                }

                insert_thread(&tx, team_id, &head)?;
                // In the order they were said, so a reply's parent is
                // already a row by the time the reply names it.
                for message in &messages {
                    insert_message(&tx, team_id, message)?;
                }
                for revision in &revisions {
                    insert_revision(&tx, team_id, revision)?;
                }

                if let Err(refused) = append(
                    &tx,
                    team_id,
                    &actor,
                    head.created.at,
                    Some(head.created.actor),
                    FORGE_THREAD_OPENED,
                    vec![SubjectRef::forge_thread(*head.id.as_uuid())],
                    serde_json::json!({
                        "thread": head.id.as_uuid().to_string(),
                        "anchor_kind": head.kind,
                        "anchor_pursuit": head.pursuit.map(|id| id.as_uuid().to_string()),
                        "anchor_node": head.node.map(|id| id.as_uuid().to_string()),
                        "anchor_entry": head.entry.map(|id| id.as_uuid().to_string()),
                        "anchor_change_point": head.point.map(|id| id.as_uuid().to_string()),
                        "title": head.title.as_ref().map(Name::as_str),
                        "messages": messages.len(),
                    }),
                )? {
                    return Ok(Err(refused));
                }
                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?
    }

    async fn get(&self, id: &ThreadId) -> Result<Option<Thread>, DomainError> {
        let (id, team_id) = (*id, self.team_id);
        self.isle
            .call(move |conn| {
                let head = conn
                    .query_row(
                        "SELECT * FROM forge_thread WHERE id = ?1 AND team_id = ?2",
                        params![id.as_uuid(), team_id],
                        thread_row,
                    )
                    .optional()?;
                let Some(head) = head else {
                    return Ok(None);
                };
                build_thread(conn, &head).map(Some)
            })
            .await
            .map_err(infra_err)
    }

    async fn anchored(&self, anchor: Anchor) -> Result<Vec<Thread>, DomainError> {
        // The five columns the anchor flattens to, matched as a whole.
        // `IS` rather than `=` because four of them are NULL on any
        // given kind, and `= NULL` is never true.
        let (kind, pursuit, node, entry, point) = rows::anchor_columns(anchor);
        let team_id = self.team_id;
        self.isle
            .call(move |conn| {
                let heads: Vec<ThreadRow> = conn
                    .prepare(
                        "SELECT * FROM forge_thread \
                          WHERE team_id = ?1 \
                            AND anchor_kind = ?2 \
                            AND anchor_pursuit IS ?3 \
                            AND anchor_node IS ?4 \
                            AND anchor_entry IS ?5 \
                            AND anchor_change_point IS ?6",
                    )?
                    .query_map(
                        params![
                            team_id,
                            kind,
                            pursuit.map(|id| *id.as_uuid()),
                            node.map(|id| *id.as_uuid()),
                            entry.map(|id| *id.as_uuid()),
                            point.map(|id| *id.as_uuid()),
                        ],
                        thread_row,
                    )?
                    .collect::<rusqlite::Result<_>>()?;
                heads
                    .iter()
                    .map(|head| build_thread(conn, head))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .map_err(infra_err)
    }

    async fn say(&self, thread: &ThreadId, message: &Message) -> Result<(), DomainError> {
        let thread = *thread;
        let row = rows::take_message_apart(thread, message);
        let (team_id, actor) = (self.team_id, self.actor.clone());

        let said = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                // The model's refusal, asked of the rows. The foreign
                // key says the parent is a message; it does not say the
                // parent is a message of *this* conversation, and a
                // reply reaching out of its own would make "the thread
                // this belongs to" a question with two answers. The
                // team predicate is the same question a scope further
                // out.
                let held: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM forge_thread WHERE id = ?1 AND team_id = ?2",
                    params![thread.as_uuid(), team_id],
                    |found| found.get(0),
                )?;
                if held == 0 {
                    return Ok(Err(SayRefusal::NoSuchThread));
                }
                if let Some(parent) = row.parent {
                    let held: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM forge_thread_message \
                          WHERE id = ?1 AND thread_id = ?2",
                        params![parent.as_uuid(), thread.as_uuid()],
                        |found| found.get(0),
                    )?;
                    if held == 0 {
                        return Ok(Err(SayRefusal::NotThisThread(parent)));
                    }
                }
                insert_message(&tx, team_id, &row)?;

                if let Err(refused) = append(
                    &tx,
                    team_id,
                    &actor,
                    row.act.at,
                    Some(row.act.actor),
                    FORGE_THREAD_SAID,
                    vec![SubjectRef::forge_thread(*thread.as_uuid())],
                    serde_json::json!({
                        "thread": thread.as_uuid().to_string(),
                        "message": row.id.as_uuid().to_string(),
                        "replying_to": row.parent.map(|id| id.as_uuid().to_string()),
                    }),
                )? {
                    return Ok(Err(SayRefusal::Answered(refused)));
                }
                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?;

        said.map_err(|refusal| match refusal {
            // Not a conflict: the caller addressed one thread and named
            // a message of another. Nothing is contended and no row
            // could change to make it hold — the request describes
            // something that is not this thread.
            SayRefusal::NotThisThread(parent) => {
                DomainError::Validation(format!("message {parent} is not in thread {thread}"))
            }
            SayRefusal::NoSuchThread => DomainError::not_found("thread", thread),
            SayRefusal::Answered(refused) => refused,
        })
    }

    async fn amend(
        &self,
        thread: &ThreadId,
        message: &MessageId,
        revision: &Revision,
    ) -> Result<(), DomainError> {
        let (thread, message) = (*thread, *message);
        let revision = revision.clone();
        let (team_id, actor) = (self.team_id, self.actor.clone());

        let amended = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let held: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM forge_thread_message m \
                       JOIN forge_thread t ON t.id = m.thread_id \
                      WHERE m.id = ?1 AND m.thread_id = ?2 AND t.team_id = ?3",
                    params![message.as_uuid(), thread.as_uuid(), team_id],
                    |found| found.get(0),
                )?;
                if held == 0 {
                    return Ok(Err(None));
                }
                // Its place among that message's corrections. Read
                // inside the write, so two corrections arriving at once
                // cannot be given one position — the primary key
                // refuses the second either way, and this is what makes
                // the refusal rare rather than what makes it correct.
                let position: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM forge_thread_revision WHERE message_id = ?1",
                    params![message.as_uuid()],
                    |found| found.get(0),
                )?;
                let row = rows::take_revision_apart(message, position as usize, &revision);
                insert_revision(&tx, team_id, &row)?;

                if let Err(refused) = append(
                    &tx,
                    team_id,
                    &actor,
                    row.act.at,
                    Some(row.act.actor),
                    FORGE_THREAD_AMENDED,
                    vec![SubjectRef::forge_thread(*thread.as_uuid())],
                    serde_json::json!({
                        "thread": thread.as_uuid().to_string(),
                        "message": message.as_uuid().to_string(),
                        "position": position,
                    }),
                )? {
                    return Ok(Err(Some(refused)));
                }
                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?;

        amended.map_err(|refused| match refused {
            None => DomainError::Validation(format!("message {message} is not in thread {thread}")),
            Some(refused) => refused,
        })
    }

    async fn rename(
        &self,
        id: &ThreadId,
        title: Option<&Name>,
        act: &Act,
    ) -> Result<(), DomainError> {
        let (id, title, act) = (*id, title.cloned(), ActRow::of(act));
        let (team_id, actor) = (self.team_id, self.actor.clone());

        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let old: Option<Option<String>> = tx
                    .query_row(
                        "SELECT title FROM forge_thread WHERE id = ?1 AND team_id = ?2",
                        params![id.as_uuid(), team_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(old) = old else {
                    return Ok(Err(DomainError::not_found("thread", id)));
                };

                tx.execute(
                    "UPDATE forge_thread SET title = ?3, updated_at = ?4, updated_by = ?5, \
                            updated_kind = ?6 \
                      WHERE id = ?1 AND team_id = ?2",
                    params![
                        id.as_uuid(),
                        team_id,
                        title.as_ref().map(Name::as_str),
                        datetime_to_ms(&act.at),
                        act.actor.as_uuid(),
                        act.kind,
                    ],
                )?;

                if let Err(refused) = append(
                    &tx,
                    team_id,
                    &actor,
                    act.at,
                    Some(act.actor),
                    FORGE_THREAD_RENAMED,
                    vec![SubjectRef::forge_thread(*id.as_uuid())],
                    serde_json::json!({
                        "thread": id.as_uuid().to_string(),
                        "old": old,
                        "new": title.as_ref().map(Name::as_str),
                    }),
                )? {
                    return Ok(Err(refused));
                }
                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?
    }
}

/// Whether the four anchor columns name rows this team has.
///
/// A thread hangs off one of four things and each is reached
/// differently: a pursuit and a change point are rows with a scope
/// column, a round is a `pursuit_node`, and an entry is an entry of a
/// round — which is not a row at all, so what is checked for it is the
/// round it was had in. That is the same place the schema stops
/// checking, and for the same reason.
fn anchor_is_this_teams(
    conn: &Connection,
    team_id: Uuid,
    head: &ThreadRow,
) -> rusqlite::Result<Result<(), DomainError>> {
    let missing = |entity: &'static str, id: Uuid| Ok(Err(DomainError::not_found(entity, id)));

    if let Some(pursuit) = head.pursuit
        && !team_has(conn, team_id, "pursuit", pursuit.as_uuid())?
    {
        return missing("pursuit", *pursuit.as_uuid());
    }
    if let Some(node) = head.node
        && !team_has(conn, team_id, "pursuit_node", node.as_uuid())?
    {
        return missing("round", *node.as_uuid());
    }
    if let Some(point) = head.point
        && !team_has(conn, team_id, "change_point", point.as_uuid())?
    {
        return missing("change point", *point.as_uuid());
    }
    Ok(Ok(()))
}

/// Why something was not said.
enum SayRefusal {
    /// This team has no such conversation.
    NoSuchThread,
    /// The reply names a message of a different conversation.
    NotThisThread(MessageId),
    /// The ledger refused the record.
    Answered(DomainError),
}

// ----------------------------------------------------------------------
// The boundary: what a handle stands for, and what content is real.
// ----------------------------------------------------------------------

impl TeamForge {
    /// The handle for one thing, minted if this is the first time.
    ///
    /// `INSERT … ON CONFLICT DO NOTHING` and then a read, rather than a
    /// read and then an insert: the two statements are one transaction,
    /// and the conflict clause is what makes the second caller find the
    /// first one's row instead of a violation.
    ///
    /// `display_name` is captured here and nowhere else, which is what
    /// the conflict clause buys beyond concurrency: a later resolve of
    /// the same handle does not reach the row, so the name a handle was
    /// minted under is the name it keeps.
    ///
    /// **No ledger entry.** Minting a handle is not something somebody
    /// did — it happens on the way to a write, the write records who,
    /// and an event for the handle would say a person appeared. The
    /// same-tx rule is about state changes the ledger is the record of,
    /// and this is not one.
    async fn handle(
        &self,
        stands_for: &'static str,
        subject: Option<String>,
        display_name: Option<String>,
    ) -> Result<ActorId, DomainError> {
        let team_id = self.team_id;
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO forge_actor \
                         (id, team_id, stands_for, subject, display_name, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                     ON CONFLICT DO NOTHING",
                    params![
                        Uuid::now_v7(),
                        team_id,
                        stands_for,
                        subject.as_deref(),
                        display_name.as_deref(),
                        Utc::now().timestamp_millis(),
                    ],
                )?;
                let id: Uuid = tx.query_row(
                    "SELECT id FROM forge_actor \
                      WHERE team_id = ?1 AND stands_for = ?2 \
                        AND COALESCE(subject, '') = COALESCE(?3, '')",
                    params![team_id, stands_for, subject.as_deref()],
                    |row| row.get(0),
                )?;
                tx.commit()?;
                Ok(ActorId::from_uuid(id))
            })
            .await
            .map_err(infra_err)
    }
}

#[async_trait]
impl Actors for TeamForge {
    /// The handle for whoever this write is by.
    ///
    /// Keyed on the author and nothing else. The triple also says which
    /// agent carried the write out and through which entry point, and
    /// neither is who did it — the forge keeps a handle on an actor,
    /// and an agent acting for somebody does not make a second
    /// somebody.
    ///
    /// A write that named nobody resolves to one handle rather than a
    /// fresh one each time: "nobody said who" is a single answer.
    ///
    /// **The name is `None`, and the request's stamp is not it.** This
    /// handle holds an authenticated member's display name only if the
    /// author being resolved is that member, and the author arrives as
    /// an opaque subject token that nothing here can compare against a
    /// `user_id`. Stamping the requester's name onto whatever handle
    /// they happen to be resolving would write one person's name onto
    /// another's row and call it a capture. So the column stays the
    /// seat #148 revision 9 describes, and it is filled by the caller
    /// that knows the token and the person are the same.
    async fn resolve(&self, by: &AttributionContext) -> Result<ActorId, DomainError> {
        match by.author() {
            None => self.handle("unrecorded", None, None).await,
            Some(Author::Owner) => self.handle("owner", None, None).await,
            Some(Author::Subject(subject)) => {
                self.handle("subject", Some(subject.clone()), None).await
            }
        }
    }

    /// The handle for this instance acting on this team's forge, which
    /// is what a line's rule writes as.
    ///
    /// One row per team, with no subject. The row is per team rather
    /// than per instance because every other row in this table is, and
    /// a shared one would be the single place where one team's write
    /// touches another team's row.
    async fn server(&self) -> Result<ActorId, DomainError> {
        self.handle("server", None, None).await
    }
}

#[async_trait]
impl Store for TeamForge {
    /// Whether this team has an asset by that id.
    ///
    /// Scoped like every other read here, and the scope is doing real
    /// work: an id another team minted must read as absent, or one
    /// team's line could name another team's content and the foreign
    /// key would be the only thing left saying no.
    async fn exists(&self, asset: &AssetId) -> Result<bool, DomainError> {
        let (id, team_id) = (*asset.as_uuid(), self.team_id);
        self.isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM team_asset WHERE id = ?1 AND team_id = ?2)",
                    params![id, team_id],
                    |row| row.get(0),
                )
            })
            .await
            .map_err(infra_err)
    }
}

// ----------------------------------------------------------------------
// Content — the verb the hosting adds (#148 decision 5).
// ----------------------------------------------------------------------

/// What one entry of content is about, gathered so the in-transaction
/// half takes one argument rather than four more positional ones.
struct EnteringContent<'a> {
    /// The surrogate the team is minting for it.
    asset: Uuid,
    /// The open work it is entering against.
    pursuit: Uuid,
    /// The verified digest the bytes hashed to.
    digest: &'a str,
    /// The instant the row and the event both carry.
    occurred_at_ms: i64,
}

/// The content verb's rows and its event, inside the caller's
/// transaction — [`TeamForge::enter_content`]'s whole write, so that
/// method is the transaction and this is what is in it.
fn enter_content_in_tx(
    tx: &Transaction<'_>,
    team_id: Uuid,
    actor: &LedgerActor,
    entering: EnteringContent<'_>,
) -> rusqlite::Result<Result<LedgerEvent, DomainError>> {
    let EnteringContent {
        asset,
        pursuit,
        digest,
        occurred_at_ms,
    } = entering;
    if !team_has(tx, team_id, "pursuit", &pursuit)? {
        return Ok(Err(DomainError::not_found(
            "pursuit",
            PursuitId::from_uuid(pursuit),
        )));
    }
    let ended: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pursuit_node WHERE pursuit_id = ?1 AND kind = 'close')",
        params![pursuit],
        |row| row.get(0),
    )?;
    if ended {
        return Ok(Err(DomainError::settled(format!(
            "work {pursuit} has ended; content enters a team against open work \
             (#148 decision 5)"
        ))));
    }
    match link_mark_in_tx(tx, team_id, digest)? {
        // Linked and live: decision 7's second contributor, who gets
        // an asset of their own over the copy already there.
        Some(None) => {}
        Some(Some(_)) => {
            return Ok(Err(DomainError::blocked(format!(
                "{digest} is marked for purge in this team; unmark it before naming it \
                 as content"
            ))));
        }
        None => {
            tx.execute(
                "INSERT INTO team_blob_link (team_id, digest, created_at) VALUES (?1, ?2, ?3)",
                params![team_id, digest, occurred_at_ms],
            )?;
        }
    }
    tx.execute(
        "INSERT INTO team_asset (id, team_id, created_at, digest, entered_for)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![asset, team_id, occurred_at_ms, digest, pursuit],
    )?;
    // Both ends of the trace: the digest, which is how the store's
    // side of it is asked about, and the work, which is how decision
    // 5's attachment is. The team asset is in the payload rather than
    // the index — the ledger's subject vocabulary is the team's, and
    // a surrogate this plane mints per promotion is not a reference
    // anything outside a payload asks by.
    let digest_subject = match teams_core::domain::ledger::SubjectRef::digest(digest) {
        Ok(subject) => subject,
        Err(refused) => return Ok(Err(ledger_refusal(FORGE_CONTENT_ENTERED, refused))),
    };
    match append_event_in_tx(
        tx,
        team_id,
        actor,
        occurred_at_ms,
        FORGE_CONTENT_ENTERED,
        vec![digest_subject, SubjectRef::forge_pursuit(pursuit)],
        serde_json::json!({
            "asset_id": asset.to_string(),
            "digest": digest,
            "pursuit_id": pursuit.to_string(),
        }),
    )? {
        Ok(event) => Ok(Ok(event)),
        Err(refused) => Ok(Err(ledger_refusal(FORGE_CONTENT_ENTERED, refused))),
    }
}

/// One `team_asset` as a caller reading it back sees it — what the
/// bulk resolve answers with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldAsset {
    /// The team's own surrogate. Decision 6: each plane mints its own
    /// `AssetId`, and this one is never a local one.
    pub asset: AssetId,
    /// The CAS entry this asset was converted from, when the
    /// conversion was one blob — which is the whole of v0 (`V8`).
    pub digest: Option<String>,
    /// The work the content entered against (decision 5).
    pub entered_for: Option<PursuitId>,
    /// When the team minted it.
    pub created_at_ms: i64,
}

impl TeamForge {
    /// Mints the asset a team holds for content that has just entered
    /// it against open work — the content verb's write half (#148
    /// decision 5).
    ///
    /// The bytes are already durable in the CAS by the time this runs,
    /// on the upload path's ordering (#83 §3): what is left is the
    /// three rows that say the team holds them, and they land in one
    /// transaction with the event. A failure before this leaves an
    /// orphan blob, which the sweep takes; a failure inside it leaves
    /// nothing at all.
    ///
    /// **Both refusals are about the work rather than the bytes.**
    /// Content arrives against *open* work or it does not arrive:
    /// decision 5 keeps the team from holding an asset unattached to
    /// work, and ended work is not something new content can attach
    /// to. A pursuit belonging to another team reads as absent, the
    /// scope every other read here keeps.
    ///
    /// **A digest whose link is marked for purge is refused, not
    /// re-linked.** The mark means a reclaim is coming for those bytes
    /// (#95), and minting an asset over them would hand a line content
    /// that is scheduled to disappear — the "line lying about the
    /// present" decision 2 forbids, arriving by the back door. The
    /// remedy is the caller's and the message says it: unmark first.
    ///
    /// A digest already linked and live is **not** refused, which is
    /// where this parts company with
    /// [`add_blob_link`](crate::sqlite::repo::SqliteTeamsRepository::add_blob_link).
    /// Decision 7 mints an asset per promotion over one stored copy,
    /// so the second contributor of identical bytes gets their own
    /// asset and their own event, and the link row is left as it is.
    pub async fn enter_content(
        &self,
        pursuit: PursuitId,
        digest: String,
        occurred_at_ms: i64,
    ) -> Result<(AssetId, LedgerEvent), DomainError> {
        let asset = AssetId::new();
        let (team_id, actor) = (self.team_id, self.actor.clone());
        let (asset_uuid, pursuit_uuid) = (*asset.as_uuid(), *pursuit.as_uuid());

        let event = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let outcome = enter_content_in_tx(
                    &tx,
                    team_id,
                    &actor,
                    EnteringContent {
                        asset: asset_uuid,
                        pursuit: pursuit_uuid,
                        digest: &digest,
                        occurred_at_ms,
                    },
                )?;
                match outcome {
                    Ok(event) => {
                        tx.commit()?;
                        Ok(Ok(event))
                    }
                    Err(refused) => {
                        tx.rollback()?;
                        Ok(Err(refused))
                    }
                }
            })
            .await
            .map_err(infra_err)??;
        Ok((asset, event))
    }

    /// The assets among `assets` this team holds, with what each was
    /// converted from — the bulk resolve (#148 decision 19).
    ///
    /// Only the ones held come back, and an id this team did not mint
    /// is simply not in the answer: a caller learns which of its own
    /// ids resolve here and nothing about anybody else's, which is the
    /// same scope [`Store::exists`] keeps one id at a time.
    pub async fn resolve_assets(
        &self,
        assets: Vec<AssetId>,
    ) -> Result<Vec<HeldAsset>, DomainError> {
        if assets.is_empty() {
            return Ok(Vec::new());
        }
        let team_id = self.team_id;
        let wanted: BTreeSet<Uuid> = assets.iter().map(|asset| *asset.as_uuid()).collect();
        let rows: Vec<(Uuid, Option<String>, Option<Uuid>, i64)> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, digest, entered_for, created_at FROM team_asset
                      WHERE id = ?1 AND team_id = ?2",
                )?;
                let mut found = Vec::new();
                for id in wanted {
                    let row = stmt
                        .query_row(params![id, team_id], |row| {
                            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                        })
                        .optional()?;
                    if let Some(row) = row {
                        found.push(row);
                    }
                }
                Ok(found)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, digest, entered_for, created_at_ms)| HeldAsset {
                asset: AssetId::from_uuid(id),
                digest,
                entered_for: entered_for.map(PursuitId::from_uuid),
                created_at_ms,
            })
            .collect())
    }
}
