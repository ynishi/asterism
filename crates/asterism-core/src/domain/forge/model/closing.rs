//! Ending work — the one act that moves both logs.
//!
//! ```text
//!   close(&Line, &Pursuit, outcome, act)
//!            │
//!            ├── Abandoned ────────────────► Closing::Abandoned { close }
//!            │
//!            └── Satisfied ── normalise ──► write set ──► collisions
//!                                 │             │             │
//!                             (nothing        (empty)      (any)
//!                              refuses)          │             │
//!                                 │           refuse        refuse
//!                                 ▼
//!                        Closing::Landed { close, point }
//! ```
//!
//! Everywhere else, a decision moves one log. Opening, passing and
//! taking something in write only to the work log; renaming a line
//! writes only to its own description. This is the exception, and it
//! is the only one: ending work as satisfied puts a change point on
//! the line, and the two are one act rather than two that happen to
//! run together.
//!
//! # Both, or neither
//!
//! [`Closing`] holds the close and the change point in one value with
//! private fields. There is no order between them, no window where one
//! is written and the other is not, and no way to hold either alone —
//! `Close::new` and `ChangePoint::new` are both closed to the model,
//! and outside tests this is the only function that calls them.
//!
//! When [`close`] refuses, neither node exists. Nothing was minted, so
//! there is nothing to undo and nothing to compensate for later. That
//! is what "one act" has to mean in a type: not that two writes are
//! coordinated, but that there is one thing that either happened or
//! did not.
//!
//! # Deciding is not applying, and neither is storing
//!
//! [`close`] returns what would be born and touches nothing. Putting
//! it on the two logs in memory is [`Closing::apply`]. Keeping it is
//! somebody else's problem, and the port that does it takes this one
//! value, so there is no second call for a caller to forget.
//!
//! Splitting it this way is what makes the decision testable against
//! any pair of a line and a pursuit, without a store and without a
//! clock: the same inputs give the same answer, and the answer is a
//! value rather than a mutation somebody has to go looking for.
//!
//! # This function does not settle anything
//!
//! A collision is refused here, never resolved. Turning one into a
//! divergence writes a pass into the work log, under the line's
//! strategy, and it happens while the work is open — by the time
//! anybody is closing, the question has been settled or it has not.
//!
//! Closing is where that is checked and nowhere else, which is what
//! keeps the check from being a flag somebody has to clear: collisions
//! are computed from the two logs whenever they are asked, so what is
//! refused here is the state the logs are actually in.

use crate::domain::forge::model::act::Act;
use crate::domain::forge::model::change::{collisions, normalise, write_set};
use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::history::ChangePoint;
use crate::domain::forge::model::line::{Line, Standing};
use crate::domain::forge::model::pursuit::{Close, Outcome, Pursuit};
use crate::domain::forge::model::table::Table;

/// What ending work produced.
///
/// A close, and the change point born with it when the work put
/// something on the line. One value rather than two, so that a reader
/// never has to hold "satisfied" and "has a change point" as separate
/// facts that ought to agree.
///
/// **The fields are private, and that is the point.** An enum with
/// public variant fields could be taken apart by pattern — bind the
/// close, drop the change point, end the work — which would leave the
/// pursuit satisfied and the line where it was, exactly the state this
/// type exists to make unreachable. The whole surface is
/// [`close`](Self::close), [`point`](Self::point) and
/// [`apply`](Self::apply), and none of them hands out an owned half.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closing {
    close: Close,
    point: Option<ChangePoint>,
}

impl Closing {
    /// The node the work ends at, whichever ending it was.
    pub fn close(&self) -> &Close {
        &self.close
    }

    /// What the line moves to, if it moves.
    pub fn point(&self) -> Option<&ChangePoint> {
        self.point.as_ref()
    }

    /// Whether anything is going on the line.
    pub fn lands(&self) -> bool {
        self.point.is_some()
    }

