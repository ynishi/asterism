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

    /// A remark was hung on an entry the round never touched.
    #[error("that round did not touch that entry")]
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

    /// A round carried no operations.
    #[error("a round writes something — otherwise nothing happened")]
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
        "{} axes this work asks for moved on the line after the work was cut from it — resolve \
         it, by the line's rule or by hand, and the same close then lands",
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
    ///
    /// The count is parenthesised rather than made the subject of the
    /// sentence, because a message reading "1 pieces of work" was what
    /// a screen showed the first time this refusal reached one.
    #[error(
        "work is still open against this line ({0} unclosed); close it first, and the \
         same drop then works"
    )]
    WorkStillOpen(usize),

    /// The store handed back a record the model would not have
    /// written, and [`restore`](crate::domain::forge::model::restore)
    /// says which rule it breaks.
    ///
    /// **The test is whether the refusal describes a row the store
    /// handed back**, not whether the rule it breaks came from the
    /// write path. Both kinds are here: `restore::line` and
    /// `restore::pursuit` replay `History::record`, `Pursuit::push`
    /// and `Pursuit::end`, which are the write path's own; `chain` and
    /// the empty-thread guard are rules `restore` states itself and
    /// nothing else has. All of them answer the same question — could
    /// this have been written — so all of them are marked.
    ///
    /// Nothing a caller did causes this. It asked to read, and there
    /// is no state for it to be in conflict with — so it reads as
    /// infrastructure, and the request that finds it fails with a
    /// `500` rather than being told to try again. The inner refusal
    /// travels along because it says exactly which invariant the row
    /// broke, which is the first thing anybody looking at the row
    /// wants to know.
    ///
    /// Marked at the boundary rather than by splitting the variants
    /// underneath. The two refusals noticed first were `NotOnHead` and
    /// `NameTaken`, both of which the write path raises too; splitting
    /// them would have left `AlreadyClosed`, which arrives the same
    /// way, and every rule these functions learn later. A boundary
    /// covers the ones nobody has written yet.
    ///
    /// The rows that never reach `restore` — because they cannot be
    /// assembled into the types it takes — are the adapter's to answer
    /// for, and `forge::rows` answers them the same way.
    #[error("this record could not have been written: {0}")]
    Unwritable(Box<ForgeError>),
}

impl ForgeError {
    /// Marks a refusal as the store's rather than the caller's.
    ///
    /// Used by [`restore`](crate::domain::forge::model::restore) around
    /// every replay of a write-path rule. Already-marked refusals pass
    /// through unchanged, so a nested restore does not report a record
    /// that could not have been written that could not have been
    /// written.
    pub fn unwritable(self) -> Self {
        match self {
            already @ Self::Unwritable(_) => already,
            refusal => Self::Unwritable(Box::new(refusal)),
        }
    }
}

