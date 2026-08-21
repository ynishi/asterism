//! Keeping work logs, stated in the forge's own words.
//!
//! [`Pursuits`] is the other face the forge asks for, and it is the
//! quiet one: opening work and adding passes to it are what happens
//! most, and neither touches a line.
//!
//! # A pass names the node it sits on
//!
//! [`Pursuits::push`] takes the node the caller believes the work ends
//! at, beside the pass it wants to add. The model refuses a pass that
//! does not sit on the head, but it judges the work log it was
//! *given*, which is the log as it was when it was read. Naming the
//! head makes the write itself conditional, so two passes written at
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
//! # Reading gives back the whole pursuit
//!
//! Adding a pass checks the head, deciding a close folds every pass,
//! and both are rules the model holds about the chain. Handing back
//! less would move them to whoever answers this call.
//!
//! [`Closings::commit`]: crate::domain::forge::closings::Closings::commit

use async_trait::async_trait;

use crate::domain::forge::model::pursuit::{Pursuit, Round};
use crate::domain::forge::model::value::{NodeId, PursuitId};
// SHARED KERNEL: `DomainError` is a boundary type.
use crate::error::DomainError;

/// Keeps work logs.
#[async_trait]
pub trait Pursuits: Send + Sync {
    /// Records work that has just been opened.
    async fn open(&self, pursuit: &Pursuit) -> Result<(), DomainError>;

    /// Reads work back whole, every pass included.
    async fn get(&self, id: &PursuitId) -> Result<Option<Pursuit>, DomainError>;

    /// Adds a pass, on the condition that `on` is still the node the
    /// work ends at.
    ///
    /// Returns [`Conflict`](DomainError::Conflict) when it is not:
    /// somebody else wrote a pass first, and this caller is holding a
    /// log that has moved.
    async fn push(&self, id: &PursuitId, on: NodeId, round: &Round) -> Result<(), DomainError>;
}
