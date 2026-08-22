//! What goes when a line goes, and what that lets go of.
//!
//! ```text
//!   Line (Archived)          Pursuit*  (against it, all ended)
//!     └ holds ──┐              └ holds ──┐
//!               ▼                        ▼
//!            releases(line, work) ──▶ every Content this line and
//!                                     its work stop holding
//! ```
//!
//! # What it does not say
//!
//! **That anything is free afterwards.** This answers for one line and
//! the work against it, which is all it is given and all it could
//! check. Another line naming the same content goes on holding it, and
//! a caller reading this as "safe to delete" would be reading a claim
//! about one holder as a claim about every holder.
//!
//! What catches that is the store: the reference is a foreign key, and
//! deleting content a second line names is refused there. That refusal
//! names a column and no more, which is exactly the shape the persona
//! purge was given a message for — so a caller acting on this set has
//! the same work to do, and this module is not the place it gets done.
//! Answering "is anything else holding it" needs every line, and
//! nothing here has them.
//!
//! # Why this is one answer rather than two
//!
//! A line holds what its chain named; a pursuit holds what its
//! operations named. Dropping a line takes the work against it —
//! a log cut from a history that no longer exists is a record of a
//! proposal against nothing, and its base names a node that is gone.
//! So the set that becomes releasable is the union, and asking for it
//! as a union is the point: a caller adding the two up itself is a
//! caller that can forget the second one, and forgetting it looks
//! exactly like success.
//!
//! # This module reads both logs
//!
//! It is the third that does, after [`change`](super::change) and
//! [`closing`](super::closing), and the reason is different from
//! theirs. Those two answer what work *means* against a line. This one
//! answers what is lost — a question about the two records as records,
//! which is why it takes the work as a slice rather than reaching for
//! it: a line does not keep a list of its pursuits, and one that did
//! would be a second answer to what the pursuits already say.
//!
//! # It does not drop anything
//!
//! There is no delete here, as there is nowhere else in this module —
//! see the model's own note on that. What this returns is what a drop
//! *would* release, together with the refusals that say a drop may not
//! happen at all. Whatever does the deleting asks first.

use std::collections::BTreeSet;

use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::pursuit::Pursuit;
use crate::domain::forge::model::value::Content;

