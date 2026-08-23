//! SQLite adapter for the forge's ports.
//!
//! One type for `Lines`, `Pursuits`, `Closings` and `Threads`, where
//! the rest of this directory is one per port. The close is why: it
//! writes a change point, its rows and an ending together, and two
//! adapters sharing one transaction is a shape that only reads as
//! sharing when they are the same object.
//!
//! # Where the work is, and where it is not
//!
//! Taking a domain value apart and putting one back lives in
//! [`crate::forge::rows`], which the in-memory store uses too. What is
//! here is SQL and nothing else: the same nine shapes, written as
//! columns.
//!
//! # The head is never read to be compared
//!
//! Nothing here selects a head and checks it against what a caller
//! decided: `UNIQUE (line_id, parent_id)` and `UNIQUE (pursuit_id,
//! parent_id)` refuse a fork as part of the insert. The rule is
//! [`Closings`]' — "on the parent nothing has taken" — and what the
//! index adds is that the check is the write rather than something
//! beside it that could be answered from a row somebody else has
//! since moved.
//!
//! What that costs is telling one constraint violation from another.
//! SQLite names the columns rather than the index — `UNIQUE constraint
//! failed: change_point.line_id, change_point.parent_id` — so that
//! column list is what is matched, and matched exactly.
//!
//! # And the one place a log is read to decide something
//!
//! A close that loses its parent is decided again in here, from a line
//! and a pursuit read inside the transaction that lost. That read is
//! not a comparison: nothing is checked against what the caller
//! decided, and the answer comes from the model rather than from this
//! adapter. What the transaction contributes is that the logs cannot
//! move between the read and the write, which is why the second
//! attempt is the last one.
//!
//! Which column list is which, and what a substring test would read
//! out of the wrong one, is on `is_unique_violation`.

use std::collections::BTreeSet;
use std::sync::Arc;

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
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, Row, params};
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::forge::rows::{
    self, ActRow, ChangePointRow, ChangeRowRow, LineRow, PursuitNodeRow, PursuitOpRow, PursuitRow,
    ThreadMessageRow, ThreadRevisionRow, ThreadRow,
};
use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `Lines`, `Pursuits`, `Closings` and `Threads`.
#[derive(Clone)]
pub struct SqliteForge {
    isle: AsyncIsle,
}

impl SqliteForge {
    /// Wraps a connection.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// Every conversation about anything on one line, as a subquery
/// taking the line's id as `?1`.
///
/// Three of the four anchors reach a line by a different road, and the
/// fourth — an entry as a round had it — arrives by the round's, since
/// it is a node id in the same column. Written once because a drop
/// deletes from three tables through it and three copies of a
/// three-branch predicate is three chances to fix two of them.
const THREADS_OF_A_LINE: &str = "SELECT id FROM forge_thread \
     WHERE anchor_pursuit IN (SELECT id FROM pursuit WHERE line_id = ?1) \
        OR anchor_node IN (SELECT n.id FROM pursuit_node n \
                             JOIN pursuit p ON p.id = n.pursuit_id \
                            WHERE p.line_id = ?1) \
        OR anchor_change_point IN (SELECT id FROM change_point WHERE line_id = ?1)";

/// A line's history does not fork.
const ONE_POINT_PER_PARENT: &str = "change_point.line_id, change_point.parent_id";
/// Neither does a pursuit.
const ONE_NODE_PER_PARENT: &str = "pursuit_node.pursuit_id, pursuit_node.parent_id";
/// And work ends once.
const ONE_ENDING_PER_PURSUIT: &str = "pursuit_node.pursuit_id";

/// Whether this error is that unique constraint, and not another.
///
/// Matched on the exact column list SQLite reports, because one of
/// them is a prefix of another: a violation of `pursuit_node.pursuit_id` is
/// a second ending, and `pursuit_node.pursuit_id, pursuit_node.parent_id` is a
/// fork. `contains` reads the first out of the second — a fork
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

/// Refuses a stored value this model does not have a name for.
///
/// The alternative is a wildcard arm, and the arm has to pick
/// something: `_ => Outcome::Satisfied` reads a row nobody could write
/// as one somebody did, and turns work that gave up into work that
/// landed. A `CHECK` keeps those rows out today, which is exactly why
/// the coercion looked harmless — but the read half is what answers
/// for a database somebody repaired by hand, and answering by guessing
/// is the one thing it must not do.
fn unknown<T>(column: &str, value: &str) -> rusqlite::Result<T> {
    Err(rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("a stored `{column}` says `{value}`, which this model has no name for").into(),
    ))
}

