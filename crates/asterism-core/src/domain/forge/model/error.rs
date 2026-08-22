//! What the model refuses.
//!
//! The model has its own error type, and every refusal in this module
//! is one of its variants. Reaching for the shared error instead would
//! mean the forge states a rule in one place and names it in another's
//! vocabulary — and a rule named `Validation` is a rule nobody can
//! match on, so callers end up matching on message text.
//!
//! # It is folded once, at the edge
//!
//! [`DomainError`] is the shared vocabulary, and the conversion below
//! is the only place the model meets it. Adding a refusal means adding
//! a variant here and deciding, once, which shared kind it reads as —
//! not repeating that decision at every call site.
//!
//! # It only grows
//!
//! Every refusal the forge learns is added here. That is what makes
//! the set of ways this model can say no readable in one place, which
//! is the same reason a caller wants a typed error at all.

use thiserror::Error;

use crate::domain::forge::model::change::Collision;
use crate::domain::forge::model::strategy::StrategyError;
use crate::domain::forge::model::value::Name;

// SHARED VOCABULARY: `DomainError` is a boundary type. This module is the
// only one in the model that names it — everything else refuses in the
// forge's own vocabulary and converts at this edge.
use crate::error::DomainError;

/// A refusal the model makes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ForgeError {
    /// A name was blank, or became blank once trimmed.
    #[error("a name must not be blank")]
    BlankName,

    /// A message said nothing.
    #[error("a message must say something")]
    BlankBody,

    /// A remark was hung on an entry the pass never touched.
    #[error("that pass did not touch that entry")]
    NotInThatRound,

    /// A reply, or a correction, named a message of another
    /// conversation.
    #[error("that message belongs to another thread")]
    NotInThatThread,

    /// A strategy was named by a blank string — a line that points at
    /// nothing settles nothing.
    #[error("a strategy must be named")]
    BlankStrategy,

    /// The rule this line settles collisions by wrote nothing, and
    /// said why.
    ///
    /// The work keeps its collision and stays open, which is a state
    /// somebody can act on by hand.
    #[error("the line's strategy did not settle this: {0}")]
    Strategy(#[from] StrategyError),

    /// A row stated no axis at all — nothing is being said about the
    /// entry, so it does not belong in a table.
    #[error("a row states no axis — nothing is being said about the entry")]
    EmptyRow,

    /// A row took an entry off the line while also naming or filling
    /// it, which is a second spelling of a state that already has one.
    #[error(
        "a row takes an entry off the line and moves another axis — a removing row carries only \
         existence, so that a table stays a description of what its change point did"
    )]
    RemovalMovesAnotherAxis,

    /// A table carried no rows — a line advancing to say nothing.
    #[error("a table has at least one row — a line does not move to say nothing")]
    EmptyTable,

    /// A change point named something other than the head as its
    /// parent, which would fork the history.
    #[error(
        "a change point sits on the head — this one names another node as its parent, and a \
         history does not fork"
    )]
    NotOnHead,

    /// The change would leave two entries on the line answering to the
    /// same name.
    #[error("two entries would be on the line under the name {0:?}")]
    NameTaken(Name),

    /// A pass carried no operations.
    #[error("a pass writes something — otherwise nothing happened")]
    EmptyRound,

    /// A conversation had nothing said in it.
    ///
    /// Unreachable from the model's own verbs, which is why it is a
    /// refusal rather than an invariant: `Thread::open` takes the
    /// first message and nothing removes one. It is what a stored
    /// thread with no rows reads back as.
    #[error("a conversation is what was said in it, and nothing was said in this one")]
    EmptyThread,

    /// Work that has ended was written to.
    #[error("this work has ended — pick it up as new work rather than adding to a record")]
    AlreadyClosed,

    /// Work was judged against a line it was not cut from.
    #[error("this work was cut from a node this line does not have")]
    UnknownBase,

    /// The rule offered is not the one this line settles by.
    ///
    /// Answering with it would settle a line by a rule nobody chose
    /// for it, which is the setting quietly not mattering.
    #[error("that is not the rule this line settles collisions by")]
    WrongStrategy,

    /// This line has no such node.
    #[error("this line does not have that change point")]
    UnknownChangePoint,

    /// A rule answered a collision with operations that leave it
    /// standing.
    ///
    /// Checked rather than trusted: a rule is written outside the
    /// model, and one that returns something is not thereby one that
    /// answered. Finding out here costs a fold; finding out later
    /// means a close refusing for a collision somebody was told had
    /// been handled.
    #[error("the line's strategy wrote operations that do not settle what it was asked about")]
    Unsettled,

    /// Work was judged against a line it does not belong to.
    ///
    /// Separate from [`UnknownBase`](Self::UnknownBase), which is a
    /// pursuit of this line holding a node the line does not have.
    /// This one is the pair being wrong before anything is folded.
    #[error("this work is against another line")]
    NotThisLine,

    /// Everything the work would put on the line, the line already
    /// says.
    ///
    /// Usually because somebody else said it first. Nothing is left to
    /// record, and a change point carrying nothing is a line advancing
    /// to say nothing — so the work is closed as abandoned instead,
    /// and what it tried stays readable.
    #[error(
        "everything this work would change, the line already says — close it as abandoned, since \
         a change point carrying nothing is a line moving to say nothing"
    )]
    NothingToRecord,

    /// The line moved axes this work still asks to move, after the
    /// work was cut from it.
    ///
    /// Carries them so a caller can say which, rather than sending
    /// anybody back to recompute what the refusal already knew.
    #[error(
        "{} axes this work asks for moved on the line after the work was cut from it",
        .0.len()
    )]
    Collides(Vec<Collision>),

    /// The line has been archived, and an archived line does not move.
    ///
    /// It is still readable, still holds everything it ever held, and
    /// can be reopened. What it will not do is take a change point —
    /// which is the whole of what archiving means, because a line that
    /// still moved would be a line that was archived in name only.
    #[error("this line is archived; reopen it before putting anything on it")]
    Archived,

    /// Something asked to drop a line that is still open.
    ///
    /// Dropping is reachable only through the archive, as purging is
    /// reachable only through the trash everywhere else in this
    /// codebase. The two steps are what make an irreversible one
    /// deliberate.
    #[error("a line is dropped from the archive; archive it first")]
    NotArchived,

    /// Something asked to drop a line that work is still open against.
    ///
    /// Dropping takes the history that work was cut from, so what is
    /// left is a log against nothing. Ending the work first is not a
    /// formality — it is the record of what happened to it.
    #[error("{0} pieces of work are still open against this line")]
    WorkStillOpen(usize),
}

