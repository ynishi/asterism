//! The shapes a store keeps, and the two translations either side of
//! them.
//!
//! Every field here is a scalar or an id. Nothing holds a [`Line`], a
//! [`Pursuit`], a [`ChangePoint`] or an [`Op`] — that is the whole
//! point of the module, and the reason it is written out rather than
//! replaced by keeping the domain values in a map.
//!
//! The names match what the SQLite tables will be called, because the
//! adapter that writes those is meant to be able to read this one as a
//! specification of what it owes.

use asterism_core::domain::forge::model::act::{Act, Actor};
use asterism_core::domain::forge::model::history::ChangePoint;
use asterism_core::domain::forge::model::line::{Line, Standing};
use asterism_core::domain::forge::model::op::{Op, OpKind};
use asterism_core::domain::forge::model::pursuit::{Close, Outcome, Pursuit, Round};
use asterism_core::domain::forge::model::restore;
use asterism_core::domain::forge::model::table::Row;
use asterism_core::domain::forge::model::value::{
    ActorId, ChangePointId, Content, EntryId, Existence, LineId, Name, NodeId, PursuitId,
    StrategyId,
};
use asterism_core::error::DomainError;
use chrono::{DateTime, Utc};

/// An act, flattened the way a row carries one: a stamp, a handle, and
/// which kind of actor the handle names.
#[derive(Debug, Clone, Copy)]
pub struct ActRow {
    /// When.
    pub at: DateTime<Utc>,
    /// The forge's handle on whoever it was.
    pub actor: ActorId,
    /// `user` or `system`. A rule is not a person and the forge says
    /// so; a column that flattened the two would lose the one thing
    /// the model keeps about an actor for itself.
    pub kind: &'static str,
}

impl ActRow {
    /// Flattens an act into the columns a row carries.
    pub fn of(act: &Act) -> Self {
        let (actor, kind) = match act.by() {
            Actor::User(id) => (id, "user"),
            Actor::System(id) => (id, "system"),
        };
        Self {
            at: act.at(),
            actor,
            kind,
        }
    }

    /// Reads the columns back, refusing an actor kind the model does
    /// not have.
    pub fn read(self) -> Result<Act, DomainError> {
        let by = match self.kind {
            "user" => Actor::User(self.actor),
            "system" => Actor::System(self.actor),
            other => {
                return Err(DomainError::Validation(format!(
                    "a stored act names an actor kind this model does not have: {other}"
                )));
            }
        };
        Ok(Act::new(self.at, by))
    }
}

/// `line` — the repository, and the two things about it that are not
/// in its history.
#[derive(Debug, Clone)]
pub struct LineRow {
    /// Which line.
    pub id: LineId,
    /// What it is called. Moves without touching the history.
    pub name: Name,
    /// The rule it settles collisions by. Moves the same way.
    pub strategy: StrategyId,
    /// Whether it is still being worked on. Beside the name and the
    /// strategy because it is a statement about the line, not about
    /// anything the line carries.
    pub standing: Standing,
    /// The node it begins at. Inline rather than a row of its own: a
    /// history has exactly one and can never be without it.
    pub genesis: ChangePointId,
    /// When the line began, and who began it.
    pub genesis_act: ActRow,
    /// When the description was made.
    pub created: ActRow,
    /// The last time the description moved.
    pub updated: ActRow,
}

/// `change_point` — one move of a line, without the table it carries.
#[derive(Debug, Clone)]
pub struct ChangePointRow {
    /// Which node.
    pub id: ChangePointId,
    /// Which line it is on.
    pub line: LineId,
    /// The node it was recorded on top of. This is the order — there
    /// is no sequence column, and a reader walks the links.
    pub parent: ChangePointId,
    /// The work it came out of.
    pub from: PursuitId,
    /// The ending that produced it. Not derivable from `from`, and the
    /// pair is what makes "these two were one act" readable.
    pub by: NodeId,
    /// When it landed, and who landed it.
    pub act: ActRow,
}

