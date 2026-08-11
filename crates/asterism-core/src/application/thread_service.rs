//! `ThreadService` — application-layer surface for `Thread` and its
//! `Message` children.
//!
//! Verbs:
//! - [`list`](ThreadService::list) — Threads under a given anchor.
//! - [`create`](ThreadService::create) — new Thread with a title +
//!   anchor.
//! - [`archive`](ThreadService::archive) — toggle the `archived`
//!   flag (soft hide).
//! - [`delete`](ThreadService::delete) — hard delete (cascades to
//!   messages).
//! - [`list_messages`](ThreadService::list_messages) — chronological
//!   read, optional `since` cursor for polling.
//! - [`append_message`](ThreadService::append_message) — append a
//!   Message. Same verb for UI (`author = human`) and HTTP
//!   (`author = claude_code` / `agent` / `persona`). Idempotency
//!   key deduplicates safely.
//! - [`delete_message`](ThreadService::delete_message) — remove one
//!   Message (misfire correction; there is no edit verb).
//!
//! Anchor validation: `snapshot` and `query_group` anchors are
//! id-only in P1 — the service parses the id and lets the adapter
//! surface a foreign-key error on write. `Card` (asset) anchors are
//! reserved for a later phase (P3); the service still
//! accepts them today because the anchor parser does.
//!
//! Every write here takes an [`AttributionContext`] it does not persist.
//! A Message already names its own writer through
//! [`thread::Author`](crate::domain::thread::Author), which is a
//! different factorisation of the same question (attribution splits it
//! into author × operator; `thread::Author` folds human and agent into
//! one enum). Unifying the two is deliberately left to a later
//! wave — this argument neither feeds nor overrides
//! `author_kind`.

use std::sync::Arc;

use asterism_contract::command::{
    AppendMessageCommand, ArchiveThreadCommand, CreateThreadCommand, DeleteMessageCommand,
    DeleteThreadCommand,
};
use asterism_contract::dto::{MessageDto, ThreadDto};
use chrono::{DateTime, Utc};

use crate::application::mapping::{
    message_to_dto, parse_message_id, parse_message_ref, parse_persona_id, parse_thread_anchor,
    parse_thread_id, thread_to_dto,
};
use crate::domain::attribution::AttributionContext;
use crate::domain::repository::{PersonaRepository, ThreadRepository};
use crate::domain::thread::{Author, Message, Role, Thread};
use crate::error::DomainError;

/// Application-layer surface for the Thread primitive.
pub struct ThreadService {
    threads: Arc<dyn ThreadRepository>,
    personas: Arc<dyn PersonaRepository>,
}

impl ThreadService {
    /// Wires the service around its ports.
    pub fn new(threads: Arc<dyn ThreadRepository>, personas: Arc<dyn PersonaRepository>) -> Self {
        Self { threads, personas }
    }

