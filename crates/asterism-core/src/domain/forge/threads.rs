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
//! # Nothing removes and nothing overwrites
//!
//! No delete, and no method that replaces a message. Correcting one
//! appends, which is what [`Thread::amend`] does in memory and what
//! this keeps. The absence is the same one the two logs have and it is
//! there for the same reason.
//!
//! [`Thread::amend`]: crate::domain::forge::model::thread::Thread::amend

use async_trait::async_trait;

use crate::domain::forge::model::act::Act;
use crate::domain::forge::model::thread::{Anchor, Message, Revision, Thread};
use crate::domain::forge::model::value::{MessageId, ThreadId};
// SHARED KERNEL: `DomainError` is a boundary type.
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
    /// can start separate conversations about one pass, and merging
    /// them would be deciding they were about the same thing.
    async fn anchored(&self, anchor: Anchor) -> Result<Vec<Thread>, DomainError>;

    /// Adds something said.
    ///
    /// Returns [`Conflict`](DomainError::Conflict) if the message it
    /// replies to is not in this thread — the same refusal the model
    /// makes, restated here because the model judged the thread as it
    /// was read.
    async fn say(&self, thread: &ThreadId, message: &Message) -> Result<(), DomainError>;

    /// Records a correction to something said.
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
