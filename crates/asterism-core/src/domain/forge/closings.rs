//! Ending work — the one call that writes to both logs.
//!
//! ```text
//!   Lines      open / get / rename / set_strategy      one log
//!   Pursuits   open / get / push                       one log
//!   Closings   commit                                  both, or neither
//! ```
//!
//! Every other call the forge makes writes to one log. This one writes
//! to two, because ending work as satisfied puts a change point on the
//! line and the two nodes are one act. It exists so that "one act" has
//! somewhere to be true outside the model: [`Closing`] already makes
//! the pair impossible to hold apart in memory, and this makes it
//! impossible to store apart.
//!
//! # What the contract requires
//!
//! **Both, or neither.** Every node in the closing is kept, or none of
//! them is. There is no partial outcome for a caller to detect, and
//! nothing to compensate for afterwards — an ending that half-happened
//! would leave the two logs disagreeing about whether work is over,
//! and no later read could tell which of them was right.
//!
//! **Conditional on the head.** `on` is the node the line was at when
//! the closing was decided. If the line has moved since, the write is
//! refused with [`Conflict`](DomainError::Conflict) and nothing is
//! kept — the decision was made against a line that no longer exists,
//! and writing it anyway would undo whatever arrived in between.
//!
//! That condition is the whole of the concurrency story. Nothing is
//! locked while a caller decides, no order is imposed on who gets to
//! close first, and a caller that loses reads the line again and
//! decides again — where the collision with whoever won becomes
//! visible in the ordinary way.
//!
//! # How it is achieved is not stated here
//!
//! A transaction, an append that takes several streams, one row
//! holding both — the forge does not care and does not ask. What it
//! states is the outcome a caller can rely on, which is the only part
//! it can reason about. Naming a mechanism here would be the storage
//! deciding what the model means.
//!
//! # Why abandoning comes through here too
//!
//! An abandoned closing writes to one log, so it does not need this
//! call. It goes through it anyway, because "which endings need both
//! logs" is a question about the model rather than about the caller —
//! and a second path for the easy case is a path somebody reaches for
//! with the hard one.
//!
//! [`Closing`]: crate::domain::forge::model::closing::Closing

use async_trait::async_trait;

use crate::domain::forge::model::closing::Closing;
use crate::domain::forge::model::value::{ChangePointId, LineId, PursuitId};
// SHARED VOCABULARY: `DomainError` is a boundary type.
use crate::error::DomainError;

/// Keeps what ending work produced.
#[async_trait]
pub trait Closings: Send + Sync {
    /// Keeps every node in `closing`, on the condition that `on` is
    /// still the head of the line.
    ///
    /// All of it is kept or none of it is. Returns
    /// [`Conflict`](DomainError::Conflict) when the line has moved,
    /// having written nothing.
    ///
    /// `pursuit` is named rather than read off the closing, because
    /// only half of them carry it: a change point says which work it
    /// came out of, and a closing that puts nothing on the line has no
    /// change point. The close itself names the node it sits on and
    /// not the log that node belongs to — so without this, an
    /// abandoned ending would have nowhere to be written.
    async fn commit(
        &self,
        line: &LineId,
        pursuit: &PursuitId,
        on: ChangePointId,
        closing: &Closing,
    ) -> Result<(), DomainError>;
}
