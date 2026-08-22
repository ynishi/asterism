//! The forge's ports, over rows held in memory.
//!
//! ```text
//!   Lines / Pursuits / Closings
//!            │
//!            ├─ write ──► rows::take_*_apart ──► Vec<Row> under a Mutex
//!            │
//!            └─ read  ──► rows::read_* ──► model::restore ──► Line / Pursuit
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

pub mod rows;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::forge::boundary::{Actors, Store};
use asterism_core::domain::forge::closings::Closings;
use asterism_core::domain::forge::lines::Lines;
use asterism_core::domain::forge::model::act::Act;
use asterism_core::domain::forge::model::closing::Closing;
use asterism_core::domain::forge::model::line::Line;
use asterism_core::domain::forge::model::pursuit::{Pursuit, Round};
use asterism_core::domain::forge::model::value::{
    ActorId, ChangePointId, LineId, Name, NodeId, PursuitId, StrategyId,
};
use asterism_core::domain::forge::pursuits::Pursuits;
use asterism_core::domain::value::{AssetId, PersonaId};
use asterism_core::error::DomainError;
use async_trait::async_trait;

use rows::{ChangePointRow, ChangeRowRow, LineRow, WorkNodeRow, WorkOpRow, WorkRow};

/// Everything the store holds, in the shape it holds it.
#[derive(Debug, Default)]
struct Tables {
    lines: Vec<LineRow>,
    change_points: Vec<ChangePointRow>,
    change_rows: Vec<ChangeRowRow>,
    work: Vec<WorkRow>,
    work_nodes: Vec<WorkNodeRow>,
    work_ops: Vec<WorkOpRow>,
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
    /// Named so that its one caller is obvious in a grep, and so that
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

    /// Rebuilds one piece of work from the rows under `id`.
    fn work_at(tables: &Tables, id: &PursuitId) -> Result<Option<Pursuit>, DomainError> {
        let Some(head) = tables.work.iter().find(|row| row.id == *id) else {
            return Ok(None);
        };
        rows::read_work(head, &tables.work_nodes, &tables.work_ops).map(Some)
    }

    /// The node a work log currently ends at, read off the rows rather
    /// than off a rebuilt value: the caller asking is checking whether
    /// its own copy has moved, and rebuilding to answer that would do
    /// the expensive half of a read for a comparison of two ids.
    fn work_head(tables: &Tables, id: &PursuitId) -> Option<NodeId> {
        let head = tables.work.iter().find(|row| row.id == *id)?;
        tables
            .work_nodes
            .iter()
            .filter(|node| node.work == *id)
            .max_by_key(|node| node.seq)
            .map(|node| node.id)
            .or(Some(head.open))
    }

    /// The change point a line currently ends at, read the same way
    /// and for the same reason.
    fn line_head(tables: &Tables, id: &LineId) -> Option<ChangePointId> {
        let head = tables.lines.iter().find(|row| row.id == *id)?;
        let mut at = head.genesis;
        let by_parent: HashMap<ChangePointId, ChangePointId> = tables
            .change_points
            .iter()
            .filter(|row| row.line == *id)
            .map(|row| (row.parent, row.id))
            .collect();
        while let Some(next) = by_parent.get(&at) {
            at = *next;
        }
        Some(at)
    }
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
                return Err(DomainError::Conflict(format!(
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
}

#[async_trait]
impl Pursuits for MemoryForge {
    async fn open(&self, pursuit: &Pursuit) -> Result<(), DomainError> {
        let (head, nodes, ops) = rows::take_work_apart(pursuit);
        self.with(|tables| {
            if tables.work.iter().any(|row| row.id == head.id) {
                return Err(DomainError::Conflict(format!(
                    "work {} is already open",
                    head.id
                )));
            }
            tables.work.push(head);
            tables.work_nodes.extend(nodes);
            tables.work_ops.extend(ops);
            Ok(())
        })
    }

    async fn get(&self, id: &PursuitId) -> Result<Option<Pursuit>, DomainError> {
        self.with(|tables| Self::work_at(tables, id))
    }

    async fn of_line(&self, line: &LineId) -> Result<Vec<Pursuit>, DomainError> {
        self.with(|tables| {
            tables
                .work
                .iter()
                .filter(|row| row.of == *line)
                .map(|row| rows::read_work(row, &tables.work_nodes, &tables.work_ops))
                .collect()
        })
    }

    async fn children(&self, parent: &PursuitId) -> Result<Vec<Pursuit>, DomainError> {
        self.with(|tables| {
            tables
                .work
                .iter()
                .filter(|row| row.parent == Some(*parent))
                .map(|row| rows::read_work(row, &tables.work_nodes, &tables.work_ops))
                .collect()
        })
    }

    async fn push(&self, id: &PursuitId, on: NodeId, round: &Round) -> Result<(), DomainError> {
        self.with(|tables| {
            let at =
                Self::work_head(tables, id).ok_or_else(|| DomainError::not_found("work", id))?;
            if at != on {
                return Err(DomainError::Conflict(format!(
                    "work {id} has moved: this pass sits on {on}, and the log ends at {at}"
                )));
            }
            let seq = tables
                .work_nodes
                .iter()
                .filter(|node| node.work == *id)
                .count();
            let (node, ops) = rows::take_round_apart(*id, round, seq);
            tables.work_nodes.push(node);
            tables.work_ops.extend(ops);
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
        on: ChangePointId,
        closing: &Closing,
    ) -> Result<(), DomainError> {
        self.with(|tables| {
            let at = Self::line_head(tables, line)
                .ok_or_else(|| DomainError::not_found("line", line))?;
            if at != on {
                return Err(DomainError::Conflict(format!(
                    "line {line} has moved: this close lands on {on}, and the line ends at {at}"
                )));
            }
            let seq = tables
                .work_nodes
                .iter()
                .filter(|node| node.work == *pursuit)
                .count();

            // Assembled before anything is pushed, so a refusal from
            // the translation leaves the store as it was. Both halves
            // go on together or neither does — the property the whole
            // port exists for, and the only way this store can state
            // it.
            let ending = rows::take_close_apart(*pursuit, closing.close(), seq);
            let landing = closing
                .point()
                .map(|point| rows::take_change_point_apart(*line, point));

            tables.work_nodes.push(ending);
            if let Some((point, change_rows)) = landing {
                tables.change_points.push(point);
                tables.change_rows.extend(change_rows);
            }
            Ok(())
        })
    }
}

/// What the layer below answers, for a store that has no layer below.
///
/// Says yes to everything. The question is whether a persona holds an
/// asset, and there are no assets here to hold — a caller wanting the
/// refusal exercised should reach for [`HoldsNothing`].
#[derive(Debug, Clone, Copy, Default)]
pub struct HoldsEverything;

#[async_trait]
impl Store for HoldsEverything {
    async fn owns(&self, _persona: &PersonaId, _asset: &AssetId) -> Result<bool, DomainError> {
        Ok(true)
    }
}

/// The same face, answering no.
#[derive(Debug, Clone, Copy, Default)]
pub struct HoldsNothing;

#[async_trait]
impl Store for HoldsNothing {
    async fn owns(&self, _persona: &PersonaId, _asset: &AssetId) -> Result<bool, DomainError> {
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