/// `change_row` — one axis-triple of one entry, under one change
/// point. The primary key is `(point, entry)`.
#[derive(Debug, Clone)]
pub struct ChangeRowRow {
    /// The change point this row belongs to.
    pub point: ChangePointId,
    /// The entry it is about.
    pub entry: EntryId,
    /// Whether the change puts the entry on the line or takes it off.
    /// `None` means this change said nothing about that axis.
    pub existence: Option<Existence>,
    /// What it moves the entry's content to, if it moves it.
    pub content: Option<Content>,
    /// What it moves the entry's name to, if it moves it.
    pub name: Option<Name>,
}

/// `pursuit` — one line of work.
#[derive(Debug, Clone)]
pub struct PursuitRow {
    /// Which piece of work.
    pub id: PursuitId,
    /// The line it is against, declared when it opened.
    pub of: LineId,
    /// The work it is filed under, if any. Set once, never rewritten.
    pub parent: Option<PursuitId>,
    /// When the description was made.
    pub created: ActRow,
    /// The last time the description moved.
    pub updated: ActRow,
    /// The node the log opens at. Inline rather than a row of its own:
    /// a pursuit has exactly one and can never be without it, so the
    /// join would always match.
    pub open: NodeId,
    /// The change point the work was cut from.
    pub base: ChangePointId,
    /// A short name for the work, if it was given one.
    pub title: Option<Name>,
    /// Anything else said about why.
    pub note: Option<String>,
    /// When it opened, and who opened it.
    pub open_act: ActRow,
}

/// `pursuit_node` — a pass or an ending. `seq` is the log's own order,
/// kept because a pursuit is read forwards and a parent chain would
/// have to be walked to say the same thing.
#[derive(Debug, Clone)]
pub struct PursuitNodeRow {
    /// The pursuit it belongs to.
    pub pursuit: PursuitId,
    /// Which node.
    pub id: NodeId,
    /// The node before it.
    pub parent: NodeId,
    /// Its place in the log, counting from the first pass.
    pub seq: usize,
    /// `round` or `close`.
    pub kind: &'static str,
    /// One short free-text slot.
    pub note: Option<String>,
    /// When it was written, and by whom.
    pub act: ActRow,
    /// Set on a close and nowhere else.
    pub outcome: Option<Outcome>,
}

/// `pursuit_op` — one operation of one pass, in the order it was written.
#[derive(Debug, Clone)]
pub struct PursuitOpRow {
    /// The pass that wrote it.
    pub node: NodeId,
    /// Its place in that pass, in the order it was written.
    pub position: usize,
    /// The entry it is about.
    pub entry: EntryId,
    /// `add` / `replace` / `rename` / `remove`.
    pub verb: &'static str,
    /// The content the verb carries, on `add` and `replace`.
    pub content: Option<Content>,
    /// The name the verb carries, on `add` and `rename`.
    pub name: Option<Name>,
}

// ---------------------------------------------------------------
// Taking a domain value apart.
// ---------------------------------------------------------------

/// The row for a line that has just been opened.
///
/// One row and no others, because a line that has just been opened has
/// a genesis and nothing else — the port that calls this says so, and
/// [`Line::open`](asterism_core::domain::forge::model::line::Line::open)
/// is the only thing that produces what it takes.
///
/// A loop over `history().changes()` here would be a second way for a
/// change point to reach the store, running only for a caller that
/// does not exist. The port refuses that caller instead, which is a
/// statement somebody can read rather than a branch nothing covers.
pub fn take_new_line_apart(line: &Line) -> LineRow {
    LineRow {
        id: line.id(),
        name: line.name().clone(),
        strategy: line.strategy().clone(),
        standing: line.standing(),
        genesis: line.history().genesis().id(),
        genesis_act: ActRow::of(line.history().genesis().act()),
        created: ActRow::of(line.meta().created()),
        updated: ActRow::of(line.meta().updated()),
    }
}

/// One change point's rows, for the single-node write a close makes.
pub fn take_change_point_apart(
    line: LineId,
    point: &ChangePoint,
) -> (ChangePointRow, Vec<ChangeRowRow>) {
    (
        ChangePointRow {
            id: point.id(),
            line,
            parent: point.parent(),
            from: point.from(),
            by: point.by(),
            act: ActRow::of(point.act()),
        },
        point
            .table()
            .rows()
            .iter()
            .map(|(entry, row)| ChangeRowRow {
                point: point.id(),
                entry: *entry,
                existence: row.existence(),
                content: row.content(),
                name: row.name().cloned(),
            })
            .collect(),
    )
}