/// Reads an act out of the four columns that carry one, named by
/// prefix so the same three lines serve every table that has a stamp.
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

/// Reads one whole line: its row, its change points, and their rows.
fn read_line(conn: &Connection, id: &LineId) -> rusqlite::Result<Option<Line>> {
    let uuid = *id.as_uuid();
    let head = conn
        .query_row("SELECT * FROM line WHERE id = ?1", params![uuid], line_row)
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    let Some(head) = head else {
        return Ok(None);
    };
    Ok(Some(build_line(conn, &head)?))
}

/// The half of a line read that is the same whether one was asked for
/// or all of them were.
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

/// Reads one whole piece of work under `id`, or nothing if there is
/// none.
fn read_work(conn: &Connection, id: &PursuitId) -> rusqlite::Result<Option<Pursuit>> {
    let head = conn
        .query_row(
            "SELECT * FROM pursuit WHERE id = ?1",
            params![id.as_uuid()],
            pursuit_row,
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
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

/// Writes a change point and its rows.
///
/// Takes a connection rather than a transaction because a savepoint is
/// not one, and an attempt at a close is written inside a savepoint so
/// that the attempt can come back out on its own. Every caller is
/// inside a write either way — a bare connection would put these rows
/// on their own, which is the one thing this port must not do.
fn insert_change_point(
    conn: &Connection,
    point: &ChangePointRow,
    rows: &[ChangeRowRow],
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO change_point \
             (id, line_id, parent_id, from_work, by_node, at, actor_id, actor_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            point.id.as_uuid(),
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
            "INSERT INTO change_row (point_id, entry_id, existence, content, name) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                row.point.as_uuid(),
                row.entry.as_uuid(),
                existence_slug(row.existence),
                row.content.map(|c| *c.as_uuid()),
                row.name.as_ref().map(Name::as_str),
            ],
        )?;
    }
    Ok(())
}

/// Writes a node of a pursuit and the operations under it.
///
/// A connection for the same reason [`insert_change_point`] takes one.
fn insert_work_node(
    conn: &Connection,
    node: &PursuitNodeRow,
    ops: &[PursuitOpRow],
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO pursuit_node \
             (id, pursuit_id, parent_id, kind, outcome, note, at, actor_id, actor_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            node.id.as_uuid(),
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
            "INSERT INTO pursuit_op (node_id, position, entry_id, verb, content, name) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                op.node.as_uuid(),
                op.position as i64,
                op.entry.as_uuid(),
                op.verb,
                op.content.map(|c| *c.as_uuid()),
                op.name.as_ref().map(Name::as_str),
            ],
        )?;
    }
    Ok(())
}

/// Whether this node is one the line has: its genesis, or a change
/// point already on it.
///
/// Not a foreign key, and not for want of trying. A parent is *either*
/// the genesis or a change point, the genesis is a column on `line`
/// rather than a row of its own, and SQLite has no key that points at
/// two tables. Giving the genesis a `change_point` row would buy one —
/// at the cost of a row whose `from_work` and `by_node` are both NULL,
/// which is the pair of empty columns the model refuses to have as a
/// type, and no better as a table.
///
/// So it is a query, asked inside the write's own transaction where
/// the answer cannot go stale. What it costs is one indexed lookup;
/// what it buys is that a close cannot name a node this line never
/// had. Without it the row goes in and the line stops being readable —
/// `restore::chain` walks from the genesis, finds the point
/// unreachable, and refuses the whole history. A store that accepts
/// what it can never hand back is worse than one that refuses.
fn line_has_node(conn: &Connection, line: &LineId, node: ChangePointId) -> rusqlite::Result<bool> {
    let found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM line WHERE id = ?1 AND genesis_id = ?2",
        params![line.as_uuid(), node.as_uuid()],
        |row| row.get(0),
    )?;
    if found > 0 {
        return Ok(true);
    }
    let found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM change_point WHERE line_id = ?1 AND id = ?2",
        params![line.as_uuid(), node.as_uuid()],
        |row| row.get(0),
    )?;
    Ok(found > 0)
}

