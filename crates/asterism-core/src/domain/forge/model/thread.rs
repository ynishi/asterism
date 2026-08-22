//! Saying something about work.
//!
//! ```text
//!   Thread ── anchor : Pursuit | Round | (Round, Entry) | ChangePoint
//!    │         title?
//!    └ Message ── body / act
//!         │        parent? : another message of this thread
//!         └ Revision ── body / act        the body now is the last one
//! ```
//!
//! A node carries a note, and a note is one line written once by
//! whoever wrote the node. This is the rest: a remark on somebody
//! else's pass, a question about one entry it touched, a review of
//! what landed.
//!
//! # Four anchors, because four things are worth remarking on
//!
//! A pursuit as a whole — what this work is for. A pass — the judgement
//! somebody made in it. One entry a pass touched — this particular
//! thing, in this particular attempt. And a change point — a review
//! after the fact.
//!
//! **The entry anchor is deliberately not an entry.** Hanging a remark
//! on an entry alone would make it follow that entry into every other
//! pursuit it is ever carried into, which is how a note about one
//! attempt becomes a note about the thing itself. Anchoring at
//! `(round, entry)` says *this entry, as this pass had it*, and that
//! does not travel. A change point can be anchored to on its own,
//! because there is nowhere for a remark on it to travel to.
//!
//! # It is the forge's own, and not the annotation surface downstairs
//!
//! There is a thread primitive in the layer below, and it is not this
//! one. It anchors to snapshots, cards and query groups — the things
//! that layer has — and the four things worth remarking on here are
//! not among them, nor could they be without that layer learning what
//! a pursuit is. Sharing the type would mean one of the two sides
//! carrying anchors it can never use, and every reader of either
//! side asking which half applies.
//!
//! So they are separate, and they say who did something in different
//! words: that one records the write-side attribution triple, this one
//! records an [`Act`], because the forge's actors include a line's own
//! rule and a rule is not somebody with an attribution.
//!
//! # Nothing is overwritten and nothing is resolved
//!
//! Editing a message appends a [`Revision`]; the body now is the last
//! one, and every earlier one stays readable. That is the same reason
//! the two logs work the way they do — what was said is a fact about
//! what happened, and a correction is another one.
//!
//! There is no resolved flag, no closing a thread, no marking a remark
//! as handled. Whether something is dealt with is a word people use
//! about their work, not a shape the record has; if it matters, a
//! later message says so and that is a better record than a boolean
//! nobody has to explain.
//!
//! # Order here is the clock, and that is a real difference
//!
//! Everywhere else in this model the chain orders things and a
//! timestamp is evidence. A discussion has no chain to read an order
//! out of — a reply names its parent, but two replies to one message
//! are ordered by nothing else — so messages are read in the order
//! they were written.
//!
//! It is affordable because nothing derives from it. No fold reads a
//! thread, no rule consults one, and no refusal depends on the order
//! of two remarks. A clock that steps backwards makes a conversation
//! read oddly; it cannot make the line wrong.

use crate::domain::forge::model::act::Act;
use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::history::ChangePoint;
use crate::domain::forge::model::pursuit::{Pursuit, Round};
use crate::domain::forge::model::value::{
    ChangePointId, EntryId, MessageId, Name, NodeId, PursuitId, ThreadId,
};

/// What a thread hangs off.
///
/// Built from the thing itself rather than from its id, so that a
/// thread anchored to something that was never written is not a value
/// anybody can make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// The work as a whole.
    Pursuit(PursuitId),
    /// One pass at it.
    Round(NodeId),
    /// One entry, as one pass had it.
    Entry {
        /// The pass.
        round: NodeId,
        /// The entry it touched.
        entry: EntryId,
    },
    /// What landed on the line.
    Change(ChangePointId),
}

impl Anchor {
    /// Hangs a thread off a piece of work.
    pub fn pursuit(work: &Pursuit) -> Self {
        Self::Pursuit(work.id())
    }

    /// Hangs a thread off one pass.
    pub fn round(round: &Round) -> Self {
        Self::Round(round.id())
    }

    /// Hangs a thread off one entry, as one pass had it.
    ///
    /// Refuses an entry the pass did not touch: a remark about what a
    /// pass did to something has to be about something it did.
    pub fn entry(round: &Round, entry: EntryId) -> Result<Self, ForgeError> {
        if !round.ops().iter().any(|op| op.entry() == entry) {
            return Err(ForgeError::NotInThatRound);
        }
        Ok(Self::Entry {
            round: round.id(),
            entry,
        })
    }

    /// Hangs a thread off what landed.
    pub fn change(point: &ChangePoint) -> Self {
        Self::Change(point.id())
    }
}

