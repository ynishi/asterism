//! Letting a line's rule answer a collision.
//!
//! ```text
//!   collisions(L, W)
//!        ├─ none ─────────────────────────────────► nothing to do
//!        └─ some ──► Strategy::resolve ──► ops ──► a pass, written by the server
//!                          │                 │
//!                          │                 └─ checked: do these actually settle it?
//!                          └─ none ──────────────► the collision stands, for a person
//! ```
//!
//! **A rule does nothing a person could not have done.** It writes the
//! operations somebody resolving by hand would have written, in the
//! same four verbs, into an ordinary pass. There is no vocabulary for
//! resolving, no transformation applied on the way onto a line, and no
//! record of resolution separate from the operations themselves. What
//! it does is save somebody the typing.
//!
//! That is the whole of what automatic resolution is here, and it is
//! why the complexity lives in the rule rather than in the model: the
//! sequences differ — fork the entry and move your value onto it, keep
//! yours and move the line's, put both on new entries and take the old
//! one off, record what you tried and then drop it — and every one of
//! them is expressible already.
//!
//! # What comes back is checked
//!
//! A rule is written outside the model, so it can return operations
//! that do not settle what it was asked about. Folding them in and
//! looking is cheap, and the alternative is finding out at the far end
//! of the work, when a close refuses for a collision somebody was told
//! had been handled.
//!
//! # A rule that writes nothing is not a failure
//!
//! Some lines are meant to be sorted out by hand. Their rule returns
//! nothing, no pass is written, and the collision stays exactly where
//! anybody can see it — which is the state a person then acts on.
//!
//! # Nothing here touches the line
//!
//! The pass goes on the pursuit. A line moves when work ends, and
//! that is somewhere else.

use crate::domain::forge::model::act::{Act, Actor};
use crate::domain::forge::model::change::{collisions, normalise, states_at, write_set};
use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::op::{Op, fold};
use crate::domain::forge::model::pursuit::{Pursuit, Round};
use crate::domain::forge::model::strategy::{Divergence, Strategy};
use crate::domain::forge::model::value::ActorId;

/// Runs the line's rule over whatever this work collides with.
///
/// `server` is who the pass is written as. A rule acts under a setting
/// somebody chose rather than on its own behalf, and the node it
/// writes has to say which server ran it.
///
/// # Refusals
///
/// - [`NotThisLine`](ForgeError::NotThisLine) — the work is against
///   another line.
/// - [`AlreadyClosed`](ForgeError::AlreadyClosed) — work that has
///   ended is not resolving anything.
/// - [`WrongStrategy`](ForgeError::WrongStrategy) — the rule offered
///   is not the one this line settles by. Answering with it would
///   settle a line by a rule nobody chose for it.
/// - [`Strategy`](ForgeError::Strategy) — the rule would not decide.
/// - [`Unsettled`](ForgeError::Unsettled) — the rule wrote operations
///   that leave the collisions it was asked about standing.
pub fn react(
    line: &Line,
    work: &Pursuit,
    rule: &dyn Strategy,
    server: ActorId,
    act: Act,
) -> Result<Option<Round>, ForgeError> {
    if work.of() != line.id() {
        return Err(ForgeError::NotThisLine);
    }
    if work.outcome().is_some() {
        return Err(ForgeError::AlreadyClosed);
    }
    if rule.id() != *line.strategy() {
        return Err(ForgeError::WrongStrategy);
    }

    let found = collisions(line, work)?;
    if found.is_empty() {
        return Ok(None);
    }

    let request = work.request();
    let taken = line.states();
    let was = states_at(line.history(), work.base())?;

    let ops = rule.resolve(&Divergence::new(&found, &request, &taken, &was))?;
    if ops.is_empty() {
        // The line is sorted out by hand. Nothing is written, and the
        // collision is left where somebody can see it.
        return Ok(None);
    }

    settles(line, work, &ops, &found)?;

    Ok(Some(Round::new(
        work.head(),
        ops,
        None,
        Act::new(act.at(), Actor::System(server)),
    )?))
}

/// Refuses operations that leave the collisions they answer standing.
///
/// Folds them into what the work is already asking for and asks the same
/// question again. A rule cannot be trusted to have answered simply
/// because it returned something.
fn settles(
    line: &Line,
    work: &Pursuit,
    ops: &[Op],
    found: &[crate::domain::forge::model::change::Collision],
) -> Result<(), ForgeError> {
    let mut every: Vec<Op> = work
        .rounds()
        .iter()
        .flat_map(|round| round.ops().iter().cloned())
        .collect();
    every.extend(ops.iter().cloned());

    let after = write_set(&normalise(fold(&every), &line.states()));

    if found
        .iter()
        .any(|collision| after.contains(&(collision.entry, collision.axis)))
    {
        return Err(ForgeError::Unsettled);
    }
    Ok(())
}