/// Whether this node is one the pursuit has: the node it opened at, or
/// a node already on it. The line's question, asked of the other log
/// and for the same reason.
fn pursuit_has_node(
    conn: &Connection,
    pursuit: &PursuitId,
    node: NodeId,
) -> rusqlite::Result<bool> {
    let found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pursuit WHERE id = ?1 AND open_node = ?2",
        params![pursuit.as_uuid(), node.as_uuid()],
        |row| row.get(0),
    )?;
    if found > 0 {
        return Ok(true);
    }
    let found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pursuit_node WHERE pursuit_id = ?1 AND id = ?2",
        params![pursuit.as_uuid(), node.as_uuid()],
        |row| row.get(0),
    )?;
    Ok(found > 0)
}

#[async_trait]
impl Lines for SqliteForge {
    async fn open(&self, line: &Line) -> Result<(), DomainError> {
        if !line.history().changes().is_empty() {
            return Err(DomainError::Validation(
                "this port records a line that has just been opened; a history reaches \
                 the store one close at a time"
                    .into(),
            ));
        }
        let head = rows::take_new_line_apart(line);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO line \
                         (id, name, strategy, standing, genesis_id, genesis_at, genesis_by, \
                          genesis_kind, created_at, created_by, created_kind, \
                          updated_at, updated_by, updated_kind) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        head.id.as_uuid(),
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
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn get(&self, id: &LineId) -> Result<Option<Line>, DomainError> {
        let id = *id;
        self.isle
            .call(move |conn| read_line(conn, &id))
            .await
            .map_err(infra_err)
    }

    async fn list(&self) -> Result<Vec<Line>, DomainError> {
        self.isle
            .call(move |conn| {
                let heads: Vec<LineRow> = conn
                    .prepare("SELECT * FROM line")?
                    .query_map([], line_row)?
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
        let (id, name, act) = (*id, name.clone(), ActRow::of(act));
        let moved = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE line SET name = ?2, updated_at = ?3, updated_by = ?4, \
                            updated_kind = ?5 \
                      WHERE id = ?1",
                    params![
                        id.as_uuid(),
                        name.as_str(),
                        datetime_to_ms(&act.at),
                        act.actor.as_uuid(),
                        act.kind,
                    ],
                )
            })
            .await
            .map_err(infra_err)?;
        if moved == 0 {
            return Err(DomainError::not_found("line", id));
        }
        Ok(())
    }

