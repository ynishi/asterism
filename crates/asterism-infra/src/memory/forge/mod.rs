//! The forge's ports, over rows held in memory.
//!
//! ```text
//!   Lines / Pursuits / Closings / Threads
//!            │
//!            ├─ write ──► forge::rows::take_*_apart ──► Vec under a Mutex
//!            │
//!            └─ read  ──► forge::rows::read_* ──► restore ──► Line /
//!                                                 Pursuit / Thread
//! ```
//!
//! One `Mutex` over the whole store rather than one per table: the
//! close writes a change point, its rows and an ending together, and a
//! reader must not see half of that. A real adapter gets the same
//! property from a transaction; here it comes from holding the lock
//! across the whole of `commit`, which is the same statement made the
//! only way this store can make it.
//!
//! # What it is for
//!
//! Proving the model and the services against something that keeps
//! rows, before any of it depends on SQLite being right. A store that
//! kept the domain objects would answer every call correctly by
//! construction and would never once ask whether a line can be built
//! back out of what was written down — which is the question the whole
//! read half exists to ask, and the one the first fake never reached.
//!
//! # What it is not
//!
//! Durable, concurrent beyond one process, or a thing to run anything
//! real on. There is no index: reads scan, which is fine at the sizes
//! a test builds and would not be at any other.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::forge::boundary::{Actors, Store};
use asterism_core::domain::forge::closings::{Closings, Deciding};
use asterism_core::domain::forge::lines::Lines;
use asterism_core::domain::forge::model::act::Act;
use asterism_core::domain::forge::model::closing::Closing;
use asterism_core::domain::forge::model::line::{Line, Standing};
use asterism_core::domain::forge::model::pursuit::{Pursuit, Round};
use asterism_core::domain::forge::model::thread::{Anchor, Message, Revision, Thread};
use asterism_core::domain::forge::model::value::{
    ActorId, ChangePointId, LineId, MessageId, Name, NodeId, PursuitId, StrategyId, ThreadId,
};
use asterism_core::domain::forge::pursuits::Pursuits;
use asterism_core::domain::forge::threads::Threads;
use asterism_core::domain::value::AssetId;
use asterism_core::error::DomainError;
use async_trait::async_trait;

use crate::forge::rows::{
    self, ChangePointRow, ChangeRowRow, LineRow, PursuitNodeRow, PursuitOpRow, PursuitRow,
    ThreadMessageRow, ThreadRevisionRow, ThreadRow,
};

/// Everything the store holds, in the shape it holds it.
#[derive(Debug, Default)]
struct Tables {
    lines: Vec<LineRow>,
    change_points: Vec<ChangePointRow>,
    change_rows: Vec<ChangeRowRow>,
    pursuits: Vec<PursuitRow>,
    pursuit_nodes: Vec<PursuitNodeRow>,
    pursuit_ops: Vec<PursuitOpRow>,
    threads: Vec<ThreadRow>,
    thread_messages: Vec<ThreadMessageRow>,
    thread_revisions: Vec<ThreadRevisionRow>,
}

/// An in-memory forge store. Clone it to hand the same tables to every
/// port — the services take one `Arc` each, and they all have to be
/// looking at the same rows.
#[derive(Debug, Clone, Default)]
pub struct MemoryForge {
    tables: Arc<Mutex<Tables>>,
}

impl MemoryForge {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn with<T>(&self, job: impl FnOnce(&mut Tables) -> T) -> T {
        let mut tables = self
            .tables
            .lock()
            .expect("the store's lock is not poisoned");
        job(&mut tables)
    }

    /// Writes a change point and its rows with nothing checking them.
    ///
    /// The only way to produce a stored state the model would have
    /// refused, and it exists so that the read half can be asked what
    /// it does with one. Every ordinary path here goes through a
    /// domain value that was legal when it was built; a repair job, a
    /// bad migration or a hand-edited database is not bound by that,
    /// and "what happens then" is a question worth being able to ask
    /// on purpose rather than discovering.
    ///
    /// Named so that its callers are obvious in a grep, and so that
    /// nothing reaches for it to make a refusal go away.
    pub fn force_rows(&self, line: LineId, point: ChangePointRow, rows: Vec<ChangeRowRow>) {
        self.with(|tables| {
            debug_assert_eq!(point.line, line, "the point is filed under the line given");
            tables.change_points.push(point);
            tables.change_rows.extend(rows);
        });
    }

