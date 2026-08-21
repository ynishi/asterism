//! The values the model is made of.
//!
//! Nothing here knows how it is stored. A value that cannot be written
//! down without a column is not a value, and every rule these carry is
//! one the model states.
//!
//! Two of them are worth reading before the rest of the module:
//!
//! **[`Content`] is the only reference the forge holds downward.** It
//! wraps an asset id behind a private field, which is what lets every
//! other type in the model carry a reference into the layer below
//! without naming what it refers to. The forge compares content and
//! moves it around; reaching the thing itself is the boundary's
//! business, and keeping the vocabulary out of the model is what stops
//! the two sides from growing into each other.
//!
//! **[`Name`] promises one thing and refuses to promise more.** It is
//! trimmed and never blank, and two names match when their trimmed
//! forms match exactly — nothing else is normalised, since names are
//! chosen by people who can see the ones already there. Where a name
//! has to be unique is deliberately absent: that needs an owner to
//! answer, and the owner is outside the forge.
//!
//! The ids are surrogate and minted, never derived from content.
//! [`EntryId`] in particular is minted by work that has changed nothing
//! yet, which is what lets a later round point at what an earlier one
//! proposed.

use uuid::Uuid;

use crate::domain::forge::model::error::ForgeError;
// SHARED KERNEL: `AssetId` is a boundary type — the third crate both
// sides depend on, and neither owns. Grep `SHARED KERNEL` for every
// edge out of this module.
use crate::domain::value::AssetId;

// SHARED KERNEL (candidate): `define_uuid_id!` is not on the boundary
// list, and this is the only edge out of the model that is not. It
// shapes nothing — it is how an id newtype is spelled — so it moves
// with the split rather than before it.
use crate::domain::value::define_uuid_id;

define_uuid_id!(
    /// Surrogate id for an `Entry` — the thing a line names, minted
    /// once and never re-minted.
    ///
    /// An operation mints it on the spot, before the canonical history
    /// has heard of it, which is what lets a later round point at what
    /// an earlier one proposed. Whether the id is *on* the line is
    /// derived from the history and stored nowhere.
    EntryId
);

define_uuid_id!(
    /// Surrogate id for a node of a line's history — its genesis, or
    /// one change point.
    ///
    /// The chain is what orders them, so the id answers "which node",
    /// never "which came first".
    ChangePointId
);

define_uuid_id!(
    /// Surrogate id for a `Line` — one repository.
    LineId
);

define_uuid_id!(
    /// Surrogate id for a node of a work log — where it opened, one
    /// pass at it, or where it ended.
    ///
    /// As with a history, the chain is what orders them, so the id
    /// answers "which node" and never "which came first".
    NodeId
);

define_uuid_id!(
    /// Surrogate id for a `Pursuit` — one line of work.
    ///
    /// Declared here because a change point names the work it came out
    /// of, and that is all this module needs to know about one. The
    /// work log itself is not in this module.
    PursuitId
);

define_uuid_id!(
    /// Surrogate id for a `Thread` — one run of messages about one
    /// thing.
    ThreadId
);

define_uuid_id!(
    /// Surrogate id for a `Message`.
    ///
    /// A reply names one, and corrections are filed against one, so it
    /// is what a conversation is stitched together by.
    MessageId
);

define_uuid_id!(
    /// Surrogate id for whoever did something — the forge's handle on
    /// an actor, and the whole of what it knows about one.
    ///
    /// It is a handle rather than the identity itself, and the
    /// distinction is the point. Who a person *is* — which
    /// authenticated user, on which instance — is answered outside the
    /// forge, and the answer is not settled yet: the owner of an
    /// instance is an unbound reference until authentication binds it.
    /// An id minted here exists before that happens and keeps pointing
    /// at the same actor afterwards, so nothing already recorded has to
    /// move when the binding arrives.
    ///
    /// See [`Actor`](super::act::Actor) for the two kinds and why the
    /// kind is the one thing about an actor the forge keeps for itself.
    ActorId
);

/// Whether a row puts its entry on the line or takes it off.
///
/// The axis says nothing about bytes. `Absent` is a statement about
/// what the line carries — the content it pointed at is exactly as
/// live as it was — and taking an entry off is a thing the history
/// records, not a thing that removes a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Existence {
    /// On the line from this change point on.
    Present,
    /// Off it.
    Absent,
}

