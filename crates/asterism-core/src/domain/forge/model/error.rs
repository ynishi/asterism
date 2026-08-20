//! What the model refuses.
//!
//! The model has its own error type, and every refusal in this module
//! is one of its variants. Reaching for the shared error instead would
//! mean the forge states a rule in one place and names it in another's
//! vocabulary — and a rule named `Validation` is a rule nobody can
//! match on, so callers end up matching on message text.
//!
//! # It is folded once, at the edge
//!
//! [`DomainError`] is the shared vocabulary, and the conversion below
//! is the only place the model meets it. Adding a refusal means adding
//! a variant here and deciding, once, which shared kind it reads as —
//! not repeating that decision at every call site.
//!
//! # It only grows
//!
//! Every refusal the forge learns is added here. That is what makes
//! the set of ways this model can say no readable in one place, which
//! is the same reason a caller wants a typed error at all.

use thiserror::Error;

use crate::domain::forge::model::value::Name;

// SHARED KERNEL: `DomainError` is a boundary type. This module is the
// only one in the model that names it — everything else refuses in the
// forge's own vocabulary and converts at this edge.
use crate::error::DomainError;

/// A refusal the model makes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ForgeError {
    /// A name was blank, or became blank once trimmed.
    #[error("a name must not be blank")]
    BlankName,

    /// A row stated no axis at all — nothing is being said about the
    /// entry, so it does not belong in a table.
    #[error("a row states no axis — nothing is being said about the entry")]
    EmptyRow,

    /// A row took an entry off the line while also naming or filling
    /// it, which is a second spelling of a state that already has one.
    #[error(
        "a row takes an entry off the line and moves another axis — a removing row carries only \
         existence, so that a table stays a description of what its change point did"
    )]
    RemovalMovesAnotherAxis,

    /// A table carried no rows — a line advancing to say nothing.
    #[error("a table has at least one row — a line does not move to say nothing")]
    EmptyTable,

    /// A change point named something other than the head as its
    /// parent, which would fork the history.
    #[error(
        "a change point lands on the head — this one names another node as its parent, and a \
         history does not fork"
    )]
    NotOnHead,

    /// Landing would leave two entries on the line answering to the
    /// same name.
    #[error("two entries would be on the line under the name {0:?}")]
    NameTaken(Name),
}

impl From<ForgeError> for DomainError {
    /// Reads a refusal in the shared vocabulary.
    ///
    /// [`NotOnHead`](ForgeError::NotOnHead) and
    /// [`NameTaken`](ForgeError::NameTaken) are the state fighting
    /// back — the caller was not wrong, the line was somewhere else —
    /// so they read as conflicts. The rest are malformed input.
    fn from(error: ForgeError) -> Self {
        let message = error.to_string();
        match error {
            ForgeError::NotOnHead | ForgeError::NameTaken(_) => DomainError::Conflict(message),
            ForgeError::BlankName
            | ForgeError::EmptyRow
            | ForgeError::RemovalMovesAnotherAxis
            | ForgeError::EmptyTable => DomainError::Validation(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_that_moved_under_the_caller_reads_as_a_conflict() {
        let shared: DomainError = ForgeError::NotOnHead.into();

        assert!(matches!(shared, DomainError::Conflict(_)));
    }

    #[test]
    fn a_malformed_row_reads_as_validation() {
        let shared: DomainError = ForgeError::EmptyRow.into();

        assert!(matches!(shared, DomainError::Validation(_)));
    }

    #[test]
    fn a_name_already_answered_to_reads_as_a_conflict() {
        let taken = Name::new("key visual").unwrap();
        let shared: DomainError = ForgeError::NameTaken(taken.clone()).into();

        assert!(matches!(shared, DomainError::Conflict(_)), "{shared}");
        assert!(
            shared.to_string().contains(taken.as_str()),
            "the name a caller has to change is in the message: {shared}"
        );
    }

    #[test]
    fn the_message_survives_the_fold() {
        let refusal = ForgeError::BlankName;
        let message = refusal.to_string();
        let shared: DomainError = refusal.into();

        assert!(shared.to_string().contains(&message));
    }
}