/// The work's own row, and one `pursuit_node` row plus its `pursuit_op`s for
/// every node after the one it opened at.
pub fn take_pursuit_apart(
    pursuit: &Pursuit,
) -> (PursuitRow, Vec<PursuitNodeRow>, Vec<PursuitOpRow>) {
    let open = pursuit.opening();
    let head = PursuitRow {
        id: pursuit.id(),
        of: pursuit.of(),
        parent: pursuit.parent(),
        created: ActRow::of(pursuit.meta().created()),
        updated: ActRow::of(pursuit.meta().updated()),
        open: open.id(),
        base: open.base(),
        title: open.intent().title.clone(),
        note: open.intent().note.clone(),
        open_act: ActRow::of(open.act()),
    };

    let mut nodes = Vec::new();
    let mut ops = Vec::new();
    for (seq, round) in pursuit.rounds().iter().enumerate() {
        let (node, wrote) = take_round_apart(pursuit.id(), round, seq);
        nodes.push(node);
        ops.extend(wrote);
    }
    if let Some(close) = pursuit.close() {
        nodes.push(take_close_apart(pursuit.id(), close, nodes.len()));
    }
    (head, nodes, ops)
}

/// One pass, for the append a push makes.
pub fn take_round_apart(
    pursuit: PursuitId,
    round: &Round,
    seq: usize,
) -> (PursuitNodeRow, Vec<PursuitOpRow>) {
    let node = PursuitNodeRow {
        pursuit,
        id: round.id(),
        parent: round.parent(),
        seq,
        kind: "round",
        note: round.note().map(str::to_owned),
        act: ActRow::of(round.act()),
        outcome: None,
    };
    let ops = round
        .ops()
        .iter()
        .enumerate()
        .map(|(position, op)| {
            let (verb, content, name) = match op.kind() {
                OpKind::Add { content, name } => ("add", Some(*content), Some(name.clone())),
                OpKind::Replace { content } => ("replace", Some(*content), None),
                OpKind::Rename { name } => ("rename", None, Some(name.clone())),
                OpKind::Remove => ("remove", None, None),
            };
            PursuitOpRow {
                node: round.id(),
                position,
                entry: op.entry(),
                verb,
                content,
                name,
            }
        })
        .collect();
    (node, ops)
}

/// One ending, for the append a commit makes.
pub fn take_close_apart(pursuit: PursuitId, close: &Close, seq: usize) -> PursuitNodeRow {
    PursuitNodeRow {
        pursuit,
        id: close.id(),
        parent: close.parent(),
        seq,
        kind: "close",
        note: close.note().map(str::to_owned),
        act: ActRow::of(close.act()),
        outcome: Some(close.outcome()),
    }
}

// ---------------------------------------------------------------
// Putting one back together.
// ---------------------------------------------------------------

/// Rebuilds a line from its row, its change points and their rows.
///
/// The refusals are the model's, reached through
/// [`restore::line`]: a chain that does not line up, or a table that
/// would leave two live entries under one name, fails the read.
pub fn read_line(
    head: &LineRow,
    points: &[ChangePointRow],
    rows: &[ChangeRowRow],
) -> Result<Line, DomainError> {
    let mut built = Vec::with_capacity(points.len());
    for point in points {
        let mut table = std::collections::BTreeMap::new();
        for row in rows.iter().filter(|row| row.point == point.id) {
            table.insert(
                row.entry,
                Row::new(row.existence, row.content, row.name.clone())?,
            );
        }
        built.push(restore::change_point(
            point.id,
            point.parent,
            point.from,
            point.by,
            asterism_core::domain::forge::model::table::Table::of(table)?,
            point.act.read()?,
        ));
    }

    Ok(restore::line(
        head.id,
        head.name.clone(),
        head.strategy.clone(),
        head.standing,
        restore::meta(head.created.read()?, head.updated.read()?),
        restore::genesis(head.genesis, head.genesis_act.read()?),
        built,
    )?)
}

