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
//! parent_id)` and `UNIQUE (work_id, parent_id)` refuse one, and the
//! refusal arrives as part of the insert — so the validation is the
//! write rather than something beside it that could be answered from a
//! row somebody else has since moved.
//!
//! What that costs is telling one constraint violation from another.
//! SQLite names the columns rather than the index — `UNIQUE constraint
//! failed: change_point.line_id, change_point.parent_id` — so that
//! column list is what is matched, and matched exactly.
//!
//! `contains` is what exactness is guarding against, and the direction
//! matters: `work_node.work_id` is the second ending and is a *prefix*
//! of `work_node.work_id, work_node.parent_id`, which is a fork. So a
//! substring test asked about the ending matches the fork — it reads
//! "somebody pushed a pass first" as "this work has already ended",
//! and tells a caller that re-reading is pointless when re-reading is
//! the whole answer. The other direction cannot happen, which is why
//! naming it would be naming the wrong risk.

use asterism_core::domain::forge::closings::Closings;
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
use rusqlite::{Connection, Row, Transaction, params};
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::forge::rows::{
    self, ActRow, ChangePointRow, ChangeRowRow, LineRow, WorkNodeRow, WorkOpRow, WorkRow,
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
/// Neither does a work log.
const ONE_NODE_PER_PARENT: &str = "work_node.work_id, work_node.parent_id";
/// And work ends once.
const ONE_ENDING_PER_WORK: &str = "work_node.work_id";

/// Whether this error is that unique constraint, and not another.
///
/// Matched on the exact column list SQLite reports, because one of
/// them is a prefix of another: a violation of `work_node.work_id` is
/// a second ending, and `work_node.work_id, work_node.parent_id` is a
/// fork. `contains` would read the first out of the second and tell a
/// caller to read again and re-decide, which is not a move that helps
/// when the work has already ended.
fn is_unique_violation(error: &rusqlite::Error, columns: &str) -> bool {
    let rusqlite::Error::SqliteFailure(inner, Some(message)) = error else {
        return false;
    };
    inner.code == rusqlite::ErrorCode::ConstraintViolation
        && message
            .strip_prefix("UNIQUE constraint failed: ")
            .is_some_and(|named| named == columns)
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
            "system" => "system",
            _ => "user",
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
            "archived" => Standing::Archived,
            _ => Standing::Open,
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
        existence: row
            .get::<_, Option<String>>("existence")?
            .map(|value| match value.as_str() {
                "absent" => Existence::Absent,
                _ => Existence::Present,
            }),
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

fn work_row(row: &Row<'_>) -> rusqlite::Result<WorkRow> {
    Ok(WorkRow {
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

fn work_node_row(row: &Row<'_>) -> rusqlite::Result<WorkNodeRow> {
    let kind = row.get::<_, String>("kind")?;
    Ok(WorkNodeRow {
        work: PursuitId::from_uuid(row.get("work_id")?),
        id: NodeId::from_uuid(row.get("id")?),
        parent: NodeId::from_uuid(row.get("parent_id")?),
        seq: row.get::<_, i64>("seq")? as usize,
        kind: if kind == "close" { "close" } else { "round" },
        note: row.get("note")?,
        act: act_at(row, "at", "actor_id", "actor_kind")?,
        outcome: row
            .get::<_, Option<String>>("outcome")?
            .map(|value| match value.as_str() {
                "abandoned" => Outcome::Abandoned,
                _ => Outcome::Satisfied,
            }),
    })
}

fn work_op_row(row: &Row<'_>) -> rusqlite::Result<WorkOpRow> {
    let verb = row.get::<_, String>("verb")?;
    Ok(WorkOpRow {
        node: NodeId::from_uuid(row.get("node_id")?),
        position: row.get::<_, i64>("position")? as usize,
        entry: EntryId::from_uuid(row.get("entry_id")?),
        verb: match verb.as_str() {
            "add" => "add",
            "replace" => "replace",
            "rename" => "rename",
            _ => "remove",
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

/// Reads one whole work log: its row, its nodes, and their operations.
fn build_work(conn: &Connection, head: &WorkRow) -> rusqlite::Result<Pursuit> {
    let uuid = *head.id.as_uuid();
    let nodes: Vec<WorkNodeRow> = conn
        .prepare("SELECT * FROM work_node WHERE work_id = ?1")?
        .query_map(params![uuid], work_node_row)?
        .collect::<rusqlite::Result<_>>()?;
    let ops: Vec<WorkOpRow> = conn
        .prepare(
            "SELECT o.* FROM work_op o \
             JOIN work_node n ON n.id = o.node_id \
             WHERE n.work_id = ?1",
        )?
        .query_map(params![uuid], work_op_row)?
        .collect::<rusqlite::Result<_>>()?;

    rows::read_work(head, &nodes, &ops).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!("stored work cannot be read back: {error}").into(),
        )
    })
}

fn insert_change_point(
    tx: &Transaction<'_>,
    point: &ChangePointRow,
    rows: &[ChangeRowRow],
) -> rusqlite::Result<()> {
    tx.execute(
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
        tx.execute(
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

fn insert_work_node(
    tx: &Transaction<'_>,
    node: &WorkNodeRow,
    ops: &[WorkOpRow],
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO work_node \
             (id, work_id, parent_id, seq, kind, outcome, note, at, actor_id, actor_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            node.id.as_uuid(),
            node.work.as_uuid(),
            node.parent.as_uuid(),
            node.seq as i64,
            node.kind,
            node.outcome.map(outcome_slug),
            node.note.as_deref(),
            datetime_to_ms(&node.act.at),
            node.act.actor.as_uuid(),
            node.act.kind,
        ],
    )?;
    for op in ops {
        tx.execute(
            "INSERT INTO work_op (node_id, position, entry_id, verb, content, name) \
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

/// How many nodes a work log already has, which is the next one's
/// place in it.
fn next_seq(tx: &Transaction<'_>, work: &PursuitId) -> rusqlite::Result<usize> {
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM work_node WHERE work_id = ?1",
        params![work.as_uuid()],
        |row| row.get(0),
    )?;
    Ok(count as usize)
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
        let (head, nodes, ops) = rows::take_work_apart(pursuit);
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO work \
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
                    let its: Vec<WorkOpRow> = ops
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
            .call(move |conn| {
                let head = conn
                    .query_row(
                        "SELECT * FROM work WHERE id = ?1",
                        params![id.as_uuid()],
                        work_row,
                    )
                    .map(Some)
                    .or_else(|error| match error {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                let Some(head) = head else {
                    return Ok(None);
                };
                build_work(conn, &head).map(Some)
            })
            .await
            .map_err(infra_err)
    }

    async fn of_line(&self, line: &LineId) -> Result<Vec<Pursuit>, DomainError> {
        let line = *line;
        self.isle
            .call(move |conn| {
                let heads: Vec<WorkRow> = conn
                    .prepare("SELECT * FROM work WHERE line_id = ?1")?
                    .query_map(params![line.as_uuid()], work_row)?
                    .collect::<rusqlite::Result<_>>()?;
                heads
                    .iter()
                    .map(|head| build_work(conn, head))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .map_err(infra_err)
    }

    async fn children(&self, parent: &PursuitId) -> Result<Vec<Pursuit>, DomainError> {
        let parent = *parent;
        self.isle
            .call(move |conn| {
                let heads: Vec<WorkRow> = conn
                    .prepare("SELECT * FROM work WHERE parent_id = ?1")?
                    .query_map(params![parent.as_uuid()], work_row)?
                    .collect::<rusqlite::Result<_>>()?;
                heads
                    .iter()
                    .map(|head| build_work(conn, head))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .map_err(infra_err)
    }

    async fn push(&self, id: &PursuitId, on: NodeId, round: &Round) -> Result<(), DomainError> {
        let id = *id;
        // Assembled outside, because `seq` is the only thing about it
        // the store decides and the rest is the caller's value taken
        // apart.
        let round = round.clone();
        let landed = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let seq = next_seq(&tx, &id)?;
                let (node, ops) = rows::take_round_apart(id, &round, seq);
                debug_assert_eq!(node.parent, on, "the pass names the node it sits on");
                match insert_work_node(&tx, &node, &ops) {
                    Ok(()) => {
                        tx.commit()?;
                        Ok(Ok(()))
                    }
                    Err(error) if is_unique_violation(&error, ONE_NODE_PER_PARENT) => Ok(Err(())),
                    Err(error) => Err(error),
                }
            })
            .await
            .map_err(infra_err)?;

        landed.map_err(|()| {
            DomainError::Conflict(format!(
                "work {id} has moved: this pass sits on {on}, and something is already there"
            ))
        })
    }
}

#[async_trait]
impl Closings for SqliteForge {
    async fn commit(
        &self,
        line: &LineId,
        pursuit: &PursuitId,
        on: ChangePointId,
        closing: &Closing,
    ) -> Result<(), DomainError> {
        let (line, pursuit) = (*line, *pursuit);
        let landing = closing
            .point()
            .map(|point| rows::take_change_point_apart(line, point));
        let close = closing.close().clone();

        let landed = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let seq = next_seq(&tx, &pursuit)?;
                let ending = rows::take_close_apart(pursuit, &close, seq);

                // The ending first, so a second one is refused by
                // `idx_work_node_one_close` before anything reaches the
                // line — and the whole of it rolls back either way.
                if let Err(error) = insert_work_node(&tx, &ending, &[]) {
                    // Kept apart, because the caller's next move
                    // differs. A fork means somebody wrote a pass
                    // while this close was being decided, and reading
                    // again is what answers it. An ending already
                    // there means the work is over, and reading again
                    // finds it over — telling somebody to retry that
                    // is telling them to do it forever.
                    if is_unique_violation(&error, ONE_NODE_PER_PARENT) {
                        return Ok(Err(Refusal::WorkForked));
                    }
                    if is_unique_violation(&error, ONE_ENDING_PER_WORK) {
                        return Ok(Err(Refusal::WorkAlreadyEnded));
                    }
                    return Err(error);
                }

                if let Some((point, rows)) = &landing
                    && let Err(error) = insert_change_point(&tx, point, rows)
                {
                    if is_unique_violation(&error, ONE_POINT_PER_PARENT) {
                        return Ok(Err(Refusal::LineMoved));
                    }
                    return Err(error);
                }

                tx.commit()?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?;

        landed.map_err(|refusal| match refusal {
            Refusal::LineMoved => DomainError::Conflict(format!(
                "line {line} has moved: this close lands on {on}, and something is already there"
            )),
            Refusal::WorkForked => DomainError::Conflict(format!(
                "work {pursuit} has moved: a pass arrived where this ending would go"
            )),
            Refusal::WorkAlreadyEnded => DomainError::Conflict(format!(
                "work {pursuit} has already ended; reading it again will find the same ending"
            )),
        })
    }
}

/// What refused, and what the caller can do about it.
///
/// Three rather than two, because two of them come from one table and
/// mean opposite things: one is worth reading again for and one is
/// not. Collapsing them would discard the distinction at the only
/// place it is ever available.
enum Refusal {
    /// Something already sits on the change point this close aimed at.
    /// Read the line again and decide again.
    LineMoved,
    /// A pass arrived on the node this close sat on. Same answer:
    /// read again.
    WorkForked,
    /// The work already has an ending. Reading again finds the same
    /// one, so there is nothing to re-decide.
    WorkAlreadyEnded,
}
