//! `Thread` — the app-level container that collects `Message`s from
//! both the UI (human) and the HTTP channel (Claude Code / agents).
//!
//! A Thread is a lightweight anchor + a chronological list of
//! `Message`s. Author-agnostic: the same row receives human notes
//! and remote agent writes so the "Context → Action" transition stays
//! visually contiguous. The anchor tells *what* the Thread hangs off
//! of; `AppGlobal` is the Home-tab default, other variants attach the
//! Thread to a `Snapshot`, a query `Group`, or a `Card` (Phase 3+).
//!
//! # Invariants
//!
//! - `title` is non-empty (trimmed). Callers pick the label; the
//!   UI-suggested default for the AppGlobal case is `"Inbox"`.
//! - `Message::body` is non-empty (trimmed) — a blank submit is a
//!   caller bug, not a stored row.
//! - `Message::refs` may be empty. Duplicates and ordering are
//!   preserved verbatim (chips render in declared order).
//! - `Thread.message_count` and `Thread.last_message_at` are
//!   projections the adapter maintains from the `messages` table via
//!   a trigger — the domain entity carries them for read-side
//!   convenience but does not compute them.
//!
//! # Design notes
//!
//! - **Model unification**: UI and HTTP both hit
//!   `append_message`. Author identity is a wire-level detail
//!   (`author_kind` + `author_name`) that the server side accepts
//!   verbatim from a caller-supplied header; server never *infers*
//!   the author (spoof-prevention is a later concern; the field is
//!   just typed data for now).
//! - **Append-only Messages**: no `edit_message`
//!   verb. Misfires are corrected with `delete_message` and a fresh
//!   append.
//! - **`EntityRef` chips** are wired at the domain
//!   layer already so the Phase 4 UI does not need a schema
//!   migration — the `refs_json` column exists from day one.

use chrono::{DateTime, Utc};

use crate::domain::value::{AssetId, GroupId, MessageId, PersonaId, SnapshotId, ThreadId};
use crate::error::DomainError;

/// What a `Thread` is attached to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadAnchor {
    /// App-level, not attached to any specific entity. The Home-tab
    /// Inbox lives here.
    AppGlobal,
    /// Anchored to a `Snapshot` — Threads that discuss / annotate
    /// one immutable freeze.
    Snapshot(SnapshotId),
    /// Anchored to a query `Group`.
    QueryGroup(GroupId),
    /// Anchored to a `Card` (asset) — a per-card conversation lane.
    /// Sibling of `AssetComment` but through the Thread primitive;
    /// the two coexist in P1 (no data migration between them).
    Card(AssetId),
}

impl ThreadAnchor {
    /// Wire slug (`"app_global"` / `"snapshot"` / `"query_group"` /
    /// `"card"`).
    pub fn kind_slug(&self) -> &'static str {
        match self {
            Self::AppGlobal => "app_global",
            Self::Snapshot(_) => "snapshot",
            Self::QueryGroup(_) => "query_group",
            Self::Card(_) => "card",
        }
    }

    /// Anchor id when present (`None` for `AppGlobal`).
    pub fn anchor_id(&self) -> Option<String> {
        match self {
            Self::AppGlobal => None,
            Self::Snapshot(id) => Some(id.to_string()),
            Self::QueryGroup(id) => Some(id.to_string()),
            Self::Card(id) => Some(id.to_string()),
        }
    }
}

/// Who wrote a `Message`.
///
/// Human = the person running Asterism. `ClaudeCode` = the CLI /
/// harness talking through the HTTP surface (default remote author).
/// `Agent(name)` = a named worker / persona-tagged agent. The name
/// stays a free-form string — the domain does not enumerate agent
/// identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Author {
    /// The human running Asterism.
    Human,
    /// The Claude Code CLI / harness.
    ClaudeCode,
    /// A named agent worker or persona-tagged agent.
    Agent(String),
    /// A persona posting through the Thread surface. Kept separate
    /// from `Agent` so persona-owned metadata (avatar, theme) can be
    /// looked up without pattern-matching on a slug.
    Persona(PersonaId),
}

