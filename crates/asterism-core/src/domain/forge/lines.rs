//! Keeping lines, stated in the forge's own words.
//!
//! [`Lines`] is one of the two faces the forge asks for. Every type it
//! mentions — [`Line`], [`Name`], [`Act`] — belongs to the model, so
//! nothing here has to be translated and there is no vocabulary to
//! isolate. Whatever implements it is somebody else's problem, and the
//! forge never names them.
//!
//! # A line moves through one door, and it is not this one
//!
//! There is no `record` here. A change point exists because a pursuit
//! was satisfied, and the two are written together or not at all — so
//! the call that moves a line is [`Closings::commit`], where both
//! halves are in hand. A second way to append here would be a way to
//! move a line without ending any work, which is the state the model
//! has no word for.
//!
//! What remains is what a line is when nothing is being closed:
//! opening it, reading it, and moving its own description.
//!
//! # Reading gives back the whole line
//!
//! [`Lines::get`] answers with the history included, because the rules
//! the model holds are about the chain — deciding a close folds it,
//! and recording checks its head. Handing back less would move those
//! rules to whoever answers this call, and there would be as many
//! copies of them as there are implementations.
//!
//! # There is nothing that removes
//!
//! No delete, no truncate, no rewrite of a recorded node — the same
//! absence the model has, for the same reason: everything that ever
//! happened stays reachable, and a path that exists gets called.
//! Renaming a line and changing its strategy are here because they
//! move a line's own description, which is a record the history does
//! not keep.
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
//! [`Name`]: crate::domain::forge::model::value::Name
//! [`Act`]: crate::domain::forge::model::act::Act
//! [`Closings::commit`]: crate::domain::forge::closings::Closings::commit

use async_trait::async_trait;

use crate::domain::forge::model::act::Act;
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::value::{LineId, Name, StrategyId};
// SHARED KERNEL: `DomainError` is a boundary type.
use crate::error::DomainError;

/// Keeps lines.
#[async_trait]
pub trait Lines: Send + Sync {
    /// Records a line that has just been opened, genesis and all.
    async fn open(&self, line: &Line) -> Result<(), DomainError>;

    /// Reads a line back whole, history included.
    async fn get(&self, id: &LineId) -> Result<Option<Line>, DomainError>;

    /// Every line there is.
    ///
    /// Whole lines, histories and all, and that is affordable because
    /// of what a line is: a repository. An instance has the number of
    /// them somebody made on purpose, not one per thing they own.
    ///
    /// Which of them a given person may see is not asked here. A line
    /// carries no owner — grouping and access are outside the forge —
    /// so scoping a listing is the job of whoever knows what a person
    /// is, and this hands over what exists.
    async fn list(&self) -> Result<Vec<Line>, DomainError>;

    /// Records that a line was renamed.
    async fn rename(&self, id: &LineId, name: &Name, act: &Act) -> Result<(), DomainError>;

    /// Records that a line's strategy changed.
    async fn set_strategy(
        &self,
        id: &LineId,
        strategy: &StrategyId,
        act: &Act,
    ) -> Result<(), DomainError>;
}
