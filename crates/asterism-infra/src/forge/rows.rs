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
use asterism_core::domain::forge::model::thread::{Anchor, Body, Message, Revision, Thread};
use asterism_core::domain::forge::model::value::{
    ActorId, ChangePointId, Content, EntryId, Existence, LineId, MessageId, Name, NodeId,
    PursuitId, StrategyId, ThreadId,
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

/// `pursuit_node` — a pass or an ending.
///
/// No column says where in the log a node sits, for the reason
/// `change_point` has none: it carries its parent, and that is the
/// order. A sequence beside the chain would be a second answer to a
/// question the chain already answers, and the two disagree the first
/// time a write goes half-way.
#[derive(Debug, Clone)]
pub struct PursuitNodeRow {
    /// The pursuit it belongs to.
    pub pursuit: PursuitId,
    /// Which node.
    pub id: NodeId,
    /// The node before it. This is the order, and a reader walks the
    /// links — the same walk the line side does.
    pub parent: NodeId,
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

/// `forge_thread` — one conversation, and what it hangs off.
///
/// The anchor is a kind and four nullable columns rather than one id,
/// because what it points at is one of four things and no key points
/// at a column whose meaning depends on another. Two of the four are
/// rows and carry real references; a pass is a node of a pursuit and
/// an entry is not a row at all, so those two stay bare — the same
/// bareness every other node reference in the forge has, for the same
/// reason.
#[derive(Debug, Clone)]
pub struct ThreadRow {
    /// Which thread.
    pub id: ThreadId,
    /// `pursuit` / `round` / `entry` / `change_point`.
    pub kind: &'static str,
    /// Set on the three anchors that name work.
    pub pursuit: Option<PursuitId>,
    /// Set on `round` and `entry`.
    pub node: Option<NodeId>,
    /// Set on `entry` alone.
    pub entry: Option<EntryId>,
    /// Set on `change_point` alone.
    pub point: Option<ChangePointId>,
    /// What somebody called the conversation, if anybody did.
    pub title: Option<Name>,
    /// When it was opened and when it was last touched.
    pub created: ActRow,
    /// The second of those.
    pub updated: ActRow,
}

/// `forge_thread_message` — one thing said.
///
/// `said_at` is the order, which is this record and no other: a reply
/// names its parent, and two replies to one message are ordered by
/// nothing else. Everywhere else in the forge a chain is the order and
/// a stamp is evidence.
#[derive(Debug, Clone)]
pub struct ThreadMessageRow {
    /// Which message.
    pub id: MessageId,
    /// The conversation it was said in.
    pub thread: ThreadId,
    /// What it replies to, if it replies to anything.
    pub parent: Option<MessageId>,
    /// What it said when it was said. Corrections are their own rows.
    pub body: String,
    /// When it was said, and by whom.
    pub act: ActRow,
}

/// `forge_thread_revision` — a correction to something said.
///
/// The body now is the last of these, and every earlier one stays
/// readable. `position` orders them, because a correction names no
/// parent and there is no chain to read an order out of.
#[derive(Debug, Clone)]
pub struct ThreadRevisionRow {
    /// The message it corrects.
    pub message: MessageId,
    /// Its place among that message's corrections, oldest first.
    pub position: usize,
    /// What the message says from here on.
    pub body: String,
    /// When it was corrected, and by whom.
    pub act: ActRow,
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
    for round in pursuit.rounds() {
        let (node, wrote) = take_round_apart(pursuit.id(), round);
        nodes.push(node);
        ops.extend(wrote);
    }
    if let Some(close) = pursuit.close() {
        nodes.push(take_close_apart(pursuit.id(), close));
    }
    (head, nodes, ops)
}

/// One pass, for the append a push makes.
pub fn take_round_apart(pursuit: PursuitId, round: &Round) -> (PursuitNodeRow, Vec<PursuitOpRow>) {
    let node = PursuitNodeRow {
        pursuit,
        id: round.id(),
        parent: round.parent(),
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
pub fn take_close_apart(pursuit: PursuitId, close: &Close) -> PursuitNodeRow {
    PursuitNodeRow {
        pursuit,
        id: close.id(),
        parent: close.parent(),
        kind: "close",
        note: close.note().map(str::to_owned),
        act: ActRow::of(close.act()),
        outcome: Some(close.outcome()),
    }
}

/// A whole conversation, for the write that opens one.
///
/// The stamps both come from the first message: a thread is opened by
/// saying something, so the moment it was opened is the moment that
/// was said, and there is no second act to take them from.
pub fn take_thread_apart(
    thread: &Thread,
) -> (ThreadRow, Vec<ThreadMessageRow>, Vec<ThreadRevisionRow>) {
    let opened = ActRow::of(
        thread
            .messages()
            .first()
            .expect("a thread holds the message it was opened with")
            .act(),
    );
    let head = take_anchor_apart(thread.id(), thread.anchor(), thread.title(), opened);

    let mut messages = Vec::with_capacity(thread.messages().len());
    let mut revisions = Vec::new();
    for message in thread.messages() {
        messages.push(take_message_apart(thread.id(), message));
        revisions.extend(take_revisions_apart(message));
    }
    (head, messages, revisions)
}

/// What a thread hangs off, as the five columns that carry it.
///
/// Apart from the row it goes into, because asking *which threads hang
/// off this* is the same flattening with nothing to put it in: a query
/// has an anchor and no thread, no title and no act. Sharing this is
/// what keeps the answer to that question and the row it is matched
/// against from drifting apart.
pub type AnchorColumns = (
    &'static str,
    Option<PursuitId>,
    Option<NodeId>,
    Option<EntryId>,
    Option<ChangePointId>,
);

/// Flattens an anchor into the columns a store keeps it in.
pub fn anchor_columns(anchor: Anchor) -> AnchorColumns {
    match anchor {
        Anchor::Pursuit(work) => ("pursuit", Some(work), None, None, None),
        Anchor::Round(node) => ("round", None, Some(node), None, None),
        Anchor::Entry { round, entry } => ("entry", None, Some(round), Some(entry), None),
        Anchor::Change(point) => ("change_point", None, None, None, Some(point)),
    }
}

/// The head row of a thread, anchor flattened into its five columns.
pub fn take_anchor_apart(
    id: ThreadId,
    anchor: Anchor,
    title: Option<&Name>,
    act: ActRow,
) -> ThreadRow {
    let (kind, pursuit, node, entry, point) = anchor_columns(anchor);
    ThreadRow {
        id,
        kind,
        pursuit,
        node,
        entry,
        point,
        title: title.cloned(),
        created: act,
        updated: act,
    }
}

/// One thing said, for the append `say` makes.
pub fn take_message_apart(thread: ThreadId, message: &Message) -> ThreadMessageRow {
    ThreadMessageRow {
        id: message.id(),
        thread,
        parent: message.parent(),
        // What it said when it was said. The body now is the last
        // revision, and taking that here would write the correction
        // twice and lose what was corrected.
        body: message.said().as_str().to_owned(),
        act: ActRow::of(message.act()),
    }
}

/// Every correction to one message, oldest first.
pub fn take_revisions_apart(message: &Message) -> Vec<ThreadRevisionRow> {
    message
        .revisions()
        .iter()
        .enumerate()
        .map(|(position, revision)| take_revision_apart(message.id(), position, revision))
        .collect()
}

/// One correction, for the append `amend` makes.
pub fn take_revision_apart(
    message: MessageId,
    position: usize,
    revision: &Revision,
) -> ThreadRevisionRow {
    ThreadRevisionRow {
        message,
        position,
        body: revision.body().as_str().to_owned(),
        act: ActRow::of(revision.act()),
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
/// Nodes are put in the log's order first, by walking the parent
/// links from the node the pursuit opened at — the same walk the line
/// side does, and the reason neither table keeps a sequence beside the
/// chain. [`restore::pursuit`] then hands them back to `push` and
/// `end` in the order they arrive.
pub fn read_pursuit(
    head: &PursuitRow,
    nodes: &[PursuitNodeRow],
    ops: &[PursuitOpRow],
) -> Result<Pursuit, DomainError> {
    let ordered = chain(head, nodes)?;

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

/// Puts a pursuit's nodes in the log's order, walking from the node it
/// opened at.
///
/// The same shape as [`restore`]'s walk over a line's chain, and here
/// for the same reason: the parent is the order, so nothing has to
/// keep a second answer beside it.
///
/// Refuses anything that is not one chain covering every node given. A
/// leftover is a node the walk could not reach — a log with a hole in
/// it, or a second branch — and neither is a thing to read the
/// reachable part of.
fn chain<'a>(
    head: &PursuitRow,
    nodes: &'a [PursuitNodeRow],
) -> Result<Vec<&'a PursuitNodeRow>, DomainError> {
    let mut by_parent: std::collections::HashMap<NodeId, &PursuitNodeRow> =
        std::collections::HashMap::new();
    for node in nodes.iter().filter(|node| node.pursuit == head.id) {
        if by_parent.insert(node.parent, node).is_some() {
            return Err(DomainError::Validation(format!(
                "pursuit {} has two nodes on one parent, which is a log that forked",
                head.id
            )));
        }
    }

    let total = by_parent.len();
    let mut ordered = Vec::with_capacity(total);
    let mut at = head.open;
    while let Some(node) = by_parent.remove(&at) {
        at = node.id;
        ordered.push(node);
    }
    if !by_parent.is_empty() {
        return Err(DomainError::Validation(format!(
            "pursuit {} has {} nodes the walk from its opening cannot reach",
            head.id,
            by_parent.len()
        )));
    }
    Ok(ordered)
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

/// Rebuilds a conversation from its row, what was said, and every
/// correction.
///
/// Messages are put in the order they were said, which for a thread is
/// a stamp and not a chain. [`restore::thread`] keeps that order,
/// moving only a reply the stamps put before the message it answers,
/// and hands them back to `say` one at a time — so a reply naming a
/// message this thread does not hold is a read that fails. A reply
/// kept with an earlier stamp than its parent is not that case: it is
/// a clock that stepped backwards, and it is put back after what it
/// answers rather than refused.
pub fn read_thread(
    head: &ThreadRow,
    messages: &[ThreadMessageRow],
    revisions: &[ThreadRevisionRow],
) -> Result<Thread, DomainError> {
    let mut said: Vec<&ThreadMessageRow> = messages
        .iter()
        .filter(|row| row.thread == head.id)
        .collect();
    said.sort_by_key(|row| row.act.at);

    let mut built = Vec::with_capacity(said.len());
    for row in said {
        let mut corrections: Vec<&ThreadRevisionRow> = revisions
            .iter()
            .filter(|revision| revision.message == row.id)
            .collect();
        corrections.sort_by_key(|revision| revision.position);

        let mut made = Vec::with_capacity(corrections.len());
        for correction in corrections {
            made.push(Revision::new(
                read_body(&correction.body)?,
                correction.act.read()?,
            ));
        }
        built.push(restore::message(
            row.id,
            row.parent,
            read_body(&row.body)?,
            row.act.read()?,
            made,
        ));
    }

    Ok(restore::thread(
        head.id,
        read_anchor(head)?,
        head.title.clone(),
        built,
    )?)
}

/// The anchor a thread's five columns describe.
fn read_anchor(head: &ThreadRow) -> Result<Anchor, DomainError> {
    /// What a stored anchor promised and did not carry.
    fn missing(kind: &str, column: &str) -> DomainError {
        DomainError::Validation(format!(
            "a stored thread anchored to a {kind} does not say which: `{column}` is empty"
        ))
    }

    match head.kind {
        "pursuit" => Ok(Anchor::Pursuit(
            head.pursuit
                .ok_or_else(|| missing("pursuit", "anchor_pursuit"))?,
        )),
        "round" => Ok(Anchor::Round(
            head.node.ok_or_else(|| missing("pass", "anchor_node"))?,
        )),
        "entry" => Ok(Anchor::Entry {
            round: head.node.ok_or_else(|| missing("entry", "anchor_node"))?,
            entry: head.entry.ok_or_else(|| missing("entry", "anchor_entry"))?,
        }),
        "change_point" => {
            Ok(Anchor::Change(head.point.ok_or_else(|| {
                missing("change point", "anchor_change_point")
            })?))
        }
        other => Err(DomainError::Validation(format!(
            "a stored thread hangs off a kind of thing this model does not have: {other}"
        ))),
    }
}

/// What was said, refused if a store kept nothing.
fn read_body(said: &str) -> Result<Body, DomainError> {
    Ok(Body::new(said)?)
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

    /// A message's row carries what was said, not what it says now.
    ///
    /// The two differ only once a correction exists, and no port call
    /// hands over a message that already has one: `open` takes a
    /// thread that was just opened and `say` a message that was just
    /// said. So this is asked here, of the translation itself, rather
    /// than through a store that cannot reach the case — writing the
    /// body would put the correction in the message row *and* in a
    /// revision row, and what was corrected would be the thing that
    /// went missing.
    #[test]
    fn a_message_row_carries_what_was_said_and_the_revisions_carry_the_rest() {
        let thread = ThreadId::new();
        let mut message = Message::new(None, Body::new("this reads oddly").unwrap(), act(1));
        message.amend(Revision::new(
            Body::new("this reads oddly to me").unwrap(),
            act(2),
        ));

        let row = take_message_apart(thread, &message);
        assert_eq!(row.body, "this reads oddly", "the row keeps what was said");
        assert_eq!(row.thread, thread);

        let revisions = take_revisions_apart(&message);
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].body, "this reads oddly to me");
        assert_eq!(revisions[0].position, 0);

        // And the pair reads back as the message it came from.
        let head = take_anchor_apart(
            thread,
            Anchor::Pursuit(PursuitId::new()),
            None,
            ActRow::of(&act(1)),
        );
        let read_back = read_thread(&head, &[row], &revisions).expect("a kept conversation");
        assert_eq!(read_back.messages()[0].said().as_str(), "this reads oddly");
        assert_eq!(
            read_back.messages()[0].body().as_str(),
            "this reads oddly to me"
        );
    }
}