/// What somebody said.
///
/// Trimmed and never blank, for the reason a name is: a message that
/// says nothing is a message somebody has to read to find out it says
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Body(String);

impl Body {
    /// Takes what was said.
    pub fn new(said: impl Into<String>) -> Result<Self, ForgeError> {
        let said = said.into().trim().to_string();
        if said.is_empty() {
            return Err(ForgeError::BlankBody);
        }
        Ok(Self(said))
    }

    /// What it reads.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A correction to something already said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    body: Body,
    act: Act,
}

impl Revision {
    /// Records a correction.
    pub fn new(body: Body, act: Act) -> Self {
        Self { body, act }
    }

    /// What it now says.
    pub fn body(&self) -> &Body {
        &self.body
    }

    /// When it was corrected, and by whom.
    pub fn act(&self) -> &Act {
        &self.act
    }
}

/// One thing said, and every correction to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    id: MessageId,
    parent: Option<MessageId>,
    body: Body,
    act: Act,
    revisions: Vec<Revision>,
}

impl Message {
    /// Says something.
    pub fn new(parent: Option<MessageId>, body: Body, act: Act) -> Self {
        Self {
            id: MessageId::new(),
            parent,
            body,
            act,
            revisions: Vec::new(),
        }
    }

    /// Which message.
    pub fn id(&self) -> MessageId {
        self.id
    }

    /// What it replies to, if it replies to anything.
    pub fn parent(&self) -> Option<MessageId> {
        self.parent
    }

    /// What it says now — the last correction, or what was said first.
    pub fn body(&self) -> &Body {
        self.revisions
            .last()
            .map(Revision::body)
            .unwrap_or(&self.body)
    }

    /// What it said when it was written.
    pub fn said(&self) -> &Body {
        &self.body
    }

    /// Every correction, oldest first.
    pub fn revisions(&self) -> &[Revision] {
        &self.revisions
    }

    /// When it was said, and by whom.
    pub fn act(&self) -> &Act {
        &self.act
    }

    /// Records a correction. Nothing is overwritten.
    pub fn amend(&mut self, revision: Revision) {
        self.revisions.push(revision);
    }
}

/// A run of messages about one thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    id: ThreadId,
    anchor: Anchor,
    title: Option<Name>,
    messages: Vec<Message>,
}

impl Thread {
    /// Starts a thread with the first thing said in it.
    ///
    /// A thread with no messages would be somebody having opened a
    /// conversation and said nothing, which is not a record of
    /// anything.
    pub fn open(anchor: Anchor, title: Option<Name>, first: Message) -> Self {
        Self {
            id: ThreadId::new(),
            anchor,
            title,
            messages: vec![first],
        }
    }

    /// Which thread.
    pub fn id(&self) -> ThreadId {
        self.id
    }

    /// What it hangs off.
    pub fn anchor(&self) -> Anchor {
        self.anchor
    }

    /// What it is called, if anybody called it anything.
    pub fn title(&self) -> Option<&Name> {
        self.title.as_ref()
    }

    /// Everything said in it, in the order it was said.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Adds something said.
    ///
    /// Refuses a reply to a message of another thread: a reply reaching
    /// out of its own conversation would make "the thread this belongs
    /// to" a question with two answers.
    pub fn say(&mut self, message: Message) -> Result<(), ForgeError> {
        if let Some(parent) = message.parent()
            && !self.messages.iter().any(|held| held.id() == parent)
        {
            return Err(ForgeError::NotInThatThread);
        }
        self.messages.push(message);
        Ok(())
    }

