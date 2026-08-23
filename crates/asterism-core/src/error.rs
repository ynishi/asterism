//! `DomainError` — the innermost error type shared across every layer of
//! `asterism-core`.
//!
//! Outer layers (the Tauri `UiError`, the HTTP `ApiError`, MCP tool errors)
//! convert from this enum. Infrastructure-level failures are collapsed into
//! `Infra` so that domain vocabulary (`NotFound` / `Duplicate` / `Validation`
//! / `Conflict`) stays clean.

use crate::domain::value::{AssetId, PackId, PersonaId};

/// Errors raised by the domain layer.
///
/// Convention: `asterism-infra` converts adapter failures into
/// `anyhow::Error` and passes them through `Infra`; application services
/// return the specific domain variants directly.
#[derive(thiserror::Error, Debug)]
pub enum DomainError {
    /// No persona exists for the given id.
    #[error("persona not found: {0}")]
    PersonaNotFound(PersonaId),

    /// No asset exists for the given id.
    #[error("asset not found: {0}")]
    AssetNotFound(AssetId),

    /// No row of `entity` exists for `id` — the generic sibling of
    /// [`Self::PersonaNotFound`] / [`Self::AssetNotFound`] covering every
    /// other aggregate (session / modality / group / dir / snapshot /
    /// dispatch / thread / comment).
    ///
    /// Use this rather than `Validation` or `Conflict` for a missing
    /// target: the transport layers map it to `404`, so a client can tell
    /// "you asked for something that is not here" (`404`) apart from "your
    /// request was malformed" (`400`) and "the state fought back" (`409`).
    /// `entity` is a `&'static str` label (`"session"`, `"group"`, …) that
    /// only feeds the message; callers should not branch on it.
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Aggregate label used in the message (`"session"`, `"group"`, …).
        entity: &'static str,
        /// Identifier the caller asked for, rendered as text.
        id: String,
    },

    /// A persona with the same `pack_id` is already registered
    /// (invariant: `pack_id` is unique when present).
    #[error("persona already registered for pack id: {0}")]
    DuplicatePersona(PackId),

    /// Value-object / entity construction failed validation (empty slug,
    /// self-loop edge, out-of-range timestamp, and so on).
    #[error("validation error: {0}")]
    Validation(String),

    /// The state fought back: a unique constraint, a race, a
    /// precondition, an ending that already happened.
    ///
    /// **The kind is the point, and it is not a label.** Unlike
    /// [`NotFound::entity`](Self::NotFound), which only feeds the
    /// message, [`kind`](ConflictKind) is there precisely so a caller
    /// can branch on it — a client that retries every conflict loops
    /// forever on half of them, and one that retries none gives up on
    /// races it would win. Build these with [`Self::raced`],
    /// [`Self::blocked`], [`Self::settled`] or [`Self::clashes`].
    #[error("conflict: {message}")]
    Conflict {
        /// What the caller can do about it.
        kind: ConflictKind,
        /// What happened, for a person to read.
        message: String,
    },

    /// Failure originating from infrastructure (SQLite, filesystem, MCP
    /// client, etc.).
    #[error("infrastructure error: {0}")]
    Infra(#[from] anyhow::Error),
}

/// What a caller can do about a [`Conflict`](DomainError::Conflict).
///
/// Four answers, because there are four things a caller can do and no
/// fifth: send it again, do something else first, give up, or ask for
/// something different. Sorting a refusal into one of them is a
/// decision made where the state is known — the layer above cannot
/// recover it from a sentence, and the layer above is where the retry
/// loop lives.
///
/// Where the state is known is usually where the refusal is raised,
/// and not always: the forge raises one variant from two places, a
/// write losing a race and a read finding a row that could not have
/// been written, and it is
/// [`ForgeError::Unwritable`](crate::domain::forge::model::error::ForgeError::Unwritable)
/// that lets the conversion tell those apart rather than the raise
/// site.
///
/// This is not a status code by another name. Every one of these is a
/// `409` over HTTP, because every one of them is a conflict with the
/// current state; what differs is what happens next, which a status
/// code has never been able to say.
///
/// # Choosing one
///
/// **The kind follows from the state, and the message then has to
/// match it.** Not the other way round: an earlier draft of this said
/// a refusal is `Blocked` only when its message already names the way
/// through, which reads as a rule but is not one — it keys a promise a
/// client depends on to how somebody happened to word a sentence, so
/// improving a message would silently change the contract, and letting
/// one rot would silently break it. The wording is the part that can
/// be wrong and fixed. So: ask whether the same request works once
/// something else changes. If it does, it is `Blocked`, and a message
/// that does not say what to change is a message to fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Something landed between the read and the write. The same
    /// request may well win next time, and retrying is reasonable.
    Raced,
    /// The same request works once something else changes, and the
    /// message is required to say what.
    Blocked,
    /// It is already decided, and no amount of asking changes that.
    /// Retrying is always wrong.
    Settled,
    /// What the request asks for conflicts with something that is
    /// already there. The same request never works; a different one
    /// does.
    Clashes,
}

impl ConflictKind {
    /// A stable token for the wire.
    ///
    /// Named separately from the `Debug` rendering because this one is
    /// a promise: a client branches on it, so it changes only when the
    /// meaning does.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Raced => "raced",
            Self::Blocked => "blocked",
            Self::Settled => "settled",
            Self::Clashes => "clashes",
        }
    }

    /// Is sending the same request again worth doing?
    ///
    /// The question a retry loop actually asks, answered here rather
    /// than in each client. `Raced` yes, `Blocked` only after the
    /// message's precondition is met, and the other two never.
    pub fn worth_retrying(self) -> bool {
        matches!(self, Self::Raced)
    }
}

impl DomainError {
    /// Builds [`Self::NotFound`] from any `Display` id, so call sites do
    /// not repeat `id: id.to_string()`.
    pub fn not_found(entity: &'static str, id: impl std::fmt::Display) -> Self {
        Self::NotFound {
            entity,
            id: id.to_string(),
        }
    }

    /// A conflict of the given kind.
    pub fn conflict(kind: ConflictKind, message: impl Into<String>) -> Self {
        Self::Conflict {
            kind,
            message: message.into(),
        }
    }

    /// Something landed between the read and the write.
    /// See [`ConflictKind::Raced`].
    pub fn raced(message: impl Into<String>) -> Self {
        Self::conflict(ConflictKind::Raced, message)
    }

    /// A precondition the message names is not met.
    /// See [`ConflictKind::Blocked`].
    pub fn blocked(message: impl Into<String>) -> Self {
        Self::conflict(ConflictKind::Blocked, message)
    }

    /// Already decided. See [`ConflictKind::Settled`].
    pub fn settled(message: impl Into<String>) -> Self {
        Self::conflict(ConflictKind::Settled, message)
    }

    /// Conflicts with something already there.
    /// See [`ConflictKind::Clashes`].
    pub fn clashes(message: impl Into<String>) -> Self {
        Self::conflict(ConflictKind::Clashes, message)
    }
}
