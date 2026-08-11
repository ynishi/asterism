//! `AssetComment` — a thread of short notes attached to an Asset.
//!
//! Comments are Asset-first: an Asset is the aggregate root, comments
//! are child entries the User or a Persona can post. The intent is
//! deliberately smaller than a full chat surface — one Asset, a
//! chronological list of short bodies, each carrying an author kind
//! and (for Persona authors) a persona reference.
//!
//! Design notes:
//!
//! - **Two author kinds**: `User` (the human running Asterism) and
//!   `Persona` (an AI persona registered in the vault). We do not
//!   model a full identity table for the User side — the vault is
//!   single-user by design; every `User` post is "me". A Persona
//!   comment carries the persona id so downstream UI can render the
//!   author avatar and colour without a follow-up lookup.
//! - **Body is free-form text** (no markdown-only guard). The UI
//!   renders it as plain text with newlines preserved; a future
//!   markdown pass can layer on top without a schema change.
//! - **`edited_at`** is `None` for the original post and stamped on
//!   every `edit` — the UI reveals a "(edited)" marker when it's set
//!   without exposing the raw diff.

use chrono::{DateTime, Utc};

use crate::domain::value::{AssetCommentId, AssetId, PersonaId};
use crate::error::DomainError;

/// Who wrote a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentAuthor {
    /// The human running Asterism.
    User,
    /// One of the vault's Personas.
    Persona {
        /// Id of the Persona that authored the comment.
        persona_id: PersonaId,
    },
    /// A Persona that has since been purged. The body outlives its
    /// author: the comment is prose somebody wrote, and purging the
    /// writer is not a claim that what they wrote never happened.
    ///
    /// **Not constructible by a caller.** It is what the store reads
    /// back after `ON DELETE SET NULL` cleared the id (schema V68), and
    /// the only way to reach it — posting requires a live
    /// [`PersonaId`], so a comment can *become* orphaned but cannot be
    /// posted orphaned.
    ///
    /// The identity is gone rather than hidden: which Persona wrote it
    /// is not recoverable, because the row that answered that is the
    /// row the User deleted.
    DeletedPersona,
}

impl CommentAuthor {
    /// Slug used on the wire (`"user"` / `"persona"`).
    ///
    /// [`DeletedPersona`](Self::DeletedPersona) answers `"persona"` —
    /// the slug names *what kind of author wrote this*, and that a
    /// Persona wrote it stays true after the Persona is gone. The
    /// absence is carried by [`persona_id`](Self::persona_id) going
    /// `None`, which is the same pair the row and the wire already use;
    /// a third slug would be a new vocabulary word every consumer would
    /// have to learn in order to render the same sentence.
    pub fn kind_slug(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Persona { .. } | Self::DeletedPersona => "persona",
        }
    }

    /// Persona id when the author is a Persona that still exists.
    pub fn persona_id(&self) -> Option<&PersonaId> {
        match self {
            Self::User | Self::DeletedPersona => None,
            Self::Persona { persona_id } => Some(persona_id),
        }
    }
}

/// A single comment on an Asset.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetComment {
    /// Surrogate id (UUID v7 — chronological on the ordinary
    /// timeline).
    pub id: AssetCommentId,
    /// Asset the comment is attached to.
    pub asset_id: AssetId,
    /// Author (User or a specific Persona).
    pub author: CommentAuthor,
    /// Free-form body. Empty strings are rejected at construction
    /// time (`new`) — the UI silently discards an empty submit.
    pub body: String,
    /// When the comment was posted.
    pub created_at: DateTime<Utc>,
    /// When the comment was last edited. `None` for original posts
    /// that have never been touched.
    pub edited_at: Option<DateTime<Utc>>,
}

impl AssetComment {
    /// Builds a new comment. Rejects an empty (post-trim) body.
    pub fn new(
        asset_id: AssetId,
        author: CommentAuthor,
        body: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let body = body.into();
        if body.trim().is_empty() {
            return Err(DomainError::Validation(
                "AssetComment body must not be empty".into(),
            ));
        }
        Ok(Self {
            id: AssetCommentId::new(),
            asset_id,
            author,
            body,
            created_at: now,
            edited_at: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_body() {
        let asset = AssetId::new();
        let now = Utc::now();
        assert!(AssetComment::new(asset, CommentAuthor::User, "", now).is_err());
        assert!(AssetComment::new(asset, CommentAuthor::User, "   ", now).is_err());
        assert!(AssetComment::new(asset, CommentAuthor::User, "hi", now).is_ok());
    }

    #[test]
    fn author_slug_and_persona_id() {
        let user = CommentAuthor::User;
        assert_eq!(user.kind_slug(), "user");
        assert!(user.persona_id().is_none());
        let pid = PersonaId::new();
        let persona = CommentAuthor::Persona { persona_id: pid };
        assert_eq!(persona.kind_slug(), "persona");
        assert_eq!(persona.persona_id(), Some(&pid));
    }
}