/// The one reference the forge holds into the layer below.
///
/// The id inside is an [`AssetId`], and this newtype is the whole
/// reason no other type in the model has to say so: the forge carries
/// content around and compares it without naming what it refers to.
/// Reaching the referent is the boundary's business, and the boundary
/// is the only place the two vocabularies meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Content(AssetId);

impl Content {
    /// Wraps an id from the layer below.
    pub fn of(asset: AssetId) -> Self {
        Self(asset)
    }

    /// Rehydrates a reference from the raw id it was written as.
    pub fn from_uuid(value: Uuid) -> Self {
        Self(AssetId::from_uuid(value))
    }

    /// The underlying id, as a bare UUID.
    pub fn as_uuid(&self) -> &Uuid {
        self.0.as_uuid()
    }

    /// The id itself, for the one place that has to say it out loud.
    ///
    /// Crate-visible rather than public: the only caller that should
    /// ever want this is the client that translates a reference into
    /// the vocabulary a contract is stated in
    /// ([`boundary`](crate::domain::forge::boundary)). Rust cannot
    /// narrow visibility to a sibling module, so the restriction is
    /// stated rather than held — and what will hold it is the crate
    /// split, where a caller outside the forge has no path to reach
    /// this at all.
    pub(crate) fn asset(&self) -> AssetId {
        self.0
    }
}

/// A name something can answer to: trimmed, and never blank.
///
/// A type rather than a check at each call site, because names arrive
/// from several directions and only one of them would remember. Two
/// names are the same name when their trimmed forms match exactly —
/// nothing else is normalised, since names are chosen by people who
/// can see the ones already there.
///
/// The model says nothing about where a name has to be unique. That is
/// a question about who owns the namespace, and the owner is outside
/// the forge.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(String);

impl Name {
    /// Trims, and rejects what is left when it is empty — a nameless
    /// thing has nothing to answer "which one" with.
    pub fn new(value: impl Into<String>) -> Result<Self, ForgeError> {
        let value = value.into().trim().to_string();
        if value.is_empty() {
            return Err(ForgeError::BlankName);
        }
        Ok(Self(value))
    }

    /// The name as it reads.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Which rule a line settles collisions by.
///
/// Text rather than a minted id, because it names a piece of code
/// rather than a row: what a line stores has to still mean something
/// after a deployment that carries different implementations, and a
/// slug somebody wrote (`"mainline-first"`) survives that where a
/// generated id would only say which row of a table that deployment no
/// longer has.
///
/// The forge neither knows the set nor holds the implementations —
/// see [`Strategy`](super::strategy::Strategy). What it knows is that
/// a line points at one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StrategyId(String);

impl StrategyId {
    /// Names a rule. Trimmed, and never blank — a line that points at
    /// nothing settles nothing.
    pub fn new(value: impl Into<String>) -> Result<Self, ForgeError> {
        let value = value.into().trim().to_string();
        if value.is_empty() {
            return Err(ForgeError::BlankStrategy);
        }
        Ok(Self(value))
    }

    /// The slug as it reads.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for StrategyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed_and_never_blank() {
        assert_eq!(Name::new("  key visual  ").unwrap().as_str(), "key visual");
        assert!(Name::new("   ").is_err());
        assert!(Name::new("").is_err());
    }

    /// The reference the forge carries is the one it was given. A
    /// newtype that lost the id would leave the forge pointing at
    /// nothing, and nothing else in the model can tell.
    #[test]
    fn a_reference_survives_being_wrapped() {
        let raw = uuid::Uuid::now_v7();

        let content = Content::from_uuid(raw);

        assert_eq!(*content.as_uuid(), raw);
        assert_eq!(content, Content::of(AssetId::from_uuid(raw)));
    }

    #[test]
    fn two_names_are_the_same_when_their_trimmed_forms_match() {
        assert_eq!(Name::new(" hero ").unwrap(), Name::new("hero").unwrap());
        assert_ne!(Name::new("Hero").unwrap(), Name::new("hero").unwrap());
    }
}
