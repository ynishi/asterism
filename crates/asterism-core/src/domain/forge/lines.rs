//! What the forge needs kept, stated in the forge's own words.
//!
//! [`Lines`] is the whole of it. Every type it mentions —
//! [`Line`], [`ChangePoint`], [`Name`], [`Act`] — belongs to the
//! model, so nothing here has to be translated and there is no
//! vocabulary to isolate. Whatever implements it is somebody else's
//! problem, and the forge never names them.
//!
//! # Landing names the head
//!
//! [`Lines::land`] takes the node the caller believes is the head,
//! beside the change point it wants to append. It could take only the
//! change point — the parent is on it — and that would be the shape
//! that breaks.
//!
//! The model refuses a landing that does not sit on the head
//! ([`History::land`]), but it judges the line it was *given*, which
//! is the line as it was when it was read. Between the read and the
//! write, another landing can arrive. Naming the head makes the write
//! itself conditional, so whoever keeps the line can refuse the second
//! one — and the rule the model states survives the gap rather than
//! holding only for as long as nobody else is working.
//!
//! What a caller does with the refusal is read the line again and
//! rebuild, which is where the collision becomes visible.
//!
//! # There is nothing that removes
//!
//! No delete, no truncate, no rewrite of a landed node — the same
//! absence the model has, for the same reason: everything that ever
//! happened stays reachable, and a path that exists gets called.
//! Renaming a line and changing its strategy are here because they
//! move a line's own description, which is a record the history does
//! not keep.
//!
//! # One trait rather than a read half and a write half
//!
//! [`Lines::get`] and [`Lines::land`] are one concern: `land` is
//! conditional on what `get` returned, and splitting them puts the two
//! halves of that condition in two places for whoever implements them
//! to reconcile. It splits when there is a reason, and "reads and
//! writes are different words" is not one.
//!
//! # The error is the shared one
//!
//! The model refuses in its own vocabulary, but a port is where
//! failures from outside arrive — a line that is not there, a store
//! that is unreachable — and those have no forge-side name. So the
//! port speaks [`DomainError`], which the model's own refusals convert
//! into at a single edge.
//!
//! [`Line`]: crate::domain::forge::model::line::Line
//! [`ChangePoint`]: crate::domain::forge::model::history::ChangePoint
//! [`History::land`]: crate::domain::forge::model::history::History::land
//! [`Name`]: crate::domain::forge::model::value::Name
//! [`Act`]: crate::domain::forge::model::act::Act

use async_trait::async_trait;

use crate::domain::forge::model::act::Act;
use crate::domain::forge::model::history::ChangePoint;
use crate::domain::forge::model::line::{Line, Strategy};
use crate::domain::forge::model::value::{ChangePointId, LineId, Name};
// SHARED KERNEL: `DomainError` is a boundary type.
use crate::error::DomainError;

/// Keeps lines.
#[async_trait]
pub trait Lines: Send + Sync {
    /// Records a line that has just been opened, genesis and all.
    async fn open(&self, line: &Line) -> Result<(), DomainError>;

    /// Reads a line back whole, history included.
    ///
    /// Whole, because the rules the model holds are about the chain:
    /// landing checks the head, and the name check folds it. Handing
    /// back less would move those rules to whoever answers this call,
    /// and there would be as many copies of them as there are
    /// implementations.
    async fn get(&self, id: &LineId) -> Result<Option<Line>, DomainError>;

    /// Appends a change point, on the condition that `on` is still the
    /// head.
    ///
    /// Returns [`Conflict`](DomainError::Conflict) when it is not:
    /// somebody else landed first, and this caller is holding a line
    /// that has moved.
    async fn land(
        &self,
        id: &LineId,
        on: ChangePointId,
        point: &ChangePoint,
    ) -> Result<(), DomainError>;

    /// Records that a line was renamed.
    async fn rename(&self, id: &LineId, name: &Name, act: &Act) -> Result<(), DomainError>;

    /// Records that a line's strategy changed.
    async fn set_strategy(
        &self,
        id: &LineId,
        strategy: Strategy,
        act: &Act,
    ) -> Result<(), DomainError>;
}
