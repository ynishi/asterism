//! What storage did, in storage's own words.
//!
//! A repository knows what the database answered: a unique index
//! rejected the row, a predicate matched nothing, a row would not
//! decode. It does not know whether the caller should try again, which
//! is what [`DomainError::Conflict`]'s kind promises — and that promise
//! is the reason this module exists.
//!
//! # What went wrong without it
//!
//! `DomainError`'s four shared variants had no written rule for which
//! one a refusal belonged to, so the choice was made at every call site
//! that raised one: fifty-eight of them, thirty-nine inside this crate.
//! A SQLite repository was answering an API question. Several answers
//! were wrong, and the wrongness only became visible when `ConflictKind`
//! turned "some vague 409" into advice a client acts on.
//!
//! # The shape
//!
//! ```text
//!   repository ──► StoreFault ──► DomainError ──► 400 / 404 / 409 / 500
//!                       │              │
//!         what storage ─┘              └─ what it means, decided once,
//!         did. Seven cases,               in the `From` impl below and
//!         no judgement.                   nowhere else.
//! ```
//!
//! Each case has exactly one destination, so the mapping is a table
//! rather than an argument. That is deliberate: the repository picks by
//! what happened, which it can see, and the meaning is written down in
//! one place a reviewer can read in full before adding the next
//! refusal. `asterism-core`'s `error` module doc holds the four
//! definitions this table implements; `domain::forge::model::error` is
//! the same structure one layer up, where the forge's own vocabulary
//! meets the shared one at a single hand-written edge.
//!
//! # Why the conversion is written out
//!
//! `thiserror`'s `#[from]` derives a conversion that carries a value
//! across unchanged. This one reads which case it has and picks a
//! different destination for each, which no derive expresses — and
//! that is the point rather than a limitation. The mapping *is* the
//! specification, so it wants to be read, not generated.

use asterism_core::error::{ConflictKind, DomainError};

/// A refusal in storage's vocabulary.
///
/// Which variant each becomes is
/// [the conversion below](#impl-From%3CStoreFault%3E-for-DomainError).
///
/// **What this crate stopped naming directly is `Conflict`.** It still
/// names `Validation`, `NotFound` and `Infra` in plenty of places, and
/// that is not an oversight: those three say what happened, and only
/// `Conflict` carries [`ConflictKind`] — advice about whether to ask
/// again, which is the thing a repository has no way to judge.
/// `tests/store_fault_is_the_only_door.rs` enforces exactly that much
/// and says so.
///
/// The cases below cover more than conflicts anyway, because a
/// repository choosing between them should not have to notice where
/// the line falls: [`Absent`](Self::Absent),
/// [`CorruptRow`](Self::CorruptRow) and [`Impossible`](Self::Impossible)
/// land outside `Conflict`, and a call site reaching for one of those
/// gets the right answer without knowing that.
#[derive(Debug, Clone, thiserror::Error)]
pub enum StoreFault {
    /// What was addressed is not there.
    ///
    /// `entity` is the label a caller reads (`"session"`, `"line"`), and
    /// nothing branches on it.
    #[error("{entity} not found: {id}")]
    Absent {
        /// Which kind of thing.
        entity: &'static str,
        /// Which one, rendered.
        id: String,
    },

    /// A row is here and cannot be made sense of.
    ///
    /// The caller asked to read, its request was fine, and what came
    /// back could not have been written. Nothing it does differently
    /// helps, which is why this is the one case that is not the
    /// caller's business at all.
    #[error("{0}")]
    CorruptRow(String),

    /// A uniqueness rule rejected it.
    ///
    /// The request is well-formed; the value it carries is already
    /// spoken for. A different value works, which is the whole of what
    /// separates this from [`Impossible`](Self::Impossible) — there,
    /// no value works.
    #[error("{what} is already taken: {value}")]
    UniqueViolation {
        /// What is unique (`"a dir name at this level"`).
        what: &'static str,
        /// The value that was taken.
        value: String,
    },