    /// Lists Threads under `anchor_kind` / `anchor_id`. Archived
    /// Threads are excluded unless `include_archived` is set.
    pub async fn list(
        &self,
        anchor_kind: &str,
        anchor_id: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<ThreadDto>, DomainError> {
        let anchor = parse_thread_anchor(anchor_kind, anchor_id)?;
        let rows = self
            .threads
            .list_by_anchor(&anchor, include_archived)
            .await?;
        Ok(rows.iter().map(thread_to_dto).collect())
    }

    /// Fetches one Thread by id (`None` when absent).
    pub async fn find(&self, thread_id: &str) -> Result<Option<ThreadDto>, DomainError> {
        let tid = parse_thread_id(thread_id)?;
        Ok(self.threads.find(&tid).await?.as_ref().map(thread_to_dto))
    }

    /// Creates a Thread with the given anchor + title.
    pub async fn create(
        &self,
        command: CreateThreadCommand,
        _attribution: &AttributionContext,
    ) -> Result<ThreadDto, DomainError> {
        let anchor = parse_thread_anchor(&command.anchor_kind, command.anchor_id.as_deref())?;
        let thread = Thread::new(anchor, command.title, Utc::now())?;
        self.threads.save(&thread).await?;
        // Read back so projection columns (which are trigger-owned)
        // are consistent — a fresh Thread reads back with the same
        // zero counters, so the round-trip is cheap.
        let fresh = self
            .threads
            .find(&thread.id)
            .await?
            .ok_or_else(|| DomainError::Validation("thread vanished after save".into()))?;
        Ok(thread_to_dto(&fresh))
    }

    /// Toggles the `archived` flag on a Thread.
    pub async fn archive(
        &self,
        command: ArchiveThreadCommand,
        _attribution: &AttributionContext,
    ) -> Result<ThreadDto, DomainError> {
        let tid = parse_thread_id(&command.thread_id)?;
        let mut thread = self
            .threads
            .find(&tid)
            .await?
            .ok_or_else(|| DomainError::not_found("thread", &command.thread_id))?;
        if thread.archived != command.archived {
            thread.archived = command.archived;
            thread.updated_at = Utc::now();
            self.threads.save(&thread).await?;
        }
        Ok(thread_to_dto(&thread))
    }

    /// Deletes a Thread (cascades to its Messages). Idempotent.
    pub async fn delete(
        &self,
        command: DeleteThreadCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let tid = parse_thread_id(&command.thread_id)?;
        self.threads.delete(&tid).await
    }

    /// Lists a Thread's Messages in chronological order. `since_ms`
    /// is an exclusive cursor — pass the greatest `created_at_ms`
    /// the caller has seen to poll for fresh writes.
    pub async fn list_messages(
        &self,
        thread_id: &str,
        since_ms: Option<i64>,
    ) -> Result<Vec<MessageDto>, DomainError> {
        let tid = parse_thread_id(thread_id)?;
        let since: Option<DateTime<Utc>> = since_ms
            .map(|ms| {
                DateTime::<Utc>::from_timestamp_millis(ms).ok_or_else(|| {
                    DomainError::Validation(format!("timestamp out of range in since_ms: {ms}"))
                })
            })
            .transpose()?;
        let rows = self.threads.list_messages(&tid, since).await?;
        Ok(rows.iter().map(message_to_dto).collect())
    }

    /// Appends a Message to a Thread. Same code path for UI and
    /// HTTP writes — the author identity is data.
    pub async fn append_message(
        &self,
        command: AppendMessageCommand,
        _attribution: &AttributionContext,
    ) -> Result<MessageDto, DomainError> {
        let tid = parse_thread_id(&command.thread_id)?;
        if self.threads.find(&tid).await?.is_none() {
            return Err(DomainError::not_found("thread", &command.thread_id));
        }
        let author = match command.author_kind.as_str() {
            "human" => Author::Human,
            "claude_code" => Author::ClaudeCode,
            "agent" => {
                let name = command
                    .author_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        DomainError::Validation(
                            "author_kind = agent requires non-empty author_name".into(),
                        )
                    })?;
                Author::Agent(name.to_string())
            }
            "persona" => {
                let pid_str = command.author_persona_id.as_deref().ok_or_else(|| {
                    DomainError::Validation(
                        "author_kind = persona requires author_persona_id".into(),
                    )
                })?;
                let pid = parse_persona_id(pid_str)?;
                if self.personas.find(&pid).await?.is_none() {
                    return Err(DomainError::PersonaNotFound(pid));
                }
                Author::Persona(pid)
            }
            other => {
                return Err(DomainError::Validation(format!(
                    "unknown author_kind: {other:?}"
                )));
            }
        };
        let role = Role::from_slug(&command.role)?;
        let refs = command
            .refs
            .iter()
            .map(parse_message_ref)
            .collect::<Result<Vec<_>, _>>()?;
        let message = Message::new(tid, author, role, command.body, refs, Utc::now())?;
        let stored = self
            .threads
            .append_message(&message, command.idempotency_key.as_deref())
            .await?;
        Ok(message_to_dto(&stored))
    }

    /// Deletes a Message (misfire correction). Idempotent.
    pub async fn delete_message(
        &self,
        command: DeleteMessageCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let mid = parse_message_id(&command.message_id)?;
        self.threads.delete_message(&mid).await
    }
}
