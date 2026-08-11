//! SQLite adapter for the `ThreadRepository` port.
//!
//! Storage shape mirrors the V20 migration in
//! [`crate::sqlite::migrations`]. Projection columns on `thread`
//! (`message_count`, `last_message_at`,
//! `updated_at`) are trigger-maintained; `save` therefore preserves
//! them across an `ON CONFLICT DO UPDATE` (title / archive edits only
//! touch the caller-owned columns). `append_message` is idempotent by
//! `(thread_id, idempotency_key)` — a repeat write returns the
//! pre-existing row instead of a constraint error.

use asterism_core::domain::repository::ThreadRepository;
use asterism_core::domain::thread::{Author, EntityRef, Message, Role, Thread, ThreadAnchor};
use asterism_core::domain::value::{AssetId, GroupId, MessageId, PersonaId, SnapshotId, ThreadId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
use rusqlite_isle::AsyncIsle;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `ThreadRepository`.
#[derive(Clone)]
pub struct SqliteThreadRepository {
    isle: AsyncIsle,
}

impl SqliteThreadRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

struct ThreadRow {
    id: Uuid,
    title: String,
    anchor_kind: String,
    anchor_id: Option<Uuid>,
    created_at: i64,
    updated_at: i64,
    last_message_at: Option<i64>,
    message_count: i64,
    archived: i64,
}

impl ThreadRow {
    const COLUMNS: &'static str = "id, title, anchor_kind, anchor_id, created_at, \
         updated_at, last_message_at, message_count, archived";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            title: row.get(1)?,
            anchor_kind: row.get(2)?,
            anchor_id: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
            last_message_at: row.get(6)?,
            message_count: row.get(7)?,
            archived: row.get(8)?,
        })
    }

    fn into_domain(self) -> Result<Thread, DomainError> {
        let anchor = match self.anchor_kind.as_str() {
            "app_global" => ThreadAnchor::AppGlobal,
            "snapshot" => ThreadAnchor::Snapshot(SnapshotId::from_uuid(
                self.anchor_id
                    .ok_or_else(|| infra_msg("anchor_kind = snapshot but anchor_id NULL"))?,
            )),
            "query_group" => ThreadAnchor::QueryGroup(GroupId::from_uuid(
                self.anchor_id
                    .ok_or_else(|| infra_msg("anchor_kind = query_group but anchor_id NULL"))?,
            )),
            "card" => ThreadAnchor::Card(AssetId::from_uuid(
                self.anchor_id
                    .ok_or_else(|| infra_msg("anchor_kind = card but anchor_id NULL"))?,
            )),
            other => return Err(infra_msg(format!("unknown anchor_kind: {other:?}"))),
        };
        let message_count = u32::try_from(self.message_count)
            .map_err(|_| infra_msg(format!("negative message_count: {}", self.message_count)))?;
        Ok(Thread {
            id: ThreadId::from_uuid(self.id),
            title: self.title,
            anchor,
            created_at: ms_to_datetime(self.created_at)?,
            updated_at: ms_to_datetime(self.updated_at)?,
            last_message_at: self.last_message_at.map(ms_to_datetime).transpose()?,
            message_count,
            archived: self.archived != 0,
        })
    }
}

struct MessageRow {
    id: Uuid,
    thread_id: Uuid,
    author_kind: String,
    author_name: Option<String>,
    author_persona_id: Option<Uuid>,
    role: String,
    body: String,
    refs_json: String,
    created_at: i64,
}

