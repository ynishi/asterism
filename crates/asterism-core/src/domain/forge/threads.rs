//! Keeping what was said about work.
//!
//! One face, and a narrow one. A thread is read by whoever is looking
//! at the thing it hangs off, so the question this answers is always
//! "what was said about this", never "what was said lately".
//!
//! # Reading gives back the whole thread
//!
//! Messages and every correction to them. A conversation is read in
//! full or it is read misleadingly: a correction the reader does not
//! see is a sentence attributed to somebody who withdrew it.
//!
//! # Nothing here removes and nothing overwrites
//!
//! No delete, and no method that replaces a message. Correcting one
//! appends, which is what [`Thread::amend`] does in memory and what
//! this keeps. The absence is the same one the two logs have and it is
//! there for the same reason.
//!
//! One thing does take a conversation, and it is not on this face:
//! [`Lines::discard`] takes every thread anchored to the line it
//! drops — to a pursuit, a round, an entry as a round had it, or a
//! change point, which is every anchor [`Anchor`] has. The alternative
//! is a remark about something that no longer exists. A conversation
//! ends when the line it is about is thrown away, and nowhere else.
//!
//! [`Thread::amend`]: crate::domain::forge::model::thread::Thread::amend
//! [`Anchor`]: crate::domain::forge::model::thread::Anchor
//! [`Lines::discard`]: super::lines::Lines::discard

use async_trait::async_trait;

use crate::domain::forge::model::act::Act;
use crate::domain::forge::model::thread::{Anchor, Message, Revision, Thread};
use crate::domain::forge::model::value::{MessageId, ThreadId};
// SHARED VOCABULARY: `DomainError` is a boundary type.
use crate::error::DomainError;

/// Keeps what was said.
#[async_trait]
pub trait Threads: Send + Sync {
    /// Records a thread that has just been opened, first message and
    /// all.
    async fn open(&self, thread: &Thread) -> Result<(), DomainError>;

    /// Reads a thread back whole.
    async fn get(&self, id: &ThreadId) -> Result<Option<Thread>, DomainError>;

    /// Everything said about one thing.
    ///
    /// More than one thread can hang off the same anchor — two people
    /// can start separate conversations about one round, and merging
    /// them would be deciding they were about the same thing.
    async fn anchored(&self, anchor: Anchor) -> Result<Vec<Thread>, DomainError>;

    /// Adds something said.
    ///
    /// Returns [`Validation`](DomainError::Validation) if the message it
    /// replies to is not in this thread — the same refusal the model
    /// makes, restated here because the model judged the thread as it
    /// was read.
    ///
    /// **The same refusal means the same answer.** This said `Conflict`
    /// while the model said `Validation`, so one situation answered 409
    /// through the port and 400 through the service, and the sentence
    /// above claiming they were the same refusal was the evidence that
    /// one of them was wrong. Nothing here is contended: the caller
    /// addressed one conversation and named a message of another, which
    /// no row could change to make true.
    async fn say(&self, thread: &ThreadId, message: &Message) -> Result<(), DomainError>;

    /// Records a correction to something said.
    ///
    /// Returns [`Validation`](DomainError::Validation) if the message
    /// being corrected is not in this thread, on the same reading as
    /// [`say`](Self::say): the caller addressed one conversation and
    /// named something that is not in it, which no row could change.
    /// This answered `Conflict` too, and is the half that had no doc
    /// saying so — a correction is not a reply, and a sentence about
    /// replies would not have covered it.
    async fn amend(
        &self,
        thread: &ThreadId,
        message: &MessageId,
        revision: &Revision,
    ) -> Result<(), DomainError>;

    /// Records that a thread was renamed.
    ///
    /// Its title is a label on the conversation rather than something
    /// said in it, so moving it is not a message.
    async fn rename(
        &self,
        id: &ThreadId,
        title: Option<&crate::domain::forge::model::value::Name>,
        act: &Act,
    ) -> Result<(), DomainError>;
}