    /// Something is not yet in a state that allows this, and the
    /// message names what to change.
    ///
    /// Usually another row — the members of a session, the work open
    /// against a line. Sometimes the addressed row itself, which is
    /// what "trash it before purging" is: the thing standing in the way
    /// is its own standing, and the shape is identical. What matters is
    /// that a state change lets the *same* request through, not whose
    /// state changes.
    ///
    /// `remedy` is required, because a caller told only "refused" has
    /// to guess. The repository knows it — it ran the query that found
    /// what was in the way — while it does not know whether that makes
    /// the refusal a conflict. It does: state that changes is what
    /// refuses, so this becomes [`Blocked`](ConflictKind::Blocked).
    ///
    /// The remedy has to name something a caller can actually reach.
    /// Two of these named operations the HTTP surface does not spell —
    /// "detach them first" when nothing detaches, and "drop those
    /// lines" for a three-step sequence — which is a promise this field
    /// makes and those sentences broke.
    #[error("{what}; {remedy}")]
    PreconditionUnmet {
        /// What is standing in the way.
        what: String,
        /// What to change so the same request goes through.
        remedy: &'static str,
    },

    /// The row moved between the read and the write.
    ///
    /// An optimistic lock losing, which is the one case where sending
    /// the identical request again is a reasonable thing for a client
    /// to do on its own.
    #[error("{0}")]
    StaleWrite(String),

    /// It is already decided, and deciding is not repeatable.
    ///
    /// Distinct from [`StaleWrite`](Self::StaleWrite) by what a caller
    /// should do: nothing. Work that has ended has ended; a suggestion
    /// already ruled on stays ruled on.
    #[error("{0}")]
    AlreadyDecided(String),

    /// The request contradicts itself against this data, and no state
    /// change makes it hold.
    ///
    /// A directory moved inside itself; a group containing itself; a
    /// reply naming a message of another conversation. Nothing is
    /// contended and nothing is racing — the caller addressed one thing
    /// and described another, so this reads as a `Validation` even
    /// though a query is what noticed.
    #[error("{0}")]
    Impossible(String),
}

impl StoreFault {
    /// Builds [`Absent`](Self::Absent) from any `Display` id.
    pub fn absent(entity: &'static str, id: impl std::fmt::Display) -> Self {
        Self::Absent {
            entity,
            id: id.to_string(),
        }
    }

    /// Builds [`UniqueViolation`](Self::UniqueViolation) from any
    /// `Display` value.
    pub fn taken(what: &'static str, value: impl std::fmt::Display) -> Self {
        Self::UniqueViolation {
            what,
            value: value.to_string(),
        }
    }

    /// Builds [`PreconditionUnmet`](Self::PreconditionUnmet).
    pub fn blocked_by(what: impl Into<String>, remedy: &'static str) -> Self {
        Self::PreconditionUnmet {
            what: what.into(),
            remedy,
        }
    }

    /// A value read out of a row, refused by the model that parses it.
    ///
    /// **The same `parse` serves two callers and they are not the same
    /// answer.** `AssetRole::parse` on something a caller sent is a
    /// `Validation`: fix the request. The identical call on a column is
    /// not — the request was fine, and a row holding a role this build
    /// has no name for is a row that could not have been written.
    /// Without this the stored side inherits the request side's answer,
    /// which is how eleven decode failures came to tell callers to fix
    /// requests that had nothing wrong with them.
    ///
    /// `asterism-core`'s `ForgeError::Unwritable` is the same idea one
    /// layer up, where reading a line replays the rules writing
    /// enforced.
    pub fn parsed<T>(what: &'static str, read: Result<T, DomainError>) -> Result<T, DomainError> {
        read.map_err(|refused| Self::CorruptRow(format!("a stored {what}: {refused}")).into())
    }
}