    /// Corrects something said here.
    ///
    /// Refuses a message this thread does not hold, for the reason a
    /// reply is refused.
    pub fn amend(&mut self, message: MessageId, revision: Revision) -> Result<(), ForgeError> {
        let Some(held) = self.messages.iter_mut().find(|held| held.id() == message) else {
            return Err(ForgeError::NotInThatThread);
        };
        held.amend(revision);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    use crate::domain::attribution::AttributionContext;
    use crate::domain::forge::model::act::Actor;
    use crate::domain::forge::model::line::Line;
    use crate::domain::forge::model::op::Op;
    use crate::domain::forge::model::pursuit::Intent;
    use crate::domain::forge::model::value::{ActorId, Content, StrategyId};

    fn act(minute: u32) -> Act {
        let _ = AttributionContext::owner_surface();
        Act::new(
            Utc.with_ymd_and_hms(2026, 8, 21, 12, minute, 0).unwrap(),
            Actor::User(ActorId::new()),
        )
    }

    fn name(text: &str) -> Name {
        Name::new(text).unwrap()
    }

    fn body(text: &str) -> Body {
        Body::new(text).unwrap()
    }

    fn work() -> (Pursuit, Round, EntryId) {
        let line = Line::open(
            name(Line::ROOT),
            StrategyId::new("by-hand").unwrap(),
            act(0),
        );
        let mut work = Pursuit::open(line.id(), None, line.head(), Intent::default(), act(1));
        let arrival = Op::add(Content::from_uuid(Uuid::now_v7()), name("cut-01"));
        let entry = arrival.entry();
        let round = Round::new(work.head(), vec![arrival], None, act(2)).unwrap();
        work.push(round.clone()).unwrap();
        (work, round, entry)
    }

    #[test]
    fn a_thread_hangs_off_the_thing_it_is_about() {
        let (work, round, entry) = work();

        assert_eq!(Anchor::pursuit(&work), Anchor::Pursuit(work.id()));
        assert_eq!(Anchor::round(&round), Anchor::Round(round.id()));
        assert_eq!(
            Anchor::entry(&round, entry).unwrap(),
            Anchor::Entry {
                round: round.id(),
                entry
            }
        );
    }

    /// A remark about what a pass did to something has to be about
    /// something it did.
    #[test]
    fn an_entry_that_pass_never_touched_cannot_be_anchored_to() {
        let (_, round, _) = work();

        let refused = Anchor::entry(&round, EntryId::new());

        assert_eq!(refused.unwrap_err(), ForgeError::NotInThatRound);
    }

    #[test]
    fn a_correction_is_appended_and_the_first_wording_stays_readable() {
        let (work, _, _) = work();
        let first = Message::new(None, body("this looks wrong"), act(3));
        let said = first.id();
        let mut thread = Thread::open(Anchor::pursuit(&work), Some(name("the crop")), first);

        thread
            .amend(said, Revision::new(body("this looks wrong to me"), act(4)))
            .unwrap();

        let message = &thread.messages()[0];
        assert_eq!(message.body().as_str(), "this looks wrong to me");
        assert_eq!(message.said().as_str(), "this looks wrong");
        assert_eq!(message.revisions().len(), 1);
        assert_eq!(message.revisions()[0].act().at(), act(4).at());
    }

    #[test]
    fn a_reply_names_a_message_of_this_thread() {
        let (work, _, _) = work();
        let first = Message::new(None, body("why this one?"), act(3));
        let asked = first.id();
        let mut thread = Thread::open(Anchor::pursuit(&work), None, first);

        thread
            .say(Message::new(
                Some(asked),
                body("the other was darker"),
                act(4),
            ))
            .unwrap();

        assert_eq!(thread.messages().len(), 2);
        assert_eq!(thread.messages()[1].parent(), Some(asked));
    }

    #[test]
    fn a_reply_to_another_conversation_is_refused() {
        let (work, _, _) = work();
        let elsewhere = Message::new(None, body("said somewhere else"), act(3));
        let mut thread = Thread::open(
            Anchor::pursuit(&work),
            None,
            Message::new(None, body("said here"), act(3)),
        );

        let refused = thread.say(Message::new(
            Some(elsewhere.id()),
            body("replying across"),
            act(4),
        ));

        assert_eq!(refused.unwrap_err(), ForgeError::NotInThatThread);
        assert_eq!(thread.messages().len(), 1, "nothing was added");
    }

    #[test]
    fn correcting_a_message_this_thread_does_not_hold_is_refused() {
        let (work, _, _) = work();
        let mut thread = Thread::open(
            Anchor::pursuit(&work),
            None,
            Message::new(None, body("said here"), act(3)),
        );

        let refused = thread.amend(
            MessageId::new(),
            Revision::new(body("correcting a stranger"), act(4)),
        );

        assert_eq!(refused.unwrap_err(), ForgeError::NotInThatThread);
    }

    #[test]
    fn a_message_that_says_nothing_is_refused() {
        assert_eq!(Body::new("   ").unwrap_err(), ForgeError::BlankBody);
        assert_eq!(Body::new("").unwrap_err(), ForgeError::BlankBody);
        assert_eq!(Body::new("  said  ").unwrap().as_str(), "said");
    }

    /// There is no resolved flag to set, and no way to close a thread.
    /// Whether something is dealt with is said in a message like
    /// anything else.
    #[test]
    fn a_thread_has_no_state_to_be_in() {
        let (work, _, _) = work();
        let mut thread = Thread::open(
            Anchor::pursuit(&work),
            None,
            Message::new(None, body("this looks wrong"), act(3)),
        );

        thread
            .say(Message::new(None, body("fixed in the next pass"), act(5)))
            .unwrap();

        // Both are messages; neither is a status.
        assert_eq!(thread.messages().len(), 2);
    }
}