impl Author {
    /// Wire slug (`"human"` / `"claude_code"` / `"agent"` /
    /// `"persona"`).
    pub fn kind_slug(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::ClaudeCode => "claude_code",
            Self::Agent(_) => "agent",
            Self::Persona(_) => "persona",
        }
    }

    /// Agent name when the author is `Agent(name)`.
    pub fn agent_name(&self) -> Option<&str> {
        match self {
            Self::Agent(name) => Some(name.as_str()),
            _ => None,
        }
    }

    /// Persona id when the author is `Persona(id)`.
    pub fn persona_id(&self) -> Option<&PersonaId> {
        match self {
            Self::Persona(id) => Some(id),
            _ => None,
        }
    }
}

/// Semantic role of a `Message`.
///
/// - `Note` — plain thought / observation / discussion.
/// - `Action` — a record of an action the author performed (Claude
///   Code writes these after invoking a tool). Future: carry a
///   `action_result` JSON.
/// - `System` — pipeline-emitted event (thread created, snapshot
///   ready, job finished). Not authored by a human or an agent per
///   se; rendered with a neutral style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// Plain thought / observation / discussion.
    Note,
    /// Record of an action the author performed.
    Action,
    /// Pipeline-emitted event (not authored by a human or agent).
    System,
}

impl Role {
    /// Wire slug (`"note"` / `"action"` / `"system"`).
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Action => "action",
            Self::System => "system",
        }
    }

    /// Parses a wire slug. Rejects unknown values.
    pub fn from_slug(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "note" => Ok(Self::Note),
            "action" => Ok(Self::Action),
            "system" => Ok(Self::System),
            other => Err(DomainError::Validation(format!(
                "unknown message role: {other:?}"
            ))),
        }
    }
}

/// A reference chip embedded in a `Message` body (Phase 4 UI
/// affordance). The domain carries the enum today so the wire /
/// storage shape does not need a migration when the drag-drop
/// composer lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityRef {
    /// Reference to an asset (Card).
    Card(AssetId),
    /// Reference to a `Snapshot`.
    Snapshot(SnapshotId),
    /// Reference to a query `Group`.
    QueryGroup(GroupId),
}

impl EntityRef {
    /// Wire slug (`"card"` / `"snapshot"` / `"query_group"`).
    pub fn kind_slug(&self) -> &'static str {
        match self {
            Self::Card(_) => "card",
            Self::Snapshot(_) => "snapshot",
            Self::QueryGroup(_) => "query_group",
        }
    }

    /// Referenced entity id.
    pub fn ref_id(&self) -> String {
        match self {
            Self::Card(id) => id.to_string(),
            Self::Snapshot(id) => id.to_string(),
            Self::QueryGroup(id) => id.to_string(),
        }
    }
}

/// Thread container.
#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    /// Surrogate id (UUID v7).
    pub id: ThreadId,
    /// Display label. Non-empty (trimmed).
    pub title: String,
    /// What the Thread hangs off of.
    pub anchor: ThreadAnchor,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last mutation on either the Thread row or one of its messages
    /// (adapters stamp this).
    pub updated_at: DateTime<Utc>,
    /// Time of the most recent appended `Message`. `None` for an
    /// empty Thread.
    pub last_message_at: Option<DateTime<Utc>>,
    /// Number of messages currently attached.
    pub message_count: u32,
    /// Soft-hide flag. Archived threads are excluded from the
    /// default listing but not deleted.
    pub archived: bool,
}

impl Thread {
    /// Builds a fresh Thread. Rejects an empty (post-trim) title.
    pub fn new(
        anchor: ThreadAnchor,
        title: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let title = title.into();
        if title.trim().is_empty() {
            return Err(DomainError::Validation(
                "Thread title must not be empty".into(),
            ));
        }
        Ok(Self {
            id: ThreadId::new(),
            title,
            anchor,
            created_at: now,
            updated_at: now,
            last_message_at: None,
            message_count: 0,
            archived: false,
        })
    }
}

