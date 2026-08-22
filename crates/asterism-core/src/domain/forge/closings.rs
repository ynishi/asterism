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
//! **On the parent nothing has taken.** A closing names the node it
//! sits on, and two nodes on one parent is a fork. The write refuses
//! one, which is what "the line moved since this was decided" looks
//! like from underneath — somebody else's change point is already
//! where this one would go.
//!
//! **Decided again, once, by whoever is holding the write.** A caller
//! that loses that race does not hear about it. The store asks
//! [`Deciding`] for an ending against the two logs as the write finds
//! them, and that attempt is final: the caller decided outside the
//! write, where the line could still move, and this one is decided
//! inside it, where it cannot.
//!
//! That is the whole of the concurrency story. Nothing is locked while
//! a caller decides, no order is imposed on who gets to close first,
//! and losing costs one re-decision rather than a round trip — where
//! the collision with whoever won becomes visible in the ordinary way,
//! because deciding again is deciding against the line that won.
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

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::forge::model::closing::Closing;
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::pursuit::Pursuit;
use crate::domain::forge::model::value::{LineId, PursuitId};
// SHARED VOCABULARY: `DomainError` is a boundary type.
use crate::error::DomainError;

/// Ending work, against the two logs as they are handed over.
///
/// The same decision the caller already made, in a form the write can
/// ask for a second time. It is asked at most once, by a store that
/// has been refused, and what it is handed then is what that store can
/// see under its own lock.
///
/// # Which refusals reach it
///
/// Two, and they are the two ways a log moves under a decision made
/// outside the write:
///
/// - **A change point already sits where this one would go.** Somebody
///   else's work landed on the line first.
/// - **A node already sits where this ending would go.** A round was
///   written to the work while its close was being decided.
///
/// Nothing else. A second ending, a node neither log ever had, or a
/// refusal the model gives when asked again are **final**: the answer
/// does not change for being asked twice, and a store that asked again
/// would be asking a question it already has the answer to. That
/// division is the store's to implement and this is where it is
/// stated, so that two stores cannot each hold half of it.
///
/// # Why the store cannot decide this itself
///
/// Because deciding is the model's, and a store that could end work
/// would be a store that knows what collides. What it holds instead is
/// the one thing it has and the caller has not: a line that cannot
/// move while it is looking at it.
///
/// # Why a caller cannot be handed it back
///
/// Handing back "the line moved, decide again" is handing back a
/// decision that can go stale on the way. The caller re-reads, decides,
/// writes, and loses again — bounded by nothing except how busy the
/// line is, which is why what it replaced was a loop with a number in
/// it. Inside the write there is no second race to lose.
pub trait Deciding: Send + Sync {
    /// Ends the work again, against this line and this pursuit.
    ///
    /// Every refusal the model can give is a refusal here, and it is
    /// the answer rather than a failure of the write: a line that
    /// moved may now collide with what this work asks for, or already
    /// say it, and that is what deciding against the line as it is
    /// means.
    fn close(&self, line: &Line, pursuit: &Pursuit) -> Result<Closing, DomainError>;
}

/// Keeps what ending work produced.
#[async_trait]
pub trait Closings: Send + Sync {
    /// Keeps every node in `closing`, on the condition that nothing
    /// has taken the parent it names.
    ///
    /// All of it is kept or none of it is. When a parent is taken —
    /// the two cases [`Deciding`] names — `again` is asked for an
    /// ending against the logs as this write finds them, and that one
    /// is kept instead.
    ///
    /// Every other refusal comes back as it is, without `again` being
    /// asked: work that has already ended, a node neither log had, and
    /// whatever the second decision itself refuses. All of them arrive
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
        closing: &Closing,
        again: Arc<dyn Deciding>,
    ) -> Result<(), DomainError>;
}
