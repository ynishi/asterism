//! Instance identity — the referent behind
//! [`Author::Owner`](crate::domain::attribution::Author::Owner).
//!
//! One profile database is one Asterism instance, and one instance has
//! exactly one owner. Before this record existed, `Owner` was a variant
//! with nothing behind it: the write path could stamp it, but nothing
//! could answer "which owner". A single row fixes that — `Owner` is an
//! indirect reference to it, the same way a foreign key refers rather
//! than copies.
//!
//! **Co-ownership is not modelled.** Sharing adds subjects
//! ([`Visibility::Restricted`](crate::domain::value::Visibility::Restricted)),
//! it does not add owners; an instance with two owners would make
//! "whose instance is this" unanswerable at exactly the moment the
//! answer starts to matter.
//!
//! `owner_subject` is `None` while Asterism runs locally: there is no
//! authentication, so no token names the person at the keyboard, and
//! inventing one would be a value where there is a question. It is
//! bound once, when authentication arrives, and from then on `Owner`
//! resolves to a name that lives in the same namespace as sharing
//! subjects (see the [`attribution`](crate::domain::attribution) module
//! docs).

use chrono::{DateTime, Utc};

use crate::domain::value::InstanceId;

/// The identity record of this Asterism instance (the `instance`
/// table's single row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceIdentity {
    /// Identifier minted when the profile was first migrated. Stable
    /// for the life of the profile.
    pub id: InstanceId,
    /// When the identity was minted.
    pub created_at: DateTime<Utc>,
    /// Subject token naming the owner, or `None` while unbound.
    ///
    /// Unbound is the local-only state, not a missing value to be
    /// filled in with a guess. Binding is a one-time act of
    /// authentication; see [`InstanceIdentity::resolve_owner`] for what
    /// the two states mean to a reader.
    pub owner_subject: Option<String>,
}

impl InstanceIdentity {
    /// Resolves [`Author::Owner`](crate::domain::attribution::Author::Owner)
    /// against this record.
    ///
    /// Returns [`OwnerResolution::Unresolved`] while `owner_subject` is
    /// unbound. That is a real answer, not a failure: locally, "the
    /// owner" is a complete reference on its own, and the caller that
    /// wanted a token has to keep treating it as a relative reference
    /// rather than substituting one.
    pub fn resolve_owner(&self) -> OwnerResolution<'_> {
        match self.owner_subject.as_deref() {
            Some(subject) => OwnerResolution::Resolved(subject),
            None => OwnerResolution::Unresolved,
        }
    }
}

/// What `Author::Owner` resolves to on this instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerResolution<'a> {
    /// No subject is bound yet — the owner is only nameable relative to
    /// this instance.
    Unresolved,
    /// The bound subject token, in the same namespace as sharing
    /// subjects.
    Resolved(&'a str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn identity(owner_subject: Option<&str>) -> InstanceIdentity {
        InstanceIdentity {
            id: InstanceId::new(),
            created_at: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
            owner_subject: owner_subject.map(str::to_string),
        }
    }

    #[test]
    fn an_unbound_instance_resolves_the_owner_to_nothing() {
        // The local state. A caller asking "which subject is the owner"
        // must get "nobody has said", not a fabricated token — the same
        // discipline the attribution columns keep about `None`.
        assert_eq!(identity(None).resolve_owner(), OwnerResolution::Unresolved);
    }

    #[test]
    fn a_bound_instance_resolves_the_owner_to_its_subject() {
        assert_eq!(
            identity(Some("alice")).resolve_owner(),
            OwnerResolution::Resolved("alice")
        );
    }
}
