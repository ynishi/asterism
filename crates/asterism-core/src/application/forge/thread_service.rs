//! Saying something about work, and correcting it.
//!
//! ```text
//!   open      resolves an anchor, writes the thread and its first message
//!   say       writes a message
//!   amend     writes a correction
//!   rename    writes the thread's title
//!
//!   get / anchored / about    read
//! ```
//!
//! Nothing here touches either log. A conversation about a round does
//! not change what the round said, and a remark on a change point does
//! not move the line — which is why this is a third face rather than
//! more verbs on the two that write records.
//!
//! # An anchor is resolved, not accepted
//!
//! [`Anchor`] is built from the thing itself rather than from its id,
//! so that a thread hanging off something nobody wrote is not a value
//! anybody can make. A caller with an id has not got the thing, so
//! this service reads it: [`Anchored`] names what to look for, and
//! every verb that takes one loads the pursuit or the line before a
//! thread exists.
//!
//! That is the whole of what this service decides, and it decides it
//! by asking. The refusals are the model's — an entry a round did not
//! touch is [`Anchor::entry`]'s refusal, not one written here.
//!
//! # What this service is allowed to decide
//!
//! Nothing else. It resolves, calls the model, and writes back what
//! came out.

use std::sync::Arc;

use crate::domain::attribution::AttributionContext;
use crate::domain::forge::boundary::Actors;
use crate::domain::forge::clock::Clock;
use crate::domain::forge::lines::Lines;
use crate::domain::forge::model::act::{Act, Actor};
use crate::domain::forge::model::pursuit::{Pursuit, Round};
use crate::domain::forge::model::thread::{Anchor, Body, Message, Revision, Thread};
use crate::domain::forge::model::value::{
    ChangePointId, EntryId, LineId, MessageId, Name, NodeId, PursuitId, ThreadId,
};
use crate::domain::forge::pursuits::Pursuits;
use crate::domain::forge::threads::Threads;
use crate::error::DomainError;

/// What a caller says a thread hangs off, in ids.
///
/// The id-shaped half of [`Anchor`]. A caller has ids; the model wants
/// the things, and what turns one into the other is a read — so this
/// is the argument, and [`Anchor`] is what the service has after it
/// has looked.
///
/// A round and an entry name the work they are in as well as the node.
/// The node id alone would be enough to find a row, and not enough to
/// refuse one: a round belongs to a pursuit, and asking for it without
/// saying which work would take a node from whichever log had it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchored {
    /// The work as a whole.
    Pursuit(PursuitId),
    /// One round at it.
    Round(PursuitId, NodeId),
    /// One entry, as one round had it.
    Entry(PursuitId, NodeId, EntryId),
    /// What landed on the line.
    Change(LineId, ChangePointId),
}

/// Conversation use-case service.
pub struct ThreadService {
    threads: Arc<dyn Threads>,
    pursuits: Arc<dyn Pursuits>,
    lines: Arc<dyn Lines>,
    actors: Arc<dyn Actors>,
    clock: Arc<dyn Clock>,
}