impl From<ForgeError> for DomainError {
    /// Reads a refusal in the shared vocabulary.
    ///
    /// [`NotOnHead`](ForgeError::NotOnHead),
    /// [`NameTaken`](ForgeError::NameTaken),
    /// [`AlreadyClosed`](ForgeError::AlreadyClosed) and
    /// [`Collides`](ForgeError::Collides) are the state fighting
    /// back — the caller was not wrong, the line was somewhere else —
    /// so they read as conflicts, and so do the three standing
    /// refusals below. What is left is malformed input, plus
    /// [`Unwritable`](ForgeError::Unwritable), which is neither.
    ///
    /// **Which conflict is carried across, not flattened.** The four
    /// want four different things from a caller and the difference is
    /// only knowable here, where the refusal is raised: `NotOnHead` is
    /// a landing that arrived mid-write and the same close may win
    /// next time; `Collides` needs the work resolved first and then
    /// the same close works; `AlreadyClosed` is over; `NameTaken`
    /// needs a different name. A single `Conflict(String)` made all
    /// four one thing, and a client reading it could only retry
    /// everything — which loops on two of them — or nothing, which
    /// gives up on the one race it would win.
    ///
    /// [`NothingToRecord`](ForgeError::NothingToRecord) is the one
    /// worth arguing about, and it reads as validation: the line is
    /// not fighting anybody, it is already saying what the caller
    /// asked for. Retrying cannot change that, and the caller's move
    /// is to close the work as abandoned rather than to read again.
    ///
    /// **The three standing refusals are conflicts, and they say so
    /// themselves.** [`Archived`](ForgeError::Archived) is "reopen it
    /// before putting anything on it";
    /// [`NotArchived`](ForgeError::NotArchived) is "archive it first";
    /// [`WorkStillOpen`](ForgeError::WorkStillOpen) is work to end
    /// before a line can be dropped. Each names a state change after
    /// which the identical request goes through, which is
    /// [`Blocked`](crate::error::ConflictKind::Blocked) exactly.
    ///
    /// Note what `Blocked` does *not* require: that the refusal clears
    /// on its own. Nothing here is waiting, and none of these three
    /// becomes true by being asked again — somebody archives the line,
    /// reopens it, or ends the work, and then the same request goes
    /// through. That is the test, and "is it waiting" is not.
    fn from(error: ForgeError) -> Self {
        let message = error.to_string();
        match error {
            // Not the caller's doing, and not a conflict with
            // anything: it asked to read, and what came back could not
            // have been written. Nothing it does differently helps,
            // which is why this is the one refusal here that is
            // neither a conflict nor a validation.
            ForgeError::Unwritable(_) => DomainError::Infra(anyhow::anyhow!(message)),
            // A landing arrived while this write was deciding. Read
            // again and the same close may well go through.
            ForgeError::NotOnHead => DomainError::raced(message),
            // The line moved under this work. Resolving it is the
            // thing to do first, and then this same close works.
            ForgeError::Collides(_) => DomainError::blocked(message),
            // Over. Asking again finds it over again.
            ForgeError::AlreadyClosed => DomainError::settled(message),
            // Two entries cannot answer to one name. Another name can.
            ForgeError::NameTaken(_) => DomainError::clashes(message),
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
            | ForgeError::NothingToRecord => DomainError::Validation(message),
            // The three standing refusals: each names a state change
            // after which the same request works.
            ForgeError::Archived | ForgeError::NotArchived | ForgeError::WorkStillOpen(_) => {
                DomainError::blocked(message)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Named here rather than beside `DomainError` above, because the
    // forge's production code never names it: the four constructors
    // on `DomainError` say which kind, and only a test asserting on
    // one has to spell the type. `forge_boundary.rs` reads that
    // difference — a `use` inside `#[cfg(test)]` is not what the
    // forge depends on — which is why this does not want a line on
    // the shared vocabulary.
    use crate::error::ConflictKind;

    /// A line that moved under the caller is a race, and the kind
    /// says so.
    ///
    /// The variant alone is not the assertion worth making: all four
    /// of the forge's conflicts are `Conflict`, and what a caller does
    /// about this one — read again and send the same close — is true
    /// of only this one.
    #[test]
    fn a_line_that_moved_under_the_caller_reads_as_a_race_worth_retrying() {
        let shared: DomainError = ForgeError::NotOnHead.into();

        assert!(
            matches!(
                shared,
                DomainError::Conflict {
                    kind: ConflictKind::Raced,
                    ..
                }
            ),
            "{shared}"
        );
        // And the token a caller actually reads says the same thing.
        // The kind is internal; `reason` is the promise, so asserting
        // only the kind would leave the wire free to disagree with it.
        assert_eq!(shared.reason(), Some("raced"));
    }

    /// Work that has already ended is over, and the kind says so.
    ///
    /// The pair to the test above, and the reason the kind exists:
    /// these two are one variant and opposite advice. A client that
    /// retried both would loop here forever.
    #[test]
    fn work_that_has_already_ended_reads_as_settled_and_is_not_worth_retrying() {
        let shared: DomainError = ForgeError::AlreadyClosed.into();

        assert!(
            matches!(
                shared,
                DomainError::Conflict {
                    kind: ConflictKind::Settled,
                    ..
                }
            ),
            "{shared}"
        );
        assert_eq!(shared.reason(), Some("settled"));
    }

    /// A record the store should not have been holding is not the
    /// caller's conflict with anything.
    ///
    /// This is the case the kinds made dangerous. `NotOnHead` is a
    /// race on the write path and a forked chain on the read path, and
    /// one variant cannot be both. It went unnoticed because the
    /// SQLite adapter flattens every restore refusal into `Infra`
    /// before a caller sees it, so the model's answer was being
    /// discarded rather than read — right by accident, and over one
    /// store at a time. Saying it here is what makes it true of every
    /// store there is, and of any store written later.
    ///
    /// A read has no state to conflict with; the row simply could not
    /// have been written, so it is not a conflict at any kind.
    #[test]
    fn a_record_that_could_not_have_been_written_is_not_a_conflict() {
        let shared: DomainError = ForgeError::NotOnHead.unwritable().into();

        assert!(
            matches!(shared, DomainError::Infra(_)),
            "the store handed back something impossible, and no retry \
             advice fits that: {shared}"
        );
        assert!(
            shared.to_string().contains("could not have been written"),
            "and it says so: {shared}"
        );

        // The same variant, unwrapped, is still the write path's race.
        // One is the caller's to act on and one is not, which is the
        // whole distinction this carries.
        let raced: DomainError = ForgeError::NotOnHead.into();
        assert!(matches!(
            raced,
            DomainError::Conflict {
                kind: ConflictKind::Raced,
                ..
            }
        ));
    }

    /// Marking twice does not nest.
    #[test]
    fn a_refusal_already_the_stores_fault_is_not_wrapped_again() {
        let once = ForgeError::AlreadyClosed.unwritable();
        let twice = once.clone().unwritable();

        assert_eq!(once, twice);
        assert_eq!(
            twice
                .to_string()
                .matches("could not have been written")
                .count(),
            1,
            "{twice}"
        );
    }

    /// A collision is not a race: the same close works, but only
    /// after the work is resolved.
    #[test]
    fn a_collision_reads_as_blocked_rather_than_raced() {
        let shared: DomainError = ForgeError::Collides(Vec::new()).into();

        assert!(
            matches!(
                shared,
                DomainError::Conflict {
                    kind: ConflictKind::Blocked,
                    ..
                }
            ),
            "{shared}"
        );
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

        assert!(
            matches!(
                shared,
                DomainError::Conflict {
                    kind: ConflictKind::Clashes,
                    ..
                }
            ),
            "{shared}"
        );
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

    /// The three standing refusals, and what each says to do.
    ///
    /// Each names a state change after which the identical request
    /// goes through, which is what `Blocked` means — and `Blocked`
    /// promises the message names that change, so the promise is
    /// checked here rather than trusted.
    #[test]
    fn the_three_standing_refusals_are_blocked_and_name_the_way_through() {
        for (refusal, remedy) in [
            (ForgeError::Archived, "reopen it"),
            (ForgeError::NotArchived, "archive it first"),
            (ForgeError::WorkStillOpen(2), "close them first"),
        ] {
            let shared: DomainError = refusal.into();
            assert!(
                matches!(
                    shared,
                    DomainError::Conflict {
                        kind: ConflictKind::Blocked,
                        ..
                    }
                ),
                "each names a state change after which the same request \
                 works: {shared}"
            );
            assert_eq!(shared.reason(), Some("blocked"));
            assert!(
                shared.to_string().contains(remedy),
                "and the message says which change: {shared}"
            );
        }
    }
}