/// What dropping this line would let go of.
///
/// `work` is every pursuit against the line, ended or not — the whole
/// of what would go with it. Passing a subset understates the answer,
/// which is the failure this function exists to make hard rather than
/// impossible: it can check that what it was handed belongs to the
/// line, and it cannot check that nothing was left out.
///
/// # Refusals
///
/// - [`NotArchived`](ForgeError::NotArchived) — the line is still
///   open. Dropping is reachable only through the archive.
/// - [`WorkStillOpen`](ForgeError::WorkStillOpen) — work has not
///   ended. What it was trying is not finished being said.
/// - [`NotThisLine`](ForgeError::NotThisLine) — something in `work` is
///   against another line, and counting its contents as released here
///   would free bytes another line is still holding.
pub fn releases(line: &Line, work: &[Pursuit]) -> Result<BTreeSet<Content>, ForgeError> {
    if work.iter().any(|one| one.of() != line.id()) {
        return Err(ForgeError::NotThisLine);
    }

    let open = work.iter().filter(|one| one.outcome().is_none()).count();
    line.may_drop(open)?;

    let mut held = line.holds();
    for one in work {
        held.extend(one.holds());
    }
    Ok(held)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::forge::model::act::{Act, Actor};
    use crate::domain::forge::model::closing::close;
    use crate::domain::forge::model::op::Op;
    use crate::domain::forge::model::pursuit::{Intent, Outcome, Round};
    use crate::domain::forge::model::value::{ActorId, Name, StrategyId};
    use chrono::{DateTime, TimeZone, Utc};
    use uuid::Uuid;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 22, 11, minute, 0).unwrap()
    }

    fn act(minute: u32) -> Act {
        Act::new(at(minute), Actor::User(ActorId::new()))
    }

    fn name(text: &str) -> Name {
        Name::new(text).unwrap()
    }

    fn content() -> Content {
        Content::from_uuid(Uuid::now_v7())
    }

    fn a_line() -> Line {
        Line::open(
            name(Line::ROOT),
            StrategyId::new("by-hand").unwrap(),
            act(0),
        )
    }

    /// Opens work, writes one round, and ends it the way asked.
    fn work_on(line: &mut Line, ops: Vec<Op>, outcome: Outcome, minute: u32) -> Pursuit {
        let mut work = Pursuit::open(line.id(), None, line.head(), Intent::default(), act(minute));
        work.push(Round::new(work.head(), ops, None, act(minute)).unwrap())
            .unwrap();
        let closing = close(line, &work, outcome, None, act(minute)).unwrap();
        closing.apply(line, &mut work).unwrap();
        work
    }

    #[test]
    fn what_a_drop_releases_is_both_logs_and_not_just_the_line() {
        let mut line = a_line();
        let landed = content();
        let satisfied = work_on(
            &mut line,
            vec![Op::add(landed, name("one"))],
            Outcome::Satisfied,
            1,
        );

        // Work that gave up put nothing on the line, so what it named
        // is in its log and nowhere else — which is exactly the half a
        // caller adding these up itself would miss.
        let tried = content();
        let mut giving_up = Pursuit::open(line.id(), None, line.head(), Intent::default(), act(2));
        giving_up
            .push(
                Round::new(
                    giving_up.head(),
                    vec![Op::add(tried, name("tried"))],
                    None,
                    act(2),
                )
                .unwrap(),
            )
            .unwrap();
        let gave_up = close(&line, &giving_up, Outcome::Abandoned, None, act(3)).unwrap();
        gave_up.apply(&mut line, &mut giving_up).unwrap();

        assert!(line.holds().contains(&landed));
        assert!(
            !line.holds().contains(&tried),
            "the line never heard of what she tried"
        );

        line.archive(act(4));
        let released = releases(&line, &[satisfied, giving_up]).expect("archived, nothing open");

        assert!(released.contains(&landed));
        assert!(
            released.contains(&tried),
            "and the abandoned log's content goes too, or dropping the line \
             leaves bytes nothing can reach and nothing will free"
        );
    }

    #[test]
    fn an_open_line_releases_nothing_because_it_is_not_dropped() {
        let mut line = a_line();
        let work = work_on(
            &mut line,
            vec![Op::add(content(), name("one"))],
            Outcome::Satisfied,
            1,
        );

        let refused = releases(&line, &[work]);
        assert!(
            matches!(refused, Err(ForgeError::NotArchived)),
            "{refused:?}"
        );
    }

    #[test]
    fn work_that_has_not_ended_stops_the_drop_and_says_how_much() {
        let mut line = a_line();
        line.archive(act(1));
        let still_going = Pursuit::open(line.id(), None, line.head(), Intent::default(), act(2));

        let refused = releases(&line, &[still_going]);
        assert!(
            matches!(refused, Err(ForgeError::WorkStillOpen(1))),
            "{refused:?}"
        );
    }

    #[test]
    fn work_against_another_line_is_refused_rather_than_counted() {
        let mut mine = a_line();
        let mut theirs = a_line();
        let elsewhere = work_on(
            &mut theirs,
            vec![Op::add(content(), name("theirs"))],
            Outcome::Satisfied,
            1,
        );
        mine.archive(act(2));

        let refused = releases(&mine, &[elsewhere]);
        assert!(
            matches!(refused, Err(ForgeError::NotThisLine)),
            "counting it would free bytes the other line still holds: {refused:?}"
        );
    }

    /// An entry taken off the line is still held, and so is whatever it
    /// held before — because bringing it back is a verb, and a revival
    /// needs the content to still be there.
    #[test]
    fn a_removed_entry_is_still_released_by_the_drop_and_not_before() {
        let mut line = a_line();
        let held = content();
        let added = Op::add(held, name("one"));
        let entry = added.entry();
        let put_on = work_on(&mut line, vec![added], Outcome::Satisfied, 1);
        let took_off = work_on(&mut line, vec![Op::remove(entry)], Outcome::Satisfied, 2);

        assert!(!line.states()[&entry].alive, "it is off the line");
        assert!(
            line.holds().contains(&held),
            "and still held: undoing the removal is adding that entry back, \
             which needs the content to be there"
        );

        line.archive(act(3));
        let released = releases(&line, &[put_on, took_off]).unwrap();
        assert!(released.contains(&held), "the drop is what frees it");
    }
}
