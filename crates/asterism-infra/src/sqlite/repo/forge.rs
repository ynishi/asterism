//! SQLite adapter for the forge's ports.
//!
//! One type for `Lines`, `Pursuits` and `Closings`, where the rest of
//! this directory is one per port. The close is why: it writes a change
//! point, its rows and an ending together, and two adapters sharing one
//! transaction is a shape that only reads as sharing when they are the
//! same object.
//!
//! # Where the work is, and where it is not
//!
//! Taking a domain value apart and putting one back lives in
//! [`crate::forge::rows`], which the in-memory store uses too. What is
//! here is SQL and nothing else: the same six shapes, written as
//! columns.
//!
//! # The head is never read to be compared
//!
//! Nothing here selects a head and checks it against what a caller
//! decided. Two nodes on one parent is a fork, `UNIQUE (line_id,
//! parent_id)` and `UNIQUE (pursuit_id, parent_id)` refuse one, and the
//! refusal arrives as part of the insert — so the validation is the
//! write rather than something beside it that could be answered from a
//! row somebody else has since moved.
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
//! `contains` is what exactness is guarding against, and the direction
//! matters: `pursuit_node.pursuit_id` is the second ending and is a *prefix*
//! of `pursuit_node.pursuit_id, pursuit_node.parent_id`, which is a fork. So a
//! substring test asked about the ending matches the fork — it reads
//! "somebody pushed a pass first" as "this work has already ended",
//! and an ending is final where a fork is decided again. So the close
//! that a second decision would have landed is refused instead, and
//! the caller is told the work is over when it is only one pass
//! further along. The other direction cannot happen, which is why
//! naming it would be naming the wrong risk.

use std::sync::Arc;

use asterism_core::domain::forge::closings::{Closings, Deciding};
use asterism_core::domain::forge::lines::Lines;
use asterism_core::domain::forge::model::act::Act;
use asterism_core::domain::forge::model::closing::Closing;
use asterism_core::domain::forge::model::line::{Line, Standing};
use asterism_core::domain::forge::model::pursuit::{Outcome, Pursuit, Round};
use asterism_core::domain::forge::model::value::{
    ActorId, ChangePointId, Content, EntryId, Existence, LineId, Name, NodeId, PursuitId,
    StrategyId,
};
use asterism_core::domain::forge::pursuits::Pursuits;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::{Connection, Row, params};
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::forge::rows::{
    self, ActRow, ChangePointRow, ChangeRowRow, LineRow, PursuitNodeRow, PursuitOpRow, PursuitRow,
};
use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `Lines`, `Pursuits` and `Closings`.
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
/// refusal here that is final. So the misreading turns "a pass
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
                        match head.standing {
                            Standing::Open => "open",
                            Standing::Archived => "archived",
                        },
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
                // A pursuit that opens with passes already on it is not
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
                debug_assert_eq!(node.parent, on, "the pass names the node it sits on");
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
            PushRefusal::Forked => DomainError::Conflict(format!(
                "work {id} has moved: this pass sits on {on}, and something is already there"
            )),
            PushRefusal::NotThisPursuit => DomainError::Validation(format!(
                "this pass sits on {on}, which is not a node of work {id}"
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
                    // somebody landed on the line, or a pass arrived
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
            Refusal::LineMoved => DomainError::Conflict(format!(
                "line {line} moved under a close decided against it inside the write"
            )),
            Refusal::WorkForked => DomainError::Conflict(format!(
                "work {pursuit} moved under a close decided against it inside the write"
            )),
            Refusal::WorkAlreadyEnded => DomainError::Conflict(format!(
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
        // somebody wrote a pass while this close was being decided,
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

/// Why a pass was not written.
///
/// Apart from [`Refusal`] because the two verbs refuse different
/// things, and folding them into one enum would give each a variant
/// the other can never produce.
enum PushRefusal {
    /// Somebody wrote a pass on the node this one sits on.
    Forked,
    /// The pass sits on a node this pursuit never had.
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
    /// A pass arrived on the node this close sat on. Same answer:
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