    /// Puts what was born on the two logs.
    ///
    /// The line first, because recording is the half that can still
    /// refuse — the head may have moved, or the table may hand one
    /// name to two live entries. If it does refuse, the work log is
    /// untouched and the pursuit is still open, which is the state a
    /// caller can retry from.
    ///
    /// Nothing is decided here. Every refusal this can return was
    /// possible before [`close`] ran and is the line answering, not a
    /// second opinion about the same question.
    pub fn apply(self, line: &mut Line, pursuit: &mut Pursuit) -> Result<(), ForgeError> {
        if let Some(point) = self.point {
            line.record(point)?;
        }
        pursuit.end(self.close)
    }
}

/// Ends work, and says what that puts on the line.
///
/// Reads both logs and writes to neither. `outcome` is what the person
/// closing says they are doing; whether the model agrees is what this
/// answers.
///
/// # Refusals
///
/// - [`NotThisLine`](ForgeError::NotThisLine) — the pursuit is against
///   another line. Answering anything else would be judging work
///   against a history that has nothing to do with it.
/// - [`Archived`](ForgeError::Archived) — the line is finished with,
///   and a satisfied close is the one thing that moves it. Refused
///   before anything is folded, because the answer does not depend on
///   what the work asked for.
/// - [`AlreadyClosed`](ForgeError::AlreadyClosed) — work ends once.
/// - [`UnknownBase`](ForgeError::UnknownBase) — the node the work was
///   cut from is not in this history.
/// - [`NothingToRecord`](ForgeError::NothingToRecord) — everything the
///   work would change, the line already says. Close it as abandoned.
/// - [`Collides`](ForgeError::Collides) — the line moved axes this
///   work still asks to move, after the work was cut from it.
///
/// An abandoned close skips all but the first two: work that is giving
/// up does not have to be reconcilable with anything.
pub fn close(
    line: &Line,
    pursuit: &Pursuit,
    outcome: Outcome,
    note: Option<String>,
    act: Act,
) -> Result<Closing, ForgeError> {
    if pursuit.of() != line.id() {
        return Err(ForgeError::NotThisLine);
    }
    if pursuit.outcome().is_some() {
        return Err(ForgeError::AlreadyClosed);
    }

    let at = pursuit.log().head();

    if outcome == Outcome::Abandoned {
        return Ok(Closing {
            close: Close::new(at, Outcome::Abandoned, note, act),
            point: None,
        });
    }

    // Asked before the fold, because it does not depend on it: an
    // archived line takes no change point, and a satisfied close is a
    // change point. Work against one that is finished with can still
    // give up — that is the branch above, and it is the reason this
    // sits after it rather than beside `NotThisLine`.
    if line.standing() == Standing::Archived {
        return Err(ForgeError::Archived);
    }

    // What is left after the line's own answer is subtracted. An axis
    // the head already has this value on is not a change, so it is not
    // this work's to write and not this work's to collide over.
    let rows = normalise(pursuit.request(), &line.states());
    let writes = write_set(&rows);
    if writes.is_empty() {
        return Err(ForgeError::NothingToRecord);
    }

    let found = collisions(line, pursuit)?;
    if !found.is_empty() {
        return Err(ForgeError::Collides(found));
    }

    // Minted in this order because the change point names the close.
    // The other direction would need an id that does not exist yet,
    // which is the shape that makes one of the two optional.
    let close = Close::new(at, Outcome::Satisfied, note, act);
    let point = ChangePoint::new(line.head(), pursuit.id(), close.id(), Table::of(rows)?, act);

    Ok(Closing {
        close,
        point: Some(point),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::forge::model::act::Actor;
    use crate::domain::forge::model::value::ActorId;
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    use crate::domain::forge::model::change::Axis;
    use crate::domain::forge::model::op::Op;
    use crate::domain::forge::model::pursuit::{Intent, Round};
    use crate::domain::forge::model::value::{ChangePointId, Content, EntryId, Name, StrategyId};

    fn act(minute: u32) -> Act {
        Act::new(
            Utc.with_ymd_and_hms(2026, 8, 20, 12, minute, 0).unwrap(),
            Actor::User(ActorId::new()),
        )
    }

    fn name(text: &str) -> Name {
        Name::new(text).unwrap()
    }

    fn content() -> Content {
        Content::from_uuid(Uuid::now_v7())
    }

    fn line() -> Line {
        Line::open(name(Line::ROOT), strategy(), act(0))
    }

    fn strategy() -> StrategyId {
        StrategyId::new("mainline-first").unwrap()
    }

    fn work_on(line: &Line) -> Pursuit {
        Pursuit::open(line.id(), None, line.head(), Intent::default(), act(1))
    }

    /// Adds a pass carrying `ops`, and hands back the pursuit.
    fn passing(mut pursuit: Pursuit, ops: Vec<Op>, minute: u32) -> Pursuit {
        let round = Round::new(pursuit.log().head(), ops, None, act(minute)).unwrap();
        pursuit.push(round).unwrap();
        pursuit
    }

    /// Lands `ops` on the line through a pursuit of its own, which is
    /// how the line moves under work that is already open.
    fn landed(line: &mut Line, ops: Vec<Op>, minute: u32) -> ChangePointId {
        let pursuit = passing(work_on(line), ops, minute);
        let closing = close(line, &pursuit, Outcome::Satisfied, None, act(minute)).unwrap();
        let moved = closing.point().unwrap().id();
        let mut pursuit = pursuit;
        closing.apply(line, &mut pursuit).unwrap();
        moved
    }

    #[test]
    fn a_satisfied_close_produces_the_change_point_with_it() {
        let line = line();
        let pursuit = passing(work_on(&line), vec![Op::add(content(), name("cut-01"))], 2);

        let closing = close(&line, &pursuit, Outcome::Satisfied, None, act(3)).unwrap();

        assert!(
            closing.lands(),
            "a satisfied close puts something on the line"
        );
        let (close, point) = (closing.close(), closing.point().unwrap());
        assert_eq!(close.outcome(), Outcome::Satisfied);
        assert_eq!(point.parent(), line.head());
        assert_eq!(point.from(), pursuit.id());
        // The two halves name each other, which is what makes
        // "these were one act" readable from either side.
        assert_eq!(point.by(), close.id());
    }

    #[test]
    fn an_abandoned_close_puts_nothing_on_the_line() {
        let line = line();
        let pursuit = passing(work_on(&line), vec![Op::add(content(), name("cut-01"))], 2);

        let closing = close(&line, &pursuit, Outcome::Abandoned, None, act(3)).unwrap();

        assert!(!closing.lands());
        assert_eq!(closing.point(), None);
        assert_eq!(closing.close().outcome(), Outcome::Abandoned);
    }

    /// Abandoning is not conditional on anything the line says. Work
    /// that is giving up is exactly the work that cannot satisfy the
    /// checks, and requiring them would leave it open forever.
    #[test]
    fn work_that_says_nothing_new_can_still_be_abandoned() {
        let mut line = line();
        let held = content();
        let pursuit = passing(work_on(&line), vec![Op::add(held, name("cut-01"))], 2);
        landed(&mut line, vec![Op::add(held, name("cut-02"))], 3);

        assert!(close(&line, &pursuit, Outcome::Abandoned, None, act(4)).is_ok());
    }

    #[test]
    fn applying_puts_both_nodes_on_their_logs() {
        let mut line = line();
        let mut pursuit = passing(work_on(&line), vec![Op::add(content(), name("cut-01"))], 2);
        let closing = close(&line, &pursuit, Outcome::Satisfied, None, act(3)).unwrap();
        let moved = closing.point().unwrap().id();

        closing.apply(&mut line, &mut pursuit).unwrap();

        assert_eq!(line.head(), moved);
        assert_eq!(pursuit.outcome(), Some(Outcome::Satisfied));
        assert_eq!(line.states().len(), 1);
    }

    /// The refusal that keeps a satisfied close from being a claim
    /// nobody checked: everything this work wanted, the line already
    /// says, so there is nothing for a change point to carry.
    #[test]
    fn work_the_line_already_grants_is_refused_rather_than_landed() {
        let mut line = line();
        let entry = EntryId::new();
        let held = content();

        // Somebody else brings that entry back, with the content and
        // name this work was going to give it.
        landed(&mut line, vec![Op::add_to(entry, held, name("cut-01"))], 2);
        let mut pursuit = passing(
            work_on(&line),
            vec![Op::add_to(entry, held, name("cut-01"))],
            3,
        );

        let refused = close(&line, &pursuit, Outcome::Satisfied, None, act(4));

        // Every axis is normalised away against a line that already
        // says it, and nothing is left to record.
        assert_eq!(refused, Err(ForgeError::NothingToRecord));
        // Nothing was minted, so the work is still open and can be
        // abandoned instead.
        assert!(pursuit.outcome().is_none());
        let giving_up = close(&line, &pursuit, Outcome::Abandoned, None, act(5)).unwrap();
        assert!(giving_up.apply(&mut line, &mut pursuit).is_ok());
    }

    /// Two people adding what looks like the same thing are adding two
    /// things, and both land. An entry is what it is by id, so
    /// identical content under an unused name is a second arrival
    /// rather than a repeat of the first — deciding they are the same
    /// is a question about the bytes, and it is asked below this layer.
    #[test]
    fn the_same_content_arriving_as_a_second_entry_is_not_nothing() {
        let mut line = line();
        let held = content();
        let pursuit = passing(work_on(&line), vec![Op::add(held, name("cut-01"))], 2);
        landed(&mut line, vec![Op::add(held, name("cut-02"))], 3);

        let closing = close(&line, &pursuit, Outcome::Satisfied, None, act(4)).unwrap();

        assert!(closing.lands());
    }

    #[test]
    fn an_axis_the_line_moved_first_and_this_work_has_not_seen_is_refused() {
        let mut line = line();
        let entry = EntryId::new();
        let pursuit = passing(
            work_on(&line),
            vec![Op::add_to(entry, content(), name("cut-01"))],
            2,
        );
        // The same entry arrives on the line through other work.
        landed(
            &mut line,
            vec![Op::add_to(entry, content(), name("other"))],
            3,
        );

        let refused = close(&line, &pursuit, Outcome::Satisfied, None, act(4));

        let Err(ForgeError::Collides(found)) = refused else {
            panic!("the line moved an axis this work moves");
        };
        assert!(
            found
                .iter()
                .any(|c| c.entry == entry && c.axis == Axis::Content)
        );
    }

    /// Closing refuses collisions; it never settles them. The line's
    /// strategy is read where a pass is written, and by the time
    /// anybody is closing there is nothing left for it to decide.
    #[test]
    fn closing_refuses_a_collision_whatever_the_strategy_request() {
        for rule in ["mainline-first", "by-hand"] {
            let mut line = line();
            line.set_strategy(StrategyId::new(rule).unwrap(), act(1));
            let entry = EntryId::new();
            let pursuit = passing(
                work_on(&line),
                vec![Op::add_to(entry, content(), name("cut-01"))],
                2,
            );
            landed(
                &mut line,
                vec![Op::add_to(entry, content(), name("other"))],
                3,
            );

            let refused = close(&line, &pursuit, Outcome::Satisfied, None, act(4));

            assert!(matches!(refused, Err(ForgeError::Collides(_))));
        }
    }

    /// What clears a collision is the work saying what the line says.
    /// Nothing else does, because nothing else can be recorded.
    #[test]
    fn work_that_comes_round_to_the_lines_value_can_close() {
        let mut line = line();
        let entry = EntryId::new();
        let pursuit = passing(
            work_on(&line),
            vec![Op::add_to(entry, content(), name("cut-01"))],
            2,
        );
        landed(
            &mut line,
            vec![Op::add_to(entry, content(), name("cut-01"))],
            3,
        );
        assert!(matches!(
            close(&line, &pursuit, Outcome::Satisfied, None, act(4)),
            Err(ForgeError::Collides(_))
        ));

        // Coming round to the line's value, and bringing something of
        // its own that the line has not heard of.
        let theirs = line.states()[&entry].content.unwrap();
        let pursuit = passing(
            pursuit,
            vec![
                Op::replace(entry, theirs),
                Op::add(content(), name("cut-02")),
            ],
            5,
        );

        let closing = close(&line, &pursuit, Outcome::Satisfied, None, act(6)).unwrap();

        // The entry they disagreed about is not in what lands: the
        // work now says what the line says, so there is nothing to say.
        let point = closing
            .point()
            .expect("that work had something left to land");
        assert!(!point.table().rows().contains_key(&entry));
    }

    /// And when coming round leaves nothing at all, closing as
    /// satisfied is refused — the work has nothing to put on the line.
    #[test]
    fn work_that_is_left_saying_nothing_cannot_close_satisfied() {
        let mut line = line();
        let entry = EntryId::new();
        let pursuit = passing(
            work_on(&line),
            vec![Op::add_to(entry, content(), name("cut-01"))],
            2,
        );
        landed(
            &mut line,
            vec![Op::add_to(entry, content(), name("cut-01"))],
            3,
        );
        let theirs = line.states()[&entry].content.unwrap();
        let pursuit = passing(pursuit, vec![Op::replace(entry, theirs)], 4);

        let refused = close(&line, &pursuit, Outcome::Satisfied, None, act(5));

        assert_eq!(refused, Err(ForgeError::NothingToRecord));
    }

    #[test]
    fn work_against_another_line_is_refused_rather_than_judged() {
        let line = line();
        let elsewhere = Line::open(name("other"), strategy(), act(0));
        let pursuit = passing(work_on(&elsewhere), vec![Op::add(content(), name("a"))], 2);

        let refused = close(&line, &pursuit, Outcome::Satisfied, None, act(3));

        assert_eq!(refused, Err(ForgeError::NotThisLine));
    }

    #[test]
    fn work_that_has_ended_cannot_end_again() {
        let mut line = line();
        let mut pursuit = passing(work_on(&line), vec![Op::add(content(), name("cut-01"))], 2);
        close(&line, &pursuit, Outcome::Satisfied, None, act(3))
            .unwrap()
            .apply(&mut line, &mut pursuit)
            .unwrap();

        let refused = close(&line, &pursuit, Outcome::Abandoned, None, act(4));

        assert_eq!(refused, Err(ForgeError::AlreadyClosed));
    }

    /// The head moving between the decision and the application is the
    /// case the whole shape exists for. Nothing half-lands: the line
    /// refuses, and the pursuit is still open to be closed against the
    /// line as it now is.
    #[test]
    fn a_head_that_moved_after_the_decision_refuses_and_leaves_the_work_open() {
        let mut line = line();
        let mut pursuit = passing(work_on(&line), vec![Op::add(content(), name("cut-01"))], 2);
        let closing = close(&line, &pursuit, Outcome::Satisfied, None, act(3)).unwrap();

        landed(&mut line, vec![Op::add(content(), name("elsewhere"))], 4);

        let refused = closing.apply(&mut line, &mut pursuit);

        assert_eq!(refused, Err(ForgeError::NotOnHead));
        assert!(pursuit.outcome().is_none());
    }

    /// An archived line takes no change point, so the one close that
    /// would put one there is refused — and the one that would not is
    /// still allowed, because giving up is not a thing the line has an
    /// opinion about.
    #[test]
    fn an_archived_line_refuses_a_satisfied_close_and_allows_an_abandoned_one() {
        let mut line = line();
        let work = passing(work_on(&line), vec![Op::add(content(), name("one"))], 1);
        line.archive(act(2));

        let refused = close(&line, &work, Outcome::Satisfied, None, act(3));
        assert!(matches!(refused, Err(ForgeError::Archived)), "{refused:?}");

        let giving_up = close(&line, &work, Outcome::Abandoned, None, act(4))
            .expect("work against a finished line can still say it gave up");
        assert!(!giving_up.lands());
    }
}