    async fn set_strategy(
        &self,
        id: &LineId,
        strategy: &StrategyId,
        act: &Act,
    ) -> Result<(), DomainError> {
        let (id, strategy, act) = (*id, strategy.clone(), ActRow::of(act));
        let moved = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE line SET strategy = ?2, updated_at = ?3, updated_by = ?4, \
                            updated_kind = ?5 \
                      WHERE id = ?1",
                    params![
                        id.as_uuid(),
                        strategy.as_str(),
                        datetime_to_ms(&act.at),
                        act.actor.as_uuid(),
                        act.kind,
                    ],
                )
            })
            .await
            .map_err(infra_err)?;
        if moved == 0 {
            return Err(DomainError::not_found("line", id));
        }
        Ok(())
    }

    async fn set_standing(
        &self,
        id: &LineId,
        standing: Standing,
        act: &Act,
    ) -> Result<(), DomainError> {
        let (id, act) = (*id, ActRow::of(act));
        let moved = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE line SET standing = ?2, updated_at = ?3, updated_by = ?4, \
                            updated_kind = ?5 \
                      WHERE id = ?1",
                    params![
                        id.as_uuid(),
                        standing_slug(standing),
                        datetime_to_ms(&act.at),
                        act.actor.as_uuid(),
                        act.kind,
                    ],
                )
            })
            .await
            .map_err(infra_err)?;
        if moved == 0 {
            return Err(DomainError::not_found("line", id));
        }
        Ok(())
    }

    async fn discard(&self, id: &LineId, covering: &[PursuitId]) -> Result<(), DomainError> {
        let id = *id;
        let covering: Vec<PursuitId> = covering.to_vec();

        let dropped = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;

                let standing: Option<String> = tx
                    .query_row(
                        "SELECT standing FROM line WHERE id = ?1",
                        params![id.as_uuid()],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(standing) = standing else {
                    return Ok(Err(DropRefusal::NoSuchLine));
                };

                // The first condition the port states. A drop is
                // decided against an archived line, and the standing
                // it was decided against is read here rather than
                // trusted from the caller's copy: a line taken back
                // out of the archive in between is exactly the race
                // `covering` exists to distrust, one field over.
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
                // Two ways the sets differ, and they are not one
                // refusal. Work this drop did not name is work opened
                // since the caller read the list — a race, and the
                // whole reason the ids come in. A name that is not
                // against this line at all cannot have got there by a
                // race: nothing removes a pursuit but a drop of its
                // line, and that line is here. It is the caller
                // naming somebody else's work, which is the model's
                // `NotThisLine` arriving one layer down.
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
                // over these six statements is right for every shape a
                // line can hold: work filed under work is a chain, and
                // one `DELETE` cannot walk it parent-last.
                //
                // Deferring moves the check to COMMIT, by which point
                // the rows that would have violated it are gone too.
                // What survives the deferral is the check that matters:
                // a reference into this line from outside it — another
                // line's work filed under this line's — still fails,
                // and fails the whole drop.
                tx.pragma_update(None, "defer_foreign_keys", 1)?;

                // What was said about any of it goes too. A remark
                // hangs off a pursuit, a round, an entry as a round had
                // it, or a change point, and every one of those is
                // about to stop existing — so a thread left behind
                // would be a remark about nothing, which the read half
                // refuses and two of the four columns are keys
                // against.
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
                tx.execute("DELETE FROM line WHERE id = ?1", params![id.as_uuid()])?;

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
            DropRefusal::Reopened => DomainError::raced(format!(
                "line {id} is out of the archive again, and a drop is decided against an \
                 archived line"
            )),
            DropRefusal::WorkOpenedSince { opened } => DomainError::raced(format!(
                "{opened} pieces of work have been opened on line {id} since this drop was \
                 decided, and what it releases was decided without them"
            )),
            DropRefusal::WorkOfAnotherLine { elsewhere } => DomainError::Validation(format!(
                "this drop of line {id} names {elsewhere} pieces of work that are not against \
                 it, and what another line holds is not this drop's to release"
            )),
            DropRefusal::StillReferenced => DomainError::Validation(format!(
                "something outside line {id} points into it and is staying — work on another \
                 line filed under work on this one, or a row this drop does not know to take"
            )),
        })
    }
}