    /// Rebuilds one line from the rows under `id`.
    fn line_at(tables: &Tables, id: &LineId) -> Result<Option<Line>, DomainError> {
        let Some(head) = tables.lines.iter().find(|row| row.id == *id) else {
            return Ok(None);
        };
        let points: Vec<ChangePointRow> = tables
            .change_points
            .iter()
            .filter(|row| row.line == *id)
            .cloned()
            .collect();
        rows::read_line(head, &points, &tables.change_rows).map(Some)
    }

    /// Whether either half of this closing would land on a parent
    /// something already has.
    ///
    /// The two rules the SQLite adapter gets from `UNIQUE (line_id,
    /// parent_id)` and `UNIQUE (pursuit_id, parent_id)`, asked the only
    /// way this store can ask them — and both, because
    /// [`Deciding`] names both as races and a store implementing one of
    /// them would leave the other to whichever store was asked. No
    /// close compares a head: the closing carries the nodes it was
    /// decided against, which is the only account of them anybody
    /// needs. A push is the exception, and it is this store's alone —
    /// see `pursuit_head` below.
    ///
    /// The line is asked about only when something is going on it. An
    /// abandoned close puts nothing there, so refusing one because
    /// somebody else landed first would refuse work for giving up in
    /// the wrong millisecond — the ending, which every close has, is
    /// asked about either way.
    fn taken(tables: &Tables, line: &LineId, pursuit: &PursuitId, closing: &Closing) -> bool {
        let forked = tables
            .pursuit_nodes
            .iter()
            .any(|row| row.pursuit == *pursuit && row.parent == closing.close().parent());
        let landed = closing.point().is_some_and(|point| {
            tables
                .change_points
                .iter()
                .any(|row| row.line == *line && row.parent == point.parent())
        });
        forked || landed
    }

    /// Whether this conversation is about something on this line.
    ///
    /// Three branches for four anchors; the SQLite side's
    /// `THREADS_OF_A_LINE` says why, and asks the same three as one
    /// subquery.
    fn anchored_in(tables: &Tables, thread: &ThreadRow, line: &LineId) -> bool {
        let of_this_line = |work: &PursuitId| {
            tables
                .pursuits
                .iter()
                .any(|row| row.id == *work && row.of == *line)
        };

        if thread.pursuit.is_some_and(|work| of_this_line(&work)) {
            return true;
        }
        if let Some(node) = thread.node
            && tables
                .pursuit_nodes
                .iter()
                .any(|row| row.id == node && of_this_line(&row.pursuit))
        {
            return true;
        }
        thread.point.is_some_and(|point| {
            tables
                .change_points
                .iter()
                .any(|row| row.id == point && row.line == *line)
        })
    }

    /// Rebuilds one conversation from the rows under `id`.
    fn thread_at(tables: &Tables, id: &ThreadId) -> Result<Option<Thread>, DomainError> {
        let Some(head) = tables.threads.iter().find(|row| row.id == *id) else {
            return Ok(None);
        };
        rows::read_thread(head, &tables.thread_messages, &tables.thread_revisions).map(Some)
    }

    /// Rebuilds one piece of work from the rows under `id`.
    fn pursuit_at(tables: &Tables, id: &PursuitId) -> Result<Option<Pursuit>, DomainError> {
        let Some(head) = tables.pursuits.iter().find(|row| row.id == *id) else {
            return Ok(None);
        };
        rows::read_pursuit(head, &tables.pursuit_nodes, &tables.pursuit_ops).map(Some)
    }

    /// The node a pursuit currently ends at, read off the rows rather
    /// than off a rebuilt value: the caller asking is checking whether
    /// its own copy has moved, and rebuilding to answer that would do
    /// the expensive half of a read for a comparison of two ids.
    fn pursuit_head(tables: &Tables, id: &PursuitId) -> Option<NodeId> {
        let head = tables.pursuits.iter().find(|row| row.id == *id)?;
        // Walk from the node it opened at, because the parent is the
        // order — there is no sequence to take a maximum of.
        let by_parent: HashMap<NodeId, NodeId> = tables
            .pursuit_nodes
            .iter()
            .filter(|node| node.pursuit == *id)
            .map(|node| (node.parent, node.id))
            .collect();
        let mut at = head.open;
        while let Some(next) = by_parent.get(&at) {
            at = *next;
        }
        Some(at)
    }