impl ThreadService {
    /// Wires the service around its ports.
    pub fn new(
        threads: Arc<dyn Threads>,
        pursuits: Arc<dyn Pursuits>,
        lines: Arc<dyn Lines>,
        actors: Arc<dyn Actors>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            threads,
            pursuits,
            lines,
            actors,
            clock,
        }
    }

    /// Starts a conversation about something, with the first thing
    /// said in it.
    ///
    /// The anchor is read before anything is written, so a thread
    /// about a round nobody wrote is refused rather than kept.
    pub async fn open(
        &self,
        about: Anchored,
        title: Option<Name>,
        said: Body,
        by: &AttributionContext,
    ) -> Result<Thread, DomainError> {
        let anchor = self.resolve(about).await?;
        let first = Message::new(None, said, self.act(by).await?);
        let thread = Thread::open(anchor, title, first);
        self.threads.open(&thread).await?;
        Ok(thread)
    }

    /// Reads a conversation back whole, corrections and all.
    pub async fn get(&self, id: &ThreadId) -> Result<Thread, DomainError> {
        self.threads
            .get(id)
            .await?
            .ok_or_else(|| DomainError::not_found("thread", id))
    }

    /// Everything said about one thing.
    ///
    /// More than one, because two people can start separate
    /// conversations about one round and merging them would be deciding
    /// they were about the same thing.
    pub async fn about(&self, about: Anchored) -> Result<Vec<Thread>, DomainError> {
        let anchor = self.resolve(about).await?;
        self.threads.anchored(anchor).await
    }

    /// Says something in a conversation that already exists.
    ///
    /// `replying_to` names a message of this thread, or nothing. A
    /// reply to a message of another conversation is refused by the
    /// model, which is asked here against the thread as it reads now
    /// and asked again by the port against the thread as it writes.
    pub async fn say(
        &self,
        id: &ThreadId,
        replying_to: Option<MessageId>,
        said: Body,
        by: &AttributionContext,
    ) -> Result<Message, DomainError> {
        let mut thread = self.get(id).await?;
        let message = Message::new(replying_to, said, self.act(by).await?);
        thread.say(message.clone())?;
        self.threads.say(id, &message).await?;
        Ok(message)
    }

    /// Corrects something said. Nothing is overwritten.
    ///
    /// What the message says now becomes the correction; what it said
    /// before stays readable, because a correction is another fact
    /// about what happened rather than a repair of the first one.
    pub async fn amend(
        &self,
        id: &ThreadId,
        message: &MessageId,
        said: Body,
        by: &AttributionContext,
    ) -> Result<Revision, DomainError> {
        let mut thread = self.get(id).await?;
        let revision = Revision::new(said, self.act(by).await?);
        thread.amend(*message, revision.clone())?;
        self.threads.amend(id, message, &revision).await?;
        Ok(revision)
    }

    /// Renames a conversation, or takes its name off.
    ///
    /// A title is a label on the conversation rather than something
    /// said in it, so moving it is not a message.
    pub async fn rename(
        &self,
        id: &ThreadId,
        title: Option<&Name>,
        by: &AttributionContext,
    ) -> Result<(), DomainError> {
        self.get(id).await?;
        let act = self.act(by).await?;
        self.threads.rename(id, title, &act).await
    }

    /// Finds what a caller named, and builds the anchor out of it.
    ///
    /// Every arm is a read that can fail, and that is the point: the
    /// model cannot be handed an anchor to something that is not
    /// there, so this is where "not there" is answered.
    async fn resolve(&self, about: Anchored) -> Result<Anchor, DomainError> {
        match about {
            Anchored::Pursuit(id) => Ok(Anchor::pursuit(&self.work(&id).await?)),
            Anchored::Round(id, node) => {
                let work = self.work(&id).await?;
                Ok(Anchor::round(round_of(&work, node)?))
            }
            Anchored::Entry(id, node, entry) => {
                let work = self.work(&id).await?;
                Ok(Anchor::entry(round_of(&work, node)?, entry)?)
            }
            Anchored::Change(line, point) => {
                let held = self
                    .lines
                    .get(&line)
                    .await?
                    .ok_or_else(|| DomainError::not_found("line", line))?;
                let landed = held
                    .history()
                    .changes()
                    .iter()
                    .find(|change| change.id() == point)
                    .ok_or_else(|| {
                        DomainError::Validation(format!(
                            "line {line} has no change point {point} to say anything about"
                        ))
                    })?;
                Ok(Anchor::change(landed))
            }
        }
    }

    async fn work(&self, id: &PursuitId) -> Result<Pursuit, DomainError> {
        self.pursuits
            .get(id)
            .await?
            .ok_or_else(|| DomainError::not_found("pursuit", id))
    }

    /// Stamps an act: now, by whoever this write is from.
    async fn act(&self, by: &AttributionContext) -> Result<Act, DomainError> {
        Ok(Act::new(
            self.clock.now(),
            Actor::User(self.actors.resolve(by).await?),
        ))
    }
}

/// The round this work wrote under `node`.
///
/// A free function rather than a method on `Pursuit`, because looking
/// a node up is what a caller holding an id has to do and not
/// something the model owes: nothing inside the model reaches a round
/// by id, since everything there already has the value.
fn round_of(work: &Pursuit, node: NodeId) -> Result<&Round, DomainError> {
    work.rounds()
        .iter()
        .find(|round| round.id() == node)
        .ok_or_else(|| {
            DomainError::Validation(format!(
                "work {} has no round {node} to say anything about",
                work.id()
            ))
        })
}