/// Writes a thread's head row.
fn insert_thread(conn: &Connection, head: &ThreadRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO forge_thread \
             (id, anchor_kind, anchor_pursuit, anchor_node, anchor_entry, \
              anchor_change_point, title, created_at, created_by, created_kind, \
              updated_at, updated_by, updated_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            head.id.as_uuid(),
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
fn insert_message(conn: &Connection, row: &ThreadMessageRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO forge_thread_message \
             (id, thread_id, parent_id, body, said_at, said_by, said_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            row.id.as_uuid(),
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
fn insert_revision(conn: &Connection, row: &ThreadRevisionRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO forge_thread_revision \
             (message_id, position, body, said_at, said_by, said_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            row.message.as_uuid(),
            row.position as i64,
            row.body,
            datetime_to_ms(&row.act.at),
            row.act.actor.as_uuid(),
            row.act.kind,
        ],
    )?;
    Ok(())
}

/// Reads one whole conversation: its row, what was said, and every
/// correction.
fn build_thread(conn: &Connection, head: &ThreadRow) -> rusqlite::Result<Thread> {
    let uuid = *head.id.as_uuid();
    // Ordered here as well as in `read_thread`, which sorts by the
    // stamp and keeps what it is given for two remarks sharing one.
    // Without this that tie is whatever order the scan produced, so
    // one conversation could read two ways. The index is on
    // `(thread_id, said_at)`.
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

#[async_trait]
impl Threads for SqliteForge {
    async fn open(&self, thread: &Thread) -> Result<(), DomainError> {
        let (head, messages, revisions) = rows::take_thread_apart(thread);
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                insert_thread(&tx, &head)?;
                // In the order they were said, so a reply's parent is
                // already a row by the time the reply names it.
                for message in &messages {
                    insert_message(&tx, message)?;
                }
                for revision in &revisions {
                    insert_revision(&tx, revision)?;
                }
                tx.commit()
            })
            .await
            .map_err(infra_err)
    }

    async fn get(&self, id: &ThreadId) -> Result<Option<Thread>, DomainError> {
        let id = *id;
        self.isle
            .call(move |conn| {
                let head = conn
                    .query_row(
                        "SELECT * FROM forge_thread WHERE id = ?1",
                        params![id.as_uuid()],
                        thread_row,
                    )
                    .map(Some)
                    .or_else(|error| match error {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
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
        self.isle
            .call(move |conn| {
                let heads: Vec<ThreadRow> = conn
                    .prepare(
                        "SELECT * FROM forge_thread \
                          WHERE anchor_kind = ?1 \
                            AND anchor_pursuit IS ?2 \
                            AND anchor_node IS ?3 \
                            AND anchor_entry IS ?4 \
                            AND anchor_change_point IS ?5",
                    )?
                    .query_map(
                        params![
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
        let said = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                // The model's refusal, asked of the rows. The foreign
                // key says the parent is a message; it does not say the
                // parent is a message of *this* conversation, and a
                // reply reaching out of its own would make "the thread
                // this belongs to" a question with two answers.
                if let Some(parent) = row.parent {
                    let held: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM forge_thread_message \
                          WHERE id = ?1 AND thread_id = ?2",
                        params![parent.as_uuid(), thread.as_uuid()],
                        |found| found.get(0),
                    )?;
                    if held == 0 {
                        return Ok(Err(parent));
                    }
                }
                insert_message(&tx, &row)?;
                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?;

        said.map_err(|parent| {
            DomainError::clashes(format!("message {parent} is not in thread {thread}"))
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
        let amended = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let held: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM forge_thread_message \
                      WHERE id = ?1 AND thread_id = ?2",
                    params![message.as_uuid(), thread.as_uuid()],
                    |found| found.get(0),
                )?;
                if held == 0 {
                    return Ok(Err(()));
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
                insert_revision(
                    &tx,
                    &rows::take_revision_apart(message, position as usize, &revision),
                )?;
                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?;

        amended.map_err(|()| {
            DomainError::clashes(format!("message {message} is not in thread {thread}"))
        })
    }

    async fn rename(
        &self,
        id: &ThreadId,
        title: Option<&Name>,
        act: &Act,
    ) -> Result<(), DomainError> {
        let (id, title, act) = (*id, title.cloned(), ActRow::of(act));
        let moved = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE forge_thread SET title = ?2, updated_at = ?3, updated_by = ?4, \
                            updated_kind = ?5 \
                      WHERE id = ?1",
                    params![
                        id.as_uuid(),
                        title.as_ref().map(Name::as_str),
                        datetime_to_ms(&act.at),
                        act.actor.as_uuid(),
                        act.kind,
                    ],
                )
            })
            .await
            .map_err(infra_err)?;
        if moved == 0 {
            return Err(DomainError::not_found("thread", id));
        }
        Ok(())
    }
}

/// Why a line was not dropped.
enum DropRefusal {
    /// There is no such line to drop.
    NoSuchLine,
    /// The line is out of the archive, so the standing the drop was
    /// decided against is not the standing it has.
    Reopened,
    /// Work was opened on the line after the caller read the list, so
    /// what they were told the drop releases left it out. A race.
    WorkOpenedSince { opened: usize },
    /// The caller named work that is not against this line, which no
    /// race can produce — nothing removes a pursuit but a drop of its
    /// line, and this line is still here. The model refuses the same
    /// thing as `NotThisLine`, one layer up.
    WorkOfAnotherLine { elsewhere: usize },
    /// A row outside this line names one inside it, and taking the
    /// line would leave that row pointing at nothing.
    StillReferenced,
}

/// Whether this error is a foreign key that was not satisfied.
///
/// Its own function rather than a match at the one site, because a
/// deferred violation arrives from `COMMIT` rather than from the
/// statement that caused it, and reading that as an ordinary failure
/// to commit would report a transport problem for a rule.
fn is_foreign_key_violation(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::ConstraintViolation
                && inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY
    )
}

#[async_trait]
impl Pursuits for SqliteForge {
    async fn open(&self, pursuit: &Pursuit) -> Result<(), DomainError> {
        let (head, nodes, ops) = rows::take_pursuit_apart(pursuit);
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO pursuit \
                         (id, line_id, parent_id, open_node, base_id, title, note, \
                          open_at, open_by, open_kind, created_at, created_by, created_kind, \
                          updated_at, updated_by, updated_kind) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, \
                             ?14, ?15, ?16)",
                    params![
                        head.id.as_uuid(),
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
                    insert_work_node(&tx, node, &its)?;
                }
                tx.commit()
            })
            .await
            .map_err(infra_err)
    }

    async fn get(&self, id: &PursuitId) -> Result<Option<Pursuit>, DomainError> {
        let id = *id;
        self.isle
            .call(move |conn| read_work(conn, &id))
            .await
            .map_err(infra_err)
    }

    async fn of_line(&self, line: &LineId) -> Result<Vec<Pursuit>, DomainError> {
        let line = *line;
        self.isle
            .call(move |conn| {
                let heads: Vec<PursuitRow> = conn
                    .prepare("SELECT * FROM pursuit WHERE line_id = ?1")?
                    .query_map(params![line.as_uuid()], pursuit_row)?
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
        let parent = *parent;
        self.isle
            .call(move |conn| {
                let heads: Vec<PursuitRow> = conn
                    .prepare("SELECT * FROM pursuit WHERE parent_id = ?1")?
                    .query_map(params![parent.as_uuid()], pursuit_row)?
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
        let landed = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let (node, ops) = rows::take_round_apart(id, &round);
                debug_assert_eq!(node.parent, on, "the round names the node it sits on");
                if !pursuit_has_node(&tx, &id, node.parent)? {
                    return Ok(Err(PushRefusal::NotThisPursuit));
                }
                match insert_work_node(&tx, &node, &ops) {
                    Ok(()) => {
                        tx.commit()?;
                        Ok(Ok(()))
                    }
                    Err(error) if is_unique_violation(&error, ONE_NODE_PER_PARENT) => {
                        Ok(Err(PushRefusal::Forked))
                    }
                    Err(error) => Err(error),
                }
            })
            .await
            .map_err(infra_err)?;

        landed.map_err(|refusal| match refusal {
            PushRefusal::Forked => DomainError::raced(format!(
                "work {id} has moved: this round sits on {on}, and something is already there"
            )),
            PushRefusal::NotThisPursuit => DomainError::Validation(format!(
                "this round sits on {on}, which is not a node of work {id}"
            )),
        })
    }
}

#[async_trait]
impl Closings for SqliteForge {
    async fn commit(
        &self,
        line: &LineId,
        pursuit: &PursuitId,
        closing: &Closing,
        again: Arc<dyn Deciding>,
    ) -> Result<(), DomainError> {
        let (line, pursuit) = (*line, *pursuit);
        let closing = closing.clone();

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
                    match land(&attempt, &line, &pursuit, &closing)? {
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

                match first {
                    Ok(()) => {
                        tx.commit()?;
                        return Ok(Ok(()));
                    }
                    // The two ways a log moves under a decision:
                    // somebody landed on the line, or a round arrived
                    // on the work. Both are answered by deciding
                    // again against what is in front of us.
                    Err(Refusal::LineMoved | Refusal::WorkForked) => {}
                    Err(settled) => return Ok(Err(settled)),
                }

                // Read inside the transaction, and this is the whole
                // reason there is one attempt after this rather than
                // five. Transactions on this connection begin
                // IMMEDIATE (see `sqlite::mod`), so the write lock has
                // been held since before the first attempt — including
                // on the path where that attempt wrote nothing at all —
                // and rolling a savepoint back does not end the
                // transaction holding it. Neither log can move between
                // this read and the write below. There is no third
                // answer to lose to.
                let (Some(held), Some(work)) = (read_line(&tx, &line)?, read_work(&tx, &pursuit)?)
                else {
                    return Ok(Err(Refusal::Answered(DomainError::not_found(
                        "the logs this close was written against",
                        line,
                    ))));
                };
                let decided = match again.close(&held, &work) {
                    Ok(decided) => decided,
                    Err(refused) => return Ok(Err(Refusal::Answered(refused))),
                };

                match land(&tx, &line, &pursuit, &decided)? {
                    Ok(()) => {
                        tx.commit()?;
                        Ok(Ok(()))
                    }
                    Err(refusal) => Ok(Err(refusal)),
                }
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
            // against the logs this transaction was holding still. A
            // line that moved anyway is a line something wrote to
            // without taking the write lock.
            Refusal::LineMoved => DomainError::raced(format!(
                "line {line} moved under a close decided against it inside the write"
            )),
            Refusal::WorkForked => DomainError::raced(format!(
                "work {pursuit} moved under a close decided against it inside the write"
            )),
            // The one the message itself already says is not worth
            // asking again: reading it again finds the same ending.
            Refusal::WorkAlreadyEnded => DomainError::settled(format!(
                "work {pursuit} has already ended; reading it again will find the same ending"
            )),
            Refusal::Answered(refused) => refused,
        })
    }
}

/// One attempt at putting a closing on the two logs.
///
/// Writes both halves or refuses, and says which refusal it was. What
/// it does not do is decide anything or undo anything: the caller
/// chooses whether an attempt is a savepoint it can roll back or the
/// last thing this transaction does.
fn land(
    conn: &Connection,
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
    if !pursuit_has_node(conn, pursuit, ending.parent)? {
        return Ok(Err(Refusal::NotThisPursuit));
    }
    if let Some((point, _)) = &landing
        && !line_has_node(conn, line, point.parent)?
    {
        return Ok(Err(Refusal::NotThisLine));
    }

    // The ending first, so a second one is refused by
    // `idx_pursuit_node_one_close` before anything reaches the line —
    // and the whole of the attempt comes back out either way.
    if let Err(error) = insert_work_node(conn, &ending, &[]) {
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
        && let Err(error) = insert_change_point(conn, point, rows)
    {
        if is_unique_violation(&error, ONE_POINT_PER_PARENT) {
            return Ok(Err(Refusal::LineMoved));
        }
        return Err(error);
    }

    Ok(Ok(()))
}

/// Why a round was not written.
///
/// Apart from [`Refusal`] because the two verbs refuse different
/// things, and folding them into one enum would give each a variant
/// the other can never produce.
enum PushRefusal {
    /// Somebody wrote a round on the node this one sits on.
    Forked,
    /// The round sits on a node this pursuit never had.
    NotThisPursuit,
}

/// What refused, and what happens next.
///
/// More variants than there are tables, because two of them come from
/// one table and mean opposite things: one is worth deciding again for
/// and one is not. Collapsing them would discard the distinction at
/// the only place it is ever available.
enum Refusal {
    /// The closing names a node this line never had. Not a race — the
    /// caller decided against something else — so deciding again finds
    /// the same thing.
    NotThisLine,
    /// The ending sits on a node this pursuit never had, for the same
    /// reason and with the same answer.
    NotThisPursuit,
    /// Something already sits on the change point this close aimed at.
    /// Decide again, here, where the line cannot move.
    LineMoved,
    /// A round arrived on the node this close sat on. Same answer:
    /// decide again against the log as it is.
    WorkForked,
    /// The work already has an ending. Deciding again finds the same
    /// one, so there is nothing to re-decide.
    WorkAlreadyEnded,
    /// Deciding again answered with a refusal of its own: the line it
    /// was handed collides with what the work asks for, already says
    /// it, or is not there to be read. That answer is the caller's
    /// answer, and it comes back whole rather than as a race.
    Answered(DomainError),
}