    // There was a `line_head` here, walking the chain to say where a
    // line ends. Nothing reads it any more: a close is refused by the
    // parent already being taken, which is the rule the SQLite index
    // states, so neither store compares a head to decide a close.
    //
    // A push is where the two stores differ, and `pursuit_head` above
    // is the difference: this one compares it to the node the caller
    // was holding, where the SQLite side writes and lets
    // `UNIQUE (pursuit_id, parent_id)` refuse the second round.
}

#[async_trait]
impl Lines for MemoryForge {
    async fn open(&self, line: &Line) -> Result<(), DomainError> {
        // The port records an opening, not a history. A change point
        // reaches the store through `Closings::commit` and nowhere
        // else, which is what keeps "the line moved" and "work ended"
        // one write rather than two paths that could disagree.
        if !line.history().changes().is_empty() {
            return Err(DomainError::Validation(
                "this port records a line that has just been opened; a history reaches \
                 the store one close at a time"
                    .into(),
            ));
        }
        let head = rows::take_new_line_apart(line);
        self.with(|tables| {
            if tables.lines.iter().any(|row| row.id == head.id) {
                return Err(DomainError::clashes(format!(
                    "line {} is already open",
                    head.id
                )));
            }
            tables.lines.push(head);
            Ok(())
        })
    }

    async fn get(&self, id: &LineId) -> Result<Option<Line>, DomainError> {
        self.with(|tables| Self::line_at(tables, id))
    }

    async fn list(&self) -> Result<Vec<Line>, DomainError> {
        self.with(|tables| {
            tables
                .lines
                .iter()
                .map(|head| {
                    let points: Vec<ChangePointRow> = tables
                        .change_points
                        .iter()
                        .filter(|row| row.line == head.id)
                        .cloned()
                        .collect();
                    rows::read_line(head, &points, &tables.change_rows)
                })
                .collect()
        })
    }

    async fn rename(&self, id: &LineId, name: &Name, act: &Act) -> Result<(), DomainError> {
        self.with(|tables| {
            let row = tables
                .lines
                .iter_mut()
                .find(|row| row.id == *id)
                .ok_or_else(|| DomainError::not_found("line", id))?;
            row.name = name.clone();
            row.updated = rows::ActRow::of(act);
            Ok(())
        })
    }

    async fn set_strategy(
        &self,
        id: &LineId,
        strategy: &StrategyId,
        act: &Act,
    ) -> Result<(), DomainError> {
        self.with(|tables| {
            let row = tables
                .lines
                .iter_mut()
                .find(|row| row.id == *id)
                .ok_or_else(|| DomainError::not_found("line", id))?;
            row.strategy = strategy.clone();
            row.updated = rows::ActRow::of(act);
            Ok(())
        })
    }

    async fn set_standing(
        &self,
        id: &LineId,
        standing: Standing,
        act: &Act,
    ) -> Result<(), DomainError> {
        self.with(|tables| {
            let row = tables
                .lines
                .iter_mut()
                .find(|row| row.id == *id)
                .ok_or_else(|| DomainError::not_found("line", id))?;
            row.standing = standing;
            row.updated = rows::ActRow::of(act);
            Ok(())
        })
    }