impl MessageRow {
    const COLUMNS: &'static str = "id, thread_id, author_kind, author_name, author_persona_id, \
         role, body, refs_json, created_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            thread_id: row.get(1)?,
            author_kind: row.get(2)?,
            author_name: row.get(3)?,
            author_persona_id: row.get(4)?,
            role: row.get(5)?,
            body: row.get(6)?,
            refs_json: row.get(7)?,
            created_at: row.get(8)?,
        })
    }

    fn into_domain(self) -> Result<Message, DomainError> {
        let author = match self.author_kind.as_str() {
            "human" => Author::Human,
            "claude_code" => Author::ClaudeCode,
            "agent" => Author::Agent(
                self.author_name
                    .ok_or_else(|| infra_msg("author_kind = agent but author_name NULL"))?,
            ),
            "persona" => Author::Persona(PersonaId::from_uuid(
                self.author_persona_id
                    .ok_or_else(|| infra_msg("author_kind = persona but persona id NULL"))?,
            )),
            other => return Err(infra_msg(format!("unknown author_kind: {other:?}"))),
        };
        let role = Role::from_slug(&self.role)?;
        let refs = decode_refs(&self.refs_json)?;
        Ok(Message {
            id: MessageId::from_uuid(self.id),
            thread_id: ThreadId::from_uuid(self.thread_id),
            author,
            role,
            body: self.body,
            refs,
            created_at: ms_to_datetime(self.created_at)?,
        })
    }
}

fn infra_msg(msg: impl Into<String>) -> DomainError {
    DomainError::Infra(anyhow::anyhow!(msg.into()))
}

// Serde bridge for the `refs_json` TEXT column. Keeping the wire
// shape here (rather than reusing the wire DTO) prevents an
// asterism-contract dependency leaking into the persistence layer.
#[derive(Serialize, Deserialize)]
struct RefRow {
    kind: String,
    id: String,
}

fn encode_refs(refs: &[EntityRef]) -> String {
    let rows: Vec<RefRow> = refs
        .iter()
        .map(|r| RefRow {
            kind: r.kind_slug().to_string(),
            id: r.ref_id(),
        })
        .collect();
    serde_json::to_string(&rows).expect("static refs shape always serialises")
}

fn decode_refs(json: &str) -> Result<Vec<EntityRef>, DomainError> {
    let rows: Vec<RefRow> =
        serde_json::from_str(json).map_err(|e| infra_msg(format!("corrupt refs_json: {e}")))?;
    rows.into_iter()
        .map(|r| {
            let uuid = Uuid::parse_str(&r.id)
                .map_err(|_| infra_msg(format!("refs_json: invalid uuid {:?}", r.id)))?;
            match r.kind.as_str() {
                "card" => Ok(EntityRef::Card(AssetId::from_uuid(uuid))),
                "snapshot" => Ok(EntityRef::Snapshot(SnapshotId::from_uuid(uuid))),
                "query_group" => Ok(EntityRef::QueryGroup(GroupId::from_uuid(uuid))),
                other => Err(infra_msg(format!("refs_json: unknown kind {other:?}"))),
            }
        })
        .collect()
}

fn anchor_wire(anchor: &ThreadAnchor) -> (String, Option<Uuid>) {
    let kind = anchor.kind_slug().to_string();
    let id = match anchor {
        ThreadAnchor::AppGlobal => None,
        ThreadAnchor::Snapshot(id) => Some(*id.as_uuid()),
        ThreadAnchor::QueryGroup(id) => Some(*id.as_uuid()),
        ThreadAnchor::Card(id) => Some(*id.as_uuid()),
    };
    (kind, id)
}

