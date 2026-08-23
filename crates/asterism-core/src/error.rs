//! `DomainError` — the innermost error type shared across every layer of
//! `asterism-core`.
//!
//! Outer layers (the Tauri `UiError`, the HTTP `ApiError`, MCP tool errors)
//! convert from this enum. Infrastructure-level failures are collapsed into
//! `Infra` so that domain vocabulary (`NotFound` / `Duplicate` / `Validation`
//! / `Conflict`) stays clean.
//!
//! # Which variant a refusal belongs to
//!
//! Stated here because it was not stated anywhere, and the cost of that
//! was fifty-eight call sites each deciding for themselves — thirty-nine
//! of them inside a SQLite repository, which is not a layer that can
//! answer an API question. The four definitions below are the rule.
//!
//! A repository does not apply them. `asterism-infra` has a
//! `StoreFault` type of its own — seven cases naming what storage did,
//! with one hand-written conversion into this enum whose doc is the
//! table those seven land on. That crate is not published, so there is
//! nothing to link; `cargo doc` and the source both have it under
//! `asterism_infra::fault`.
//!
//! - **[`Validation`](DomainError::Validation)** — the request cannot be
//!   satisfied as written, and that is decidable from the request plus
//!   the identity of what it addressed. Nothing changes on its own. A
//!   blank name; an outcome that is neither word; a directory moved
//!   into itself; a reply naming a message of another conversation.
//! - **[`NotFound`](DomainError::NotFound)** and its two named
//!   siblings — what was addressed is not there.
//! - **[`Conflict`](DomainError::Conflict)** — the request is
//!   well-formed and *would* be satisfiable. What refuses it is the
//!   current state, and that state is a thing that changes. A name
//!   already taken; a lost optimistic lock; work somebody else already
//!   ended; a precondition held by another row.
//! - **[`Infra`](DomainError::Infra)** — the store handed back
//!   something that could not have been written, or the machine
//!   underneath failed. The caller is not involved.
//!
//! The line between the first and the third is the one that gets drawn
//! wrongly, and "would a different request work" is not it — a blank
//! name passes that test and is plainly a `Validation`. Ask instead
//! whether *the state* is what refuses, and whether that state is
//! something that changes.

use crate::domain::value::{AssetId, PackId, PersonaId};

/// Errors raised by the domain layer.
///
/// Convention: `asterism-infra` names what storage did in its own
/// vocabulary and converts once, at one edge; application services
/// return the specific domain variants directly. Which variant a
/// refusal belongs to is the module doc's four definitions.
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

    /// The token a caller branches on, or nothing.
    ///
    /// **Here rather than in each transport.** Every surface that
    /// reports an error has to answer the same question — which of
    /// these carries retry advice, and what is it. Answered once, the
    /// three transports cannot disagree about a promise clients act on.
    ///
    /// The alternative was a helper in `asterism-server` that the
    /// desktop would call. Nothing blocked it — `asterism-ui` already
    /// depends on that crate — and it keeps a transport-facing word
    /// out of the domain type. It was not taken because the answer is
    /// read off the variant and its kind, both defined right here: a
    /// helper elsewhere would match on this type to answer a question
    /// about this type, and the two could then be edited apart. The
    /// price of the choice made is the one to weigh against that:
    /// `DomainError` now knows a token clients branch on.
    ///
    /// Only a conflict has one. A `404` and a `400` each want one thing
    /// from a caller, so a token for them would be a field with a
    /// single value; and an `Infra` is not the caller's business at
    /// all. [`DuplicatePersona`](Self::DuplicatePersona) is the
    /// exception that proves the shape: it is a uniqueness collision in
    /// everything but representation, with nowhere of its own to carry
    /// a kind, so it answers [`Clashes`](ConflictKind::Clashes) from
    /// here.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Conflict { kind, .. } => Some(kind.as_str()),
            Self::DuplicatePersona(_) => Some(ConflictKind::Clashes.as_str()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every refusal that carries retry advice, and every one that
    /// does not.
    ///
    /// One assertion per variant, because this is what three
    /// transports now read instead of each deciding. A variant added
    /// without a row here is a variant whose token nobody chose.
    #[test]
    fn only_a_conflict_carries_a_reason() {
        for (kind, token) in [
            (ConflictKind::Raced, "raced"),
            (ConflictKind::Blocked, "blocked"),
            (ConflictKind::Settled, "settled"),
            (ConflictKind::Clashes, "clashes"),
        ] {
            assert_eq!(DomainError::conflict(kind, "x").reason(), Some(token));
        }

        // The one that is a uniqueness collision without a kind to
        // carry, and so has its answer spelled in `reason` rather than
        // at the three call sites that used to spell it.
        assert_eq!(
            DomainError::DuplicatePersona(PackId::new("pack").expect("a literal slug")).reason(),
            Some("clashes"),
            "a pack id already registered is a value the caller has to change"
        );

        assert_eq!(DomainError::Validation("x".into()).reason(), None);
        assert_eq!(DomainError::not_found("session", "s-1").reason(), None);
        assert_eq!(
            DomainError::Infra(anyhow::anyhow!("x")).reason(),
            None,
            "not the caller\'s business, so there is nothing to advise"
        );
    }
}