    async fn discard(&self, id: &LineId, covering: &[PursuitId]) -> Result<(), DomainError> {
        self.with(|tables| {
            let Some(line) = tables.lines.iter().find(|row| row.id == *id) else {
                return Err(DomainError::not_found("line", id));
            };

            // The first condition the port states, asked of the rows
            // rather than of the caller's copy: a drop is decided
            // against an archived line, and a line taken back out of
            // the archive in between is the race `covering` exists to
            // distrust, one field over.
            if line.standing != Standing::Archived {
                return Err(DomainError::raced(format!(
                    "line {id} is out of the archive again, and a drop is decided against an \
                     archived line"
                )));
            }

            // The second: the work against this line has to be the
            // work the caller decided against. Order is not part of
            // it, and neither is how the caller found them.
            let against: BTreeSet<PursuitId> = tables
                .pursuits
                .iter()
                .filter(|row| row.of == *id)
                .map(|row| row.id)
                .collect();
            let named: BTreeSet<PursuitId> = covering.iter().copied().collect();
            // Two ways the sets differ, and they are not one refusal.
            // Work this drop did not name is work opened since the
            // caller read the list — a race. A name that is not
            // against this line cannot have got there by a race:
            // nothing removes a pursuit but a drop of its line, and
            // that line is here. It is the model's `NotThisLine`,
            // arriving one layer down.
            let elsewhere = named.difference(&against).count();
            if elsewhere > 0 {
                return Err(DomainError::Validation(format!(
                    "this drop of line {id} names {elsewhere} pieces of work that are not \
                     against it, and what another line holds is not this drop's to release"
                )));
            }
            let opened = against.difference(&named).count();
            if opened > 0 {
                return Err(DomainError::raced(format!(
                    "{opened} pieces of work have been opened on line {id} since this drop was \
                     decided, and what it releases was decided without them"
                )));
            }

            // What was said about any of it goes too, and it goes
            // first: a remark hangs off a pursuit, a round, an entry as
            // a round had it, or a change point, and all four are about
            // to stop existing. This store has no key to refuse a
            // thread left behind, which is exactly why it has to be
            // the one that remembers — the SQLite side would be told.
            let threads: BTreeSet<ThreadId> = tables
                .threads
                .iter()
                .filter(|row| Self::anchored_in(tables, row, id))
                .map(|row| row.id)
                .collect();
            let said: BTreeSet<MessageId> = tables
                .thread_messages
                .iter()
                .filter(|row| threads.contains(&row.thread))
                .map(|row| row.id)
                .collect();
            tables
                .thread_revisions
                .retain(|row| !said.contains(&row.message));
            tables
                .thread_messages
                .retain(|row| !threads.contains(&row.thread));
            tables.threads.retain(|row| !threads.contains(&row.id));

            // Rows first, then what they hang off, so that a reader
            // holding the lock could never see a node whose parent
            // table has already gone. Nothing here is conditional any
            // more: the answer was settled above and this is the whole
            // of the write.
            let nodes: BTreeSet<NodeId> = tables
                .pursuit_nodes
                .iter()
                .filter(|row| named.contains(&row.pursuit))
                .map(|row| row.id)
                .collect();
            tables.pursuit_ops.retain(|row| !nodes.contains(&row.node));
            tables
                .pursuit_nodes
                .retain(|row| !named.contains(&row.pursuit));
            tables.pursuits.retain(|row| !named.contains(&row.id));

            let points: BTreeSet<ChangePointId> = tables
                .change_points
                .iter()
                .filter(|row| row.line == *id)
                .map(|row| row.id)
                .collect();
            tables
                .change_rows
                .retain(|row| !points.contains(&row.point));
            tables.change_points.retain(|row| row.line != *id);
            tables.lines.retain(|row| row.id != *id);
            Ok(())
        })
    }
}

#[async_trait]
impl Pursuits for MemoryForge {
    async fn open(&self, pursuit: &Pursuit) -> Result<(), DomainError> {
        let (head, nodes, ops) = rows::take_pursuit_apart(pursuit);
        self.with(|tables| {
            if tables.pursuits.iter().any(|row| row.id == head.id) {
                return Err(DomainError::clashes(format!(
                    "work {} is already open",
                    head.id
                )));
            }
            tables.pursuits.push(head);
            tables.pursuit_nodes.extend(nodes);
            tables.pursuit_ops.extend(ops);
            Ok(())
        })
    }

    async fn get(&self, id: &PursuitId) -> Result<Option<Pursuit>, DomainError> {
        self.with(|tables| Self::pursuit_at(tables, id))
    }

    async fn of_line(&self, line: &LineId) -> Result<Vec<Pursuit>, DomainError> {
        self.with(|tables| {
            tables
                .pursuits
                .iter()
                .filter(|row| row.of == *line)
                .map(|row| rows::read_pursuit(row, &tables.pursuit_nodes, &tables.pursuit_ops))
                .collect()
        })
    }