/// What each storage fault means to a caller.
///
/// **This table is the specification.** `asterism-core`'s `error`
/// module doc states the four definitions; this is the only place they
/// are applied to anything raised by this crate, and a new refusal is
/// settled by reading it rather than by copying a neighbour.
///
/// | storage said | the caller is told | why |
/// |---|---|---|
/// | [`Absent`](StoreFault::Absent) | `NotFound` → 404 | it is not there |
/// | [`CorruptRow`](StoreFault::CorruptRow) | `Infra` → 500 | not the caller's doing, and no request avoids it |
/// | [`UniqueViolation`](StoreFault::UniqueViolation) | `Conflict` / [`Clashes`](ConflictKind::Clashes) → 409 | existing state holds the value; another value works |
/// | [`PreconditionUnmet`](StoreFault::PreconditionUnmet) | `Conflict` / [`Blocked`](ConflictKind::Blocked) → 409 | a state change lets the same request through; the message names it |
/// | [`StaleWrite`](StoreFault::StaleWrite) | `Conflict` / [`Raced`](ConflictKind::Raced) → 409 | it moved underneath; the same request may win next time |
/// | [`AlreadyDecided`](StoreFault::AlreadyDecided) | `Conflict` / [`Settled`](ConflictKind::Settled) → 409 | decided; asking again finds it decided |
/// | [`Impossible`](StoreFault::Impossible) | `Validation` → 400 | the request does not hold against this data at any state |
///
/// The last row is the one that was got wrong before this existed. A
/// query noticing something does not make it a conflict: "a dir cannot
/// be moved into itself" is refused by the request, not by the state,
/// and answering `409` told a client to consider retrying a thing that
/// can never succeed.
///
/// Written out per variant rather than derived. `#[from]` carries a
/// value across unchanged; this reads the case to pick a destination,
/// and the reading is the part worth having in the source.
impl From<StoreFault> for DomainError {
    fn from(fault: StoreFault) -> Self {
        let said = fault.to_string();
        match fault {
            StoreFault::Absent { entity, id } => DomainError::NotFound { entity, id },
            StoreFault::CorruptRow(_) => DomainError::Infra(anyhow::anyhow!(said)),
            StoreFault::UniqueViolation { .. } => {
                DomainError::conflict(ConflictKind::Clashes, said)
            }
            StoreFault::PreconditionUnmet { .. } => {
                DomainError::conflict(ConflictKind::Blocked, said)
            }
            StoreFault::StaleWrite(_) => DomainError::conflict(ConflictKind::Raced, said),
            StoreFault::AlreadyDecided(_) => DomainError::conflict(ConflictKind::Settled, said),
            StoreFault::Impossible(_) => DomainError::Validation(said),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every case lands where the table says, and the message survives.
    ///
    /// One assertion per row, because the table is the specification
    /// and a specification with an untested row is a paragraph.
    #[test]
    fn each_storage_fault_lands_where_the_table_says() {
        let absent: DomainError = StoreFault::absent("session", "s-1").into();
        assert!(matches!(
            absent,
            DomainError::NotFound {
                entity: "session",
                ..
            }
        ));

        let corrupt: DomainError =
            StoreFault::CorruptRow("a stored act names nothing".into()).into();
        assert!(matches!(corrupt, DomainError::Infra(_)));

        let taken: DomainError = StoreFault::taken("a dir name at this level", "notes").into();
        assert!(matches!(
            taken,
            DomainError::Conflict {
                kind: ConflictKind::Clashes,
                ..
            }
        ));
        assert!(taken.to_string().contains("notes"), "{taken}");

        let blocked: DomainError =
            StoreFault::blocked_by("the dir is not empty", "move or delete its contents first")
                .into();
        assert!(matches!(
            blocked,
            DomainError::Conflict {
                kind: ConflictKind::Blocked,
                ..
            }
        ));
        assert!(
            blocked.to_string().contains("first"),
            "the remedy reaches the caller: {blocked}"
        );

        let stale: DomainError = StoreFault::StaleWrite("the line moved".into()).into();
        assert!(matches!(
            stale,
            DomainError::Conflict {
                kind: ConflictKind::Raced,
                ..
            }
        ));

        let decided: DomainError = StoreFault::AlreadyDecided("already ruled".into()).into();
        assert!(matches!(
            decided,
            DomainError::Conflict {
                kind: ConflictKind::Settled,
                ..
            }
        ));

        let impossible: DomainError =
            StoreFault::Impossible("a dir cannot be moved into itself".into()).into();
        assert!(
            matches!(impossible, DomainError::Validation(_)),
            "a query noticing it does not make it the state's doing: {impossible}"
        );
    }

    /// The two that read alike and answer differently.
    ///
    /// `UniqueViolation` and `Impossible` both refuse a request that
    /// names something, and the difference is whether *another* value
    /// would work. Naming them apart is the whole reason the caller can
    /// tell "pick another name" from "this can never hold".
    #[test]
    fn a_taken_value_and_an_impossible_one_are_not_the_same_answer() {
        let taken: DomainError = StoreFault::taken("a group name", "drafts").into();
        let impossible: DomainError =
            StoreFault::Impossible("a group cannot contain itself".into()).into();

        assert!(matches!(taken, DomainError::Conflict { .. }));
        assert!(matches!(impossible, DomainError::Validation(_)));
    }
}
