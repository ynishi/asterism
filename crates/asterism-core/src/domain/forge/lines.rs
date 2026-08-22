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
//! # Nothing removes part of a line, and one call removes all of it
//!
//! No truncate, no rewrite of a recorded node, no delete of anything
//! inside a history — the same absence the model has, for the same
//! reason: everything that ever happened stays reachable, and a path
//! that exists gets called. Renaming a line, changing its strategy and
//! archiving it are here because they move a line's own description,
//! which is a record the history does not keep.
//!
//! [`Lines::discard`] is the exception, and it is whole-line or
//! nothing. A line that holds bytes somebody else may want back has to
//! be lettable-go of somehow, and the model's answer is that the way
//! out is the whole way out: archive it, end the work against it, and
//! then it goes with everything under it. Half of it going is the
//! state that would leave a history unreadable, which is what every
//! other absence here is protecting.
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
use crate::domain::forge::model::line::{Line, Standing};
use crate::domain::forge::model::value::{LineId, Name, PursuitId, StrategyId};
// SHARED VOCABULARY: `DomainError` is a boundary type.
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

    /// Records that a line was finished with, or taken back out of the
    /// archive.
    ///
    /// One call rather than two, because the two are one field and a
    /// store that offered them separately would be offering a way to
    /// be in neither state. Idempotent, as the model's are: archiving
    /// an archived line moves nothing but the stamp, which is a record
    /// of somebody saying so again.
    async fn set_standing(
        &self,
        id: &LineId,
        standing: Standing,
        act: &Act,
    ) -> Result<(), DomainError>;

    /// Takes the line and everything under it, on two conditions asked
    /// here: that the line is still archived, and that the work
    /// against it is exactly `covering`.
    ///
    /// # What goes
    ///
    /// The line, its history, its change rows, every pursuit named
    /// here with its nodes and their operations, and every
    /// conversation hanging off any of them — a thread anchored to a
    /// pursuit, a round, an entry as a round had it, or a change point
    /// on this line goes with what it is about, messages and
    /// corrections included. Nothing else in this codebase deletes a
    /// thread ([`Threads`](super::threads::Threads) has no verb for
    /// it); a drop is where a conversation ends, because a remark
    /// about a thing that no longer exists is a remark no read can
    /// make sense of.
    ///
    /// All of it or none of it: a line whose history went while its
    /// work stayed is work whose base names a node that is gone, which
    /// no read can turn back into a value.
    ///
    /// # Why the conditions are asked here rather than trusted
    ///
    /// Because what a caller was told a drop would release was
    /// computed from a line and a list of pursuits it read first, and
    /// either can have moved by the time the write runs. A list that
    /// has grown makes the answer wrong quietly, in the direction that
    /// leaves bytes held by nothing; a line taken back out of the
    /// archive makes it a drop of a line somebody is using again.
    /// Both are asked inside the write, where they cannot go stale,
    /// and both come back as [`Conflict`](DomainError::Conflict)
    /// rather than as a silent understatement.
    ///
    /// A drop has a third condition, and it is the one that stays
    /// where the decision was made: every pursuit here has ended
    /// ([`WorkStillOpen`](crate::domain::forge::model::error::ForgeError::WorkStillOpen)).
    /// It does not need asking again, because work that has ended
    /// cannot start — an ending is a node and nothing takes one back —
    /// so the only way that answer can change under the write is work
    /// *opened* since, which is exactly what `covering` catches. The
    /// two conditions here are the two that can move.
    ///
    /// # What a store may refuse and how
    ///
    /// A name in `covering` that is not against this line is not a
    /// race and is not to be reported as one: nothing removes a
    /// pursuit but a drop of its line, and that line is the one being
    /// dropped. It is a caller naming somebody else's work, which the
    /// model refuses as
    /// [`NotThisLine`](crate::domain::forge::model::error::ForgeError::NotThisLine),
    /// so a store meeting it answers
    /// [`Validation`](DomainError::Validation) rather than `Conflict`.
    ///
    /// All of this is the same shape as ending work: the store does
    /// not re-derive what the model decided, it refuses to write when
    /// what was decided no longer describes what is there.
    async fn discard(&self, id: &LineId, covering: &[PursuitId]) -> Result<(), DomainError>;
}