    async fn children(&self, parent: &PursuitId) -> Result<Vec<Pursuit>, DomainError> {
        self.with(|tables| {
            tables
                .pursuits
                .iter()
                .filter(|row| row.parent == Some(*parent))
                .map(|row| rows::read_pursuit(row, &tables.pursuit_nodes, &tables.pursuit_ops))
                .collect()
        })
    }

    async fn push(&self, id: &PursuitId, on: NodeId, round: &Round) -> Result<(), DomainError> {
        self.with(|tables| {
            let at =
                Self::pursuit_head(tables, id).ok_or_else(|| DomainError::not_found("work", id))?;
            if at != on {
                return Err(DomainError::raced(format!(
                    "work {id} has moved: this round sits on {on}, and the log ends at {at}"
                )));
            }
            let (node, ops) = rows::take_round_apart(*id, round);
            tables.pursuit_nodes.push(node);
            tables.pursuit_ops.extend(ops);
            Ok(())
        })
    }
}

#[async_trait]
impl Closings for MemoryForge {
    async fn commit(
        &self,
        line: &LineId,
        pursuit: &PursuitId,
        closing: &Closing,
        again: Arc<dyn Deciding>,
    ) -> Result<(), DomainError> {
        self.with(|tables| {
            if !tables.lines.iter().any(|row| row.id == *line) {
                return Err(DomainError::not_found("line", line));
            }

            // Decided outside this lock, so either log may have moved
            // since. If one has, the answer is decided again from what
            // is in front of us — and this lock is why that one is
            // final: nothing can arrive between deciding and writing,
            // because the whole of both is in here.
            let decided;
            let closing = if Self::taken(tables, line, pursuit, closing) {
                let held = Self::line_at(tables, line)?
                    .ok_or_else(|| DomainError::not_found("line", line))?;
                let work = Self::pursuit_at(tables, pursuit)?
                    .ok_or_else(|| DomainError::not_found("work", pursuit))?;
                decided = again.close(&held, &work)?;
                if Self::taken(tables, line, pursuit, &decided) {
                    return Err(DomainError::raced(format!(
                        "an ending decided against line {line} as this write finds it still \
                         names a parent something has taken"
                    )));
                }
                &decided
            } else {
                closing
            };

            // Assembled before anything is pushed, so a refusal from
            // the translation leaves the store as it was. Both halves
            // go on together or neither does — the property the whole
            // port exists for, and the only way this store can state
            // it.
            let ending = rows::take_close_apart(*pursuit, closing.close());
            let landing = closing
                .point()
                .map(|point| rows::take_change_point_apart(*line, point));

            tables.pursuit_nodes.push(ending);
            if let Some((point, change_rows)) = landing {
                tables.change_points.push(point);
                tables.change_rows.extend(change_rows);
            }
            Ok(())
        })
    }
}

#[async_trait]
impl Threads for MemoryForge {
    async fn open(&self, thread: &Thread) -> Result<(), DomainError> {
        let (head, messages, revisions) = rows::take_thread_apart(thread);
        self.with(|tables| {
            if tables.threads.iter().any(|row| row.id == head.id) {
                return Err(DomainError::clashes(format!(
                    "thread {} is already open",
                    head.id
                )));
            }
            tables.threads.push(head);
            tables.thread_messages.extend(messages);
            tables.thread_revisions.extend(revisions);
            Ok(())
        })
    }

    async fn get(&self, id: &ThreadId) -> Result<Option<Thread>, DomainError> {
        self.with(|tables| Self::thread_at(tables, id))
    }

    async fn anchored(&self, anchor: Anchor) -> Result<Vec<Thread>, DomainError> {
        // Compared as columns rather than as values, because that is
        // what the SQLite side can index and this store exists to say
        // what that side owes.
        let (kind, pursuit, node, entry, point) = rows::anchor_columns(anchor);
        self.with(|tables| {
            tables
                .threads
                .iter()
                .filter(|row| {
                    row.kind == kind
                        && row.pursuit == pursuit
                        && row.node == node
                        && row.entry == entry
                        && row.point == point
                })
                .map(|row| {
                    rows::read_thread(row, &tables.thread_messages, &tables.thread_revisions)
                })
                .collect()
        })
    }