/// One appended entry in a `Thread`.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// Surrogate id (UUID v7).
    pub id: MessageId,
    /// Owning Thread.
    pub thread_id: ThreadId,
    /// Who wrote the message.
    pub author: Author,
    /// Semantic role.
    pub role: Role,
    /// Free-form body. Non-empty by invariant (post-trim). Rendered
    /// as markdown by the UI.
    pub body: String,
    /// Reference chips embedded in the body (may be empty).
    pub refs: Vec<EntityRef>,
    /// Post timestamp.
    pub created_at: DateTime<Utc>,
}

impl Message {
    /// Builds a fresh Message. Rejects an empty (post-trim) body.
    pub fn new(
        thread_id: ThreadId,
        author: Author,
        role: Role,
        body: impl Into<String>,
        refs: Vec<EntityRef>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let body = body.into();
        if body.trim().is_empty() {
            return Err(DomainError::Validation(
                "Message body must not be empty".into(),
            ));
        }
        Ok(Self {
            id: MessageId::new(),
            thread_id,
            author,
            role,
            body,
            refs,
            created_at: now,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_rejects_empty_title() {
        let now = Utc::now();
        assert!(Thread::new(ThreadAnchor::AppGlobal, "", now).is_err());
        assert!(Thread::new(ThreadAnchor::AppGlobal, "   ", now).is_err());
        assert!(Thread::new(ThreadAnchor::AppGlobal, "Inbox", now).is_ok());
    }

    #[test]
    fn thread_starts_empty() {
        let now = Utc::now();
        let t = Thread::new(ThreadAnchor::AppGlobal, "Inbox", now).unwrap();
        assert_eq!(t.message_count, 0);
        assert!(t.last_message_at.is_none());
        assert!(!t.archived);
    }

    #[test]
    fn anchor_kind_slug_and_id() {
        assert_eq!(ThreadAnchor::AppGlobal.kind_slug(), "app_global");
        assert!(ThreadAnchor::AppGlobal.anchor_id().is_none());
        let sid = SnapshotId::new();
        let a = ThreadAnchor::Snapshot(sid);
        assert_eq!(a.kind_slug(), "snapshot");
        assert_eq!(a.anchor_id(), Some(sid.to_string()));
    }

    #[test]
    fn author_slugs() {
        assert_eq!(Author::Human.kind_slug(), "human");
        assert_eq!(Author::ClaudeCode.kind_slug(), "claude_code");
        let agent = Author::Agent("lint-bot".into());
        assert_eq!(agent.kind_slug(), "agent");
        assert_eq!(agent.agent_name(), Some("lint-bot"));
        let pid = PersonaId::new();
        let persona = Author::Persona(pid);
        assert_eq!(persona.kind_slug(), "persona");
        assert_eq!(persona.persona_id(), Some(&pid));
    }

    #[test]
    fn role_roundtrip() {
        for r in [Role::Note, Role::Action, Role::System] {
            assert_eq!(Role::from_slug(r.slug()).unwrap(), r);
        }
        assert!(Role::from_slug("bogus").is_err());
    }

    #[test]
    fn message_rejects_empty_body() {
        let tid = ThreadId::new();
        let now = Utc::now();
        assert!(Message::new(tid, Author::Human, Role::Note, "", vec![], now).is_err());
        assert!(Message::new(tid, Author::Human, Role::Note, "   ", vec![], now).is_err());
        assert!(Message::new(tid, Author::Human, Role::Note, "hi", vec![], now).is_ok());
    }

    #[test]
    fn message_preserves_refs() {
        let tid = ThreadId::new();
        let now = Utc::now();
        let card = EntityRef::Card(AssetId::new());
        let snap = EntityRef::Snapshot(SnapshotId::new());
        let m = Message::new(
            tid,
            Author::Human,
            Role::Note,
            "see these",
            vec![card.clone(), snap.clone()],
            now,
        )
        .unwrap();
        assert_eq!(m.refs, vec![card, snap]);
    }
}