/// Rebuilds work from its row, its nodes and their operations.
///
/// Nodes are sorted by `seq` first: the log is read forwards, and
/// [`restore::pursuit`] hands them back to `push` and `end` in the
/// order they arrive.
pub fn read_pursuit(
    head: &PursuitRow,
    nodes: &[PursuitNodeRow],
    ops: &[PursuitOpRow],
) -> Result<Pursuit, DomainError> {
    let mut ordered: Vec<&PursuitNodeRow> = nodes
        .iter()
        .filter(|node| node.pursuit == head.id)
        .collect();
    ordered.sort_by_key(|node| node.seq);

    let mut built = Vec::with_capacity(ordered.len());
    for node in ordered {
        match node.kind {
            "round" => {
                let mut wrote: Vec<&PursuitOpRow> =
                    ops.iter().filter(|op| op.node == node.id).collect();
                wrote.sort_by_key(|op| op.position);
                let mut written = Vec::with_capacity(wrote.len());
                for op in wrote {
                    written.push(read_op(op)?);
                }
                built.push(restore::Node::Round(restore::round(
                    node.id,
                    node.parent,
                    written,
                    node.note.clone(),
                    node.act.read()?,
                )?));
            }
            "close" => {
                let outcome = node.outcome.ok_or_else(|| {
                    DomainError::Validation(
                        "a stored ending does not say how the work ended".into(),
                    )
                })?;
                built.push(restore::Node::Close(restore::close(
                    node.id,
                    node.parent,
                    outcome,
                    node.note.clone(),
                    node.act.read()?,
                )));
            }
            other => {
                return Err(DomainError::Validation(format!(
                    "a stored pursuit names a node kind this model does not have: {other}"
                )));
            }
        }
    }

    Ok(restore::pursuit(
        head.id,
        head.of,
        head.parent,
        restore::meta(head.created.read()?, head.updated.read()?),
        restore::open(
            head.open,
            head.base,
            asterism_core::domain::forge::model::pursuit::Intent {
                title: head.title.clone(),
                note: head.note.clone(),
            },
            head.open_act.read()?,
        ),
        built,
    )?)
}

/// One operation, from the verb and the two payload columns that
/// travel with it.
fn read_op(row: &PursuitOpRow) -> Result<Op, DomainError> {
    let missing = |what: &str| {
        DomainError::Validation(format!("a stored `{}` operation has no {what}", row.verb))
    };
    Ok(match row.verb {
        "add" => Op::add_to(
            row.entry,
            row.content.ok_or_else(|| missing("content"))?,
            row.name.clone().ok_or_else(|| missing("name"))?,
        ),
        "replace" => Op::replace(row.entry, row.content.ok_or_else(|| missing("content"))?),
        "rename" => Op::rename(row.entry, row.name.clone().ok_or_else(|| missing("name"))?),
        "remove" => Op::remove(row.entry),
        other => {
            return Err(DomainError::Validation(format!(
                "a stored operation names a verb this model does not have: {other}"
            )));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::forge::model::act::Actor;
    use asterism_core::domain::forge::model::value::{ActorId, StrategyId};
    use chrono::TimeZone;

    fn act(minute: u32) -> Act {
        Act::new(
            Utc.with_ymd_and_hms(2026, 8, 22, 10, minute, 0).unwrap(),
            Actor::User(ActorId::new()),
        )
    }

    fn a_line() -> Line {
        Line::open(
            Name::new(Line::ROOT).unwrap(),
            StrategyId::new("by-hand").unwrap(),
            act(0),
        )
    }

    /// The standing is the one thing about a line that no history
    /// records, so a column that dropped it would take an archived
    /// line and hand back an open one — the store quietly undoing a
    /// decision. Pinned here because nothing else reaches it: no
    /// service archives a line yet, and this is the whole of the round
    /// trip that carries it.
    #[test]
    fn an_archived_line_comes_back_archived() {
        let mut line = a_line();
        line.archive(act(1));

        let head = take_new_line_apart(&line);
        assert_eq!(head.standing, Standing::Archived, "it went into the row");

        let read_back = read_line(&head, &[], &[]).expect("a line a store kept is a line");
        assert_eq!(read_back, line);
        assert_eq!(read_back.standing(), Standing::Archived);
    }

    #[test]
    fn an_open_line_comes_back_open() {
        let line = a_line();
        let read_back = read_line(&take_new_line_apart(&line), &[], &[]).unwrap();
        assert_eq!(read_back, line);
        assert_eq!(read_back.standing(), Standing::Open);
    }
}