    async fn say(&self, thread: &ThreadId, message: &Message) -> Result<(), DomainError> {
        let row = rows::take_message_apart(*thread, message);
        self.with(|tables| {
            if !tables.threads.iter().any(|held| held.id == *thread) {
                return Err(DomainError::not_found("thread", thread));
            }
            // The model's refusal, asked of the rows: a reply reaching
            // out of its own conversation would make "the thread this
            // belongs to" a question with two answers. The model asked
            // it of the thread as the caller read it; this asks it of
            // the thread as it is being written to.
            if let Some(parent) = row.parent
                && !tables
                    .thread_messages
                    .iter()
                    .any(|held| held.thread == *thread && held.id == parent)
            {
                return Err(DomainError::clashes(format!(
                    "message {parent} is not in thread {thread}"
                )));
            }
            tables.thread_messages.push(row);
            Ok(())
        })
    }

    async fn amend(
        &self,
        thread: &ThreadId,
        message: &MessageId,
        revision: &Revision,
    ) -> Result<(), DomainError> {
        self.with(|tables| {
            if !tables
                .thread_messages
                .iter()
                .any(|held| held.thread == *thread && held.id == *message)
            {
                return Err(DomainError::clashes(format!(
                    "message {message} is not in thread {thread}"
                )));
            }
            // Its place among that message's corrections, which is the
            // count of the ones already there — appended, never
            // renumbered.
            let position = tables
                .thread_revisions
                .iter()
                .filter(|held| held.message == *message)
                .count();
            tables
                .thread_revisions
                .push(rows::take_revision_apart(*message, position, revision));
            Ok(())
        })
    }

    async fn rename(
        &self,
        id: &ThreadId,
        title: Option<&Name>,
        act: &Act,
    ) -> Result<(), DomainError> {
        self.with(|tables| {
            let row = tables
                .threads
                .iter_mut()
                .find(|row| row.id == *id)
                .ok_or_else(|| DomainError::not_found("thread", id))?;
            row.title = title.cloned();
            row.updated = rows::ActRow::of(act);
            Ok(())
        })
    }
}

/// What the layer below answers, for a store that has no layer below.
///
/// Says yes to everything. The question is whether an asset exists,
/// and there are no assets here to exist — a caller wanting the
/// refusal exercised should reach for [`HoldsNothing`].
#[derive(Debug, Clone, Copy, Default)]
pub struct HoldsEverything;

#[async_trait]
impl Store for HoldsEverything {
    async fn exists(&self, _asset: &AssetId) -> Result<bool, DomainError> {
        Ok(true)
    }
}

/// The same face, answering no.
#[derive(Debug, Clone, Copy, Default)]
pub struct HoldsNothing;

#[async_trait]
impl Store for HoldsNothing {
    async fn exists(&self, _asset: &AssetId) -> Result<bool, DomainError> {
        Ok(false)
    }
}

/// Handles for whoever is writing, minted once per subject and kept.
///
/// The forge asks who a write is by and records the answer; what a
/// handle stands for is somebody else's question. This answers it the
/// way an authenticated deployment eventually will — the same subject
/// gets the same handle back — without knowing what a subject is
/// beyond the string the attribution triple carries.
#[derive(Debug, Clone, Default)]
pub struct MemoryActors {
    known: Arc<Mutex<HashMap<String, ActorId>>>,
    server: Arc<Mutex<Option<ActorId>>>,
}

impl MemoryActors {
    /// An empty set of handles.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Actors for MemoryActors {
    async fn resolve(&self, by: &AttributionContext) -> Result<ActorId, DomainError> {
        // The subject as the triple states it. An unattributed write
        // is one actor rather than a new one each time: "nobody said
        // who" is a single answer, not a crowd.
        let subject = format!("{:?}", by.author());
        let mut known = self.known.lock().expect("the handle map is not poisoned");
        Ok(*known.entry(subject).or_default())
    }

    async fn server(&self) -> Result<ActorId, DomainError> {
        let mut server = self
            .server
            .lock()
            .expect("the server handle is not poisoned");
        Ok(*server.get_or_insert_with(ActorId::new))
    }
}