impl From<ForgeError> for DomainError {
    /// Reads a refusal in the shared vocabulary.
    ///
    /// [`NotOnHead`](ForgeError::NotOnHead),
    /// [`NameTaken`](ForgeError::NameTaken) and
    /// [`Collides`](ForgeError::Collides) are the state fighting
    /// back — the caller was not wrong, the line was somewhere else —
    /// so they read as conflicts. The rest are malformed input.
    ///
    /// [`NothingToRecord`](ForgeError::NothingToRecord) is the one
    /// worth arguing about, and it reads as validation: the line is
    /// not fighting anybody, it is already saying what the caller
    /// asked for. Retrying cannot change that, and the caller's move
    /// is to close the work as abandoned rather than to read again.
    ///
    /// The three standing refusals read the same way and for the same
    /// reason. [`Archived`](ForgeError::Archived) is not a race — the
    /// line is not somewhere else, it is finished with, and reading
    /// again will find it finished with again.
    /// [`NotArchived`](ForgeError::NotArchived) and
    /// [`WorkStillOpen`](ForgeError::WorkStillOpen) are the two steps
    /// of dropping asked out of order. Each names what the caller has
    /// to do next, and none of them is waiting.
    fn from(error: ForgeError) -> Self {
        let message = error.to_string();
        match error {
            ForgeError::NotOnHead
            | ForgeError::NameTaken(_)
            | ForgeError::AlreadyClosed
            | ForgeError::Collides(_) => DomainError::Conflict(message),
            ForgeError::BlankName
            | ForgeError::EmptyRow
            | ForgeError::RemovalMovesAnotherAxis
            | ForgeError::EmptyTable
            | ForgeError::EmptyRound
            | ForgeError::EmptyThread
            | ForgeError::UnknownBase
            | ForgeError::NotThisLine
            | ForgeError::BlankStrategy
            | ForgeError::BlankBody
            | ForgeError::NotInThatRound
            | ForgeError::NotInThatThread
            | ForgeError::Strategy(_)
            | ForgeError::WrongStrategy
            | ForgeError::UnknownChangePoint
            | ForgeError::Unsettled
            | ForgeError::NothingToRecord
            | ForgeError::Archived
            | ForgeError::NotArchived
            | ForgeError::WorkStillOpen(_) => DomainError::Validation(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_that_moved_under_the_caller_reads_as_a_conflict() {
        let shared: DomainError = ForgeError::NotOnHead.into();

        assert!(matches!(shared, DomainError::Conflict(_)));
    }

    #[test]
    fn a_malformed_row_reads_as_validation() {
        let shared: DomainError = ForgeError::EmptyRow.into();

        assert!(matches!(shared, DomainError::Validation(_)));
    }

    #[test]
    fn a_name_already_answered_to_reads_as_a_conflict() {
        let taken = Name::new("key visual").unwrap();
        let shared: DomainError = ForgeError::NameTaken(taken.clone()).into();

        assert!(matches!(shared, DomainError::Conflict(_)), "{shared}");
        assert!(
            shared.to_string().contains(taken.as_str()),
            "the name a caller has to change is in the message: {shared}"
        );
    }

    #[test]
    fn the_message_survives_the_fold() {
        let refusal = ForgeError::BlankName;
        let message = refusal.to_string();
        let shared: DomainError = refusal.into();

        assert!(shared.to_string().contains(&message));
    }
}