#[async_trait]
impl ThreadRepository for SqliteThreadRepository {
    async fn find(&self, id: &ThreadId) -> Result<Option<Thread>, DomainError> {
        let uuid = *id.as_uuid();
        let row = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM thread WHERE id = ?1",
                    ThreadRow::COLUMNS
                ))?;
                let row: Option<ThreadRow> = stmt
                    .query_row(params![uuid], ThreadRow::from_row)
                    .optional()?;
                Ok(row)
            })
            .await
            .map_err(infra_err)?;
        row.map(ThreadRow::into_domain).transpose()
    }

    async fn list_by_anchor(
        &self,
        anchor: &ThreadAnchor,
        include_archived: bool,
    ) -> Result<Vec<Thread>, DomainError> {
        let (kind, id) = anchor_wire(anchor);
        let rows = self
            .isle
            .call(move |conn| {
                let sql = if include_archived {
                    format!(
                        "SELECT {} FROM thread
                            WHERE anchor_kind = ?1 AND (anchor_id IS ?2)
                            ORDER BY last_message_at DESC NULLS LAST, created_at DESC",
                        ThreadRow::COLUMNS
                    )
                } else {
                    format!(
                        "SELECT {} FROM thread
                            WHERE anchor_kind = ?1 AND (anchor_id IS ?2)
                              AND archived = 0
                            ORDER BY last_message_at DESC NULLS LAST, created_at DESC",
                        ThreadRow::COLUMNS
                    )
                };
                let mut stmt = conn.prepare(&sql)?;
                let rows: Vec<ThreadRow> = stmt
                    .query_map(params![kind, id], ThreadRow::from_row)?
                    .collect::<Result<_, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(ThreadRow::into_domain).collect()
    }

    async fn save(&self, thread: &Thread) -> Result<(), DomainError> {
        let (anchor_kind, anchor_id) = anchor_wire(&thread.anchor);
        let id = *thread.id.as_uuid();
        let title = thread.title.clone();
        let created = datetime_to_ms(&thread.created_at);
        let updated = datetime_to_ms(&thread.updated_at);
        let archived = i64::from(thread.archived);
        self.isle
            .call(move |conn| {
                // Projection columns (`message_count`, `last_message_at`)
                // are trigger-owned; the ON CONFLICT branch touches only
                // title / archive / updated_at so a title edit never
                // clobbers the running counters.
                conn.execute(
                    "INSERT INTO thread
                         (id, title, anchor_kind, anchor_id, created_at,
                          updated_at, last_message_at, message_count, archived)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, ?7)
                     ON CONFLICT(id) DO UPDATE SET
                         title      = excluded.title,
                         archived   = excluded.archived,
                         updated_at = excluded.updated_at",
                    params![
                        id,
                        title,
                        anchor_kind,
                        anchor_id,
                        created,
                        updated,
                        archived
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn delete(&self, id: &ThreadId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute("DELETE FROM thread WHERE id = ?1", params![uuid])?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn list_messages(
        &self,
        thread_id: &ThreadId,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<Message>, DomainError> {
        let tid = *thread_id.as_uuid();
        let since_ms = since.as_ref().map(datetime_to_ms);
        let rows = self
            .isle
            .call(move |conn| {
                let rows: Vec<MessageRow> = if let Some(cursor) = since_ms {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {} FROM message
                            WHERE thread_id = ?1 AND created_at > ?2
                            ORDER BY created_at, id",
                        MessageRow::COLUMNS
                    ))?;
                    stmt.query_map(params![tid, cursor], MessageRow::from_row)?
                        .collect::<Result<_, _>>()?
                } else {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {} FROM message
                            WHERE thread_id = ?1
                            ORDER BY created_at, id",
                        MessageRow::COLUMNS
                    ))?;
                    stmt.query_map(params![tid], MessageRow::from_row)?
                        .collect::<Result<_, _>>()?
                };
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(MessageRow::into_domain).collect()
    }

    async fn append_message(
        &self,
        message: &Message,
        idempotency_key: Option<&str>,
    ) -> Result<Message, DomainError> {
        let id = *message.id.as_uuid();
        let thread_id = *message.thread_id.as_uuid();
        let author_kind = message.author.kind_slug().to_string();
        let author_name = message.author.agent_name().map(str::to_string);
        let author_persona_id = message.author.persona_id().map(|p| *p.as_uuid());
        let role = message.role.slug().to_string();
        let body = message.body.clone();
        let refs_json = encode_refs(&message.refs);
        let created = datetime_to_ms(&message.created_at);
        let key = idempotency_key.map(str::to_string);

        let row = self
            .isle
            .call(move |conn| {
                if let Some(k) = key.as_ref() {
                    // Fast path: the row may already exist from a
                    // previous idempotent write. Fetching first avoids
                    // even the INSERT attempt in the common retry case
                    // and keeps the returned Message deterministic.
                    let existing: Option<MessageRow> = conn
                        .prepare(&format!(
                            "SELECT {} FROM message
                                WHERE thread_id = ?1 AND idempotency_key = ?2",
                            MessageRow::COLUMNS
                        ))?
                        .query_row(params![thread_id, k], MessageRow::from_row)
                        .optional()?;
                    if let Some(existing) = existing {
                        return Ok(existing);
                    }
                }
                conn.execute(
                    "INSERT INTO message
                         (id, thread_id, author_kind, author_name, author_persona_id,
                          role, body, refs_json, idempotency_key, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        id,
                        thread_id,
                        author_kind,
                        author_name,
                        author_persona_id,
                        role,
                        body,
                        refs_json,
                        key,
                        created,
                    ],
                )?;
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM message WHERE id = ?1",
                    MessageRow::COLUMNS
                ))?;
                let inserted = stmt.query_row(params![id], MessageRow::from_row)?;
                Ok(inserted)
            })
            .await
            .map_err(infra_err)?;
        row.into_domain()
    }

    async fn delete_message(&self, id: &MessageId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute("DELETE FROM message WHERE id = ?1", params![uuid])?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_700_000_000_000).unwrap()
    }

    #[tokio::test]
    async fn app_global_thread_roundtrip_with_message_projection() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteThreadRepository::new(isle);

        let thread = Thread::new(ThreadAnchor::AppGlobal, "Inbox", now()).unwrap();
        repo.save(&thread).await.unwrap();

        let fresh = repo.find(&thread.id).await.unwrap().unwrap();
        assert_eq!(fresh.title, "Inbox");
        assert_eq!(fresh.message_count, 0);
        assert!(fresh.last_message_at.is_none());
        assert!(!fresh.archived);

        let msg = Message::new(
            thread.id,
            Author::Human,
            Role::Note,
            "first note",
            vec![],
            now(),
        )
        .unwrap();
        let stored = repo.append_message(&msg, None).await.unwrap();
        assert_eq!(stored.body, "first note");

        // Trigger side effects: message_count and last_message_at bump.
        let after = repo.find(&thread.id).await.unwrap().unwrap();
        assert_eq!(after.message_count, 1);
        assert_eq!(after.last_message_at, Some(msg.created_at));

        // List is chronological.
        let messages = repo.list_messages(&thread.id, None).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, stored.id);

        // since cursor excludes the seen message.
        let empty = repo
            .list_messages(&thread.id, Some(msg.created_at))
            .await
            .unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn idempotent_append_returns_existing_row() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteThreadRepository::new(isle);

        let thread = Thread::new(ThreadAnchor::AppGlobal, "Inbox", now()).unwrap();
        repo.save(&thread).await.unwrap();

        let msg = Message::new(
            thread.id,
            Author::ClaudeCode,
            Role::Action,
            "ran build",
            vec![],
            now(),
        )
        .unwrap();
        let first = repo.append_message(&msg, Some("cc-1")).await.unwrap();

        let second_input = Message::new(
            thread.id,
            Author::ClaudeCode,
            Role::Action,
            "different body would be ignored",
            vec![],
            now(),
        )
        .unwrap();
        let second = repo
            .append_message(&second_input, Some("cc-1"))
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.body, "ran build");

        let after = repo.find(&thread.id).await.unwrap().unwrap();
        assert_eq!(after.message_count, 1);
    }

    #[tokio::test]
    async fn archive_skips_default_list_include_flag_surfaces_row() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteThreadRepository::new(isle);

        let mut thread = Thread::new(ThreadAnchor::AppGlobal, "Inbox", now()).unwrap();
        repo.save(&thread).await.unwrap();

        thread.archived = true;
        thread.updated_at = now();
        repo.save(&thread).await.unwrap();

        let visible = repo
            .list_by_anchor(&ThreadAnchor::AppGlobal, false)
            .await
            .unwrap();
        assert!(visible.is_empty());
        let all = repo
            .list_by_anchor(&ThreadAnchor::AppGlobal, true)
            .await
            .unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].archived);
    }

    #[tokio::test]
    async fn delete_thread_cascades_messages_and_trigger_decrement_after_message_delete() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteThreadRepository::new(isle);

        let thread = Thread::new(ThreadAnchor::AppGlobal, "Inbox", now()).unwrap();
        repo.save(&thread).await.unwrap();
        let m1 = Message::new(thread.id, Author::Human, Role::Note, "a", vec![], now()).unwrap();
        repo.append_message(&m1, None).await.unwrap();
        let m2 = Message::new(thread.id, Author::Human, Role::Note, "b", vec![], now()).unwrap();
        repo.append_message(&m2, None).await.unwrap();
        assert_eq!(
            repo.find(&thread.id).await.unwrap().unwrap().message_count,
            2
        );

        repo.delete_message(&m1.id).await.unwrap();
        assert_eq!(
            repo.find(&thread.id).await.unwrap().unwrap().message_count,
            1
        );

        repo.delete(&thread.id).await.unwrap();
        assert!(repo.find(&thread.id).await.unwrap().is_none());
        let messages = repo.list_messages(&thread.id, None).await.unwrap();
        assert!(messages.is_empty());
    }

    // Regression: `trg_message_after_delete` must not rewind
    // `thread.updated_at` to the deleted message's post timestamp.
    // Before the fix the trigger set `updated_at = OLD.created_at`,
    // which is older than the current `updated_at` when the deletion
    // itself is what advances "now".
    #[tokio::test]
    async fn delete_message_does_not_regress_updated_at() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteThreadRepository::new(isle);

        let thread = Thread::new(ThreadAnchor::AppGlobal, "Inbox", now()).unwrap();
        repo.save(&thread).await.unwrap();
        let msg = Message::new(thread.id, Author::Human, Role::Note, "old", vec![], now()).unwrap();
        repo.append_message(&msg, None).await.unwrap();
        let before_delete = repo.find(&thread.id).await.unwrap().unwrap().updated_at;

        repo.delete_message(&msg.id).await.unwrap();

        let after_delete = repo.find(&thread.id).await.unwrap().unwrap().updated_at;
        assert!(
            after_delete >= before_delete,
            "updated_at must not regress on message delete (before={before_delete}, after={after_delete})"
        );
    }

    // Persona deletion cascades to its authored messages (chosen
    // over ON DELETE SET NULL — the CHECK constraint on
    // `author_kind = 'persona' <=> author_persona_id IS NOT NULL`
    // would otherwise trip on SET NULL and silently abort the
    // parent DELETE persona). Human/claude_code messages on the
    // same thread stay put.
    #[tokio::test]
    async fn delete_persona_cascades_authored_messages_only() {
        use rusqlite::params;
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteThreadRepository::new(isle.clone());

        let persona = uuid::Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, accent_color, display_order, archived,
                                      created_at, updated_at)
                 VALUES (?1, 'ada', NULL, 0, 0, 0, 0)",
                params![persona],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let pid = PersonaId::from_uuid(persona);

        let thread = Thread::new(ThreadAnchor::AppGlobal, "Inbox", now()).unwrap();
        repo.save(&thread).await.unwrap();

        let human_msg = Message::new(
            thread.id,
            Author::Human,
            Role::Note,
            "human note",
            vec![],
            now(),
        )
        .unwrap();
        repo.append_message(&human_msg, None).await.unwrap();

        let persona_msg = Message::new(
            thread.id,
            Author::Persona(pid),
            Role::Note,
            "persona note",
            vec![],
            now(),
        )
        .unwrap();
        repo.append_message(&persona_msg, None).await.unwrap();
        assert_eq!(
            repo.find(&thread.id).await.unwrap().unwrap().message_count,
            2
        );

        // Persona deletion must succeed (would silently abort under
        // ON DELETE SET NULL because of the CHECK on author_persona_id).
        isle.call(move |conn| {
            conn.execute("DELETE FROM persona WHERE id = ?1", params![persona])?;
            Ok(())
        })
        .await
        .unwrap();

        // Only the persona-authored message is gone; human_msg stays.
        let remaining = repo.list_messages(&thread.id, None).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, human_msg.id);
    }
}
