//! Keeping pursuits, stated in the forge's own words.
//!
//! [`Pursuits`] is the quiet one of the four faces the forge asks for:
//! opening work and adding rounds to it are what happens most, and
//! neither touches a line.
//!
//! # A round names the node it sits on
//!
//! [`Pursuits::push`] takes the node the caller believes the work ends
//! at, beside the round it wants to add. The model refuses a round that
//! does not sit on the head, but it judges the pursuit it was
//! *given*, which is the log as it was when it was read. Naming the
//! head makes the write itself conditional, so two rounds written at
//! once cannot both land and the loser is told.
//!
//! # It cannot end work
//!
//! There is no close here. Ending work as satisfied puts a change
//! point on a line, and the two are written together — that call is
//! [`Closings::commit`]. Abandoning goes the same way rather than
//! getting a shortcut of its own: one door for endings means a reader
//! of this trait cannot find a second one.
//!
//! # Listing hands back whole pursuits, and that is a bet
//!
//! [`Pursuits::of_line`] and [`Pursuits::children`] return every round
//! of every pursuit they answer with, which is more than a caller
//! showing a list needs. There is no lighter shape because the model
//! has no half-pursuit, and inventing one for a listing would put a
//! read's convenience inside the model.
//!
//! The bet is that a line does not accumulate work faster than
//! somebody can read about it. If that turns out false, what arrives
//! is a summary the transport asks for with a measurement behind it —
//! not a guess made here.
//!
//! # Reading gives back the whole pursuit
//!
//! Adding a round checks the head, deciding a close folds every round,
//! and both are rules the model holds about the chain. Handing back
//! less would move them to whoever answers this call.
//!
//! [`Closings::commit`]: crate::domain::forge::closings::Closings::commit

use async_trait::async_trait;

use crate::domain::forge::model::pursuit::{Pursuit, Round};
use crate::domain::forge::model::value::{LineId, NodeId, PursuitId};
// SHARED VOCABULARY: `DomainError` is a boundary type.
use crate::error::DomainError;

/// Keeps pursuits.
#[async_trait]
pub trait Pursuits: Send + Sync {
    /// Records work that has just been opened.
    async fn open(&self, pursuit: &Pursuit) -> Result<(), DomainError>;

    /// Reads work back whole, every round included.
    async fn get(&self, id: &PursuitId) -> Result<Option<Pursuit>, DomainError>;

    /// Every piece of work against a line, open or ended.
    ///
    /// Ended work is included because that is most of what the record
    /// is for: what was tried and abandoned is exactly what a listing
    /// that only showed live work would hide.
    async fn of_line(&self, line: &LineId) -> Result<Vec<Pursuit>, DomainError>;

    /// The work filed under a larger piece of work.
    ///
    /// A pursuit names its parent when it opens and never afterwards,
    /// so this walks one level and cannot loop. Nothing stores the
    /// other direction — which pursuits are under a parent is this
    /// question, and a kept answer would be a second copy of it.
    async fn children(&self, parent: &PursuitId) -> Result<Vec<Pursuit>, DomainError>;

    /// Adds a round, on the condition that `on` is still the node the
    /// work ends at.
    ///
    /// Returns [`Conflict`](DomainError::Conflict) when it is not:
    /// somebody else wrote a round first, and this caller is holding a
    /// log that has moved.
    async fn push(&self, id: &PursuitId, on: NodeId, round: &Round) -> Result<(), DomainError>;
}
