//! The shape of the work a released file came out of, as the file
//! itself will state it.
//!
//! A disclosure says how an artefact was made. This says who chose it:
//! a person opened work with something in mind, took some number of
//! rounds at it, ended it, and later released what the line then
//! carried. Those are the facts a reviewer holding only the file has to
//! be able to read, and the reason the forge records an
//! [`Act`](crate::domain::forge::model::act::Act) on every node.
//!
//! # Why the forge's own words do not travel
//!
//! This module names nothing in the forge. It carries a pursuit id as
//! text, an actor as [`Hand`], and two instants — none of which is a
//! forge type — because a disclosure is rendered by
//! `asterism-disclosure-format`, and a renderer that had to know what a
//! `Pursuit` is would put the intentional history into a crate whose
//! subject is container formats. The translation happens once, where
//! the release is assembled.
//!
//! # What is left out, and why each one
//!
//! **Round notes.** Free text somebody wrote for themselves, in a
//! signed document nobody can edit afterwards. The team publish path
//! already decided this for a receiver it does not control
//! (`asterism-teams-client::publish`), and a marketplace reviewer is a
//! stranger by a wider margin than a team is.
//!
//! **The operations.** What a round added, replaced, renamed or removed
//! is the deliberation, and the released set is the conclusion. A
//! reader holding the files can see what was chosen; what they cannot
//! see, and what this supplies, is that somebody chose it.
//!
//! **The prompt.** Kept out of the manifest wherever it appears, on the
//! terms `PromptDisclosure` already sets: it is disclosed once, in the
//! packet, under the IPTC property defined for it.
//!
//! # A count rather than the acts of every round
//!
//! [`rounds`](ReleaseDisclosure::rounds) says how many, not who did
//! each. Which rounds a person did and which a rule did is a question
//! about the deliberation, and the two acts that are carried whole —
//! the close and the release — are the two that decide something.

use chrono::{DateTime, Utc};

/// Whether a person or a rule did something.
///
/// The disclosure's own word for the distinction
/// [`Actor`](crate::domain::forge::model::act::Actor) draws, spelled
/// here so that this value carries no forge type. Which person is not
/// disclosed and could not be: an `ActorId` is a handle this
/// application resolves and a reader of the file cannot, and the
/// certificate the manifest is signed under is the name that travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    /// A person chose this.
    Person,
    /// A rule wrote it — a line's strategy settling a collision, or
    /// anything else the server did rather than somebody.
    Rule,
}

impl Hand {
    /// The word as it is written into the assertion.
    ///
    /// A stable string rather than a `Debug` rendering: it goes into a
    /// signed document that cannot be corrected afterwards, so renaming
    /// the variant must not silently rename what a reader keys on.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Rule => "rule",
        }
    }
}

/// One act, reduced to what a file may say about it.
///
/// When, and whose hand. That pair is the whole of what a validator can
/// act on, and it is exactly what the forge records on every node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseAct {
    /// When it happened.
    pub at: DateTime<Utc>,
    /// Whether a person or a rule did it.
    pub by: Hand,
}

impl ReleaseAct {
    /// Records an act as the file will state it.
    pub fn new(at: DateTime<Utc>, by: Hand) -> Self {
        Self { at, by }
    }
}

/// What a released file says about the work that reached it.
///
/// Assembled once, when the run that carried the files reports what it
/// wrote, and rendered into the manifest's custom assertion. Manifest
/// only, for the reason
/// [`DisclosureRecord::release`](super::DisclosureRecord::release)
/// gives on the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseDisclosure {
    /// The release's own act: when the set was written out, and whether
    /// a person or a rule said so.
    pub released: ReleaseAct,
    /// The pursuit the released change point came out of, as text.
    ///
    /// A pointer into the library that recorded it, on the same terms
    /// as the asset and dispatch ids beside it: a reader holding this
    /// Asterism instance resolves it, and a reader who does not at
    /// least learns that the deliberation exists and is recorded.
    pub pursuit_id: String,
    /// What the work was called, when whoever opened it gave it a name.
    pub intent_title: Option<String>,
    /// How many rounds were taken at it.
    pub rounds: u32,
    /// The act that ended the work — the close the released change
    /// point was born with.
    pub closed: ReleaseAct,
}

impl ReleaseDisclosure {
    /// States the shape of one released change point's work.
    pub fn new(
        released: ReleaseAct,
        pursuit_id: impl Into<String>,
        intent_title: Option<String>,
        rounds: u32,
        closed: ReleaseAct,
    ) -> Self {
        Self {
            released,
            pursuit_id: pursuit_id.into(),
            intent_title,
            rounds,
            closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 11, 12, minute, 0).unwrap()
    }

    #[test]
    fn a_hand_reads_the_same_however_the_variant_is_spelled() {
        assert_eq!(Hand::Person.as_str(), "person");
        assert_eq!(Hand::Rule.as_str(), "rule");
    }

    #[test]
    fn a_release_carries_both_acts_and_the_shape_between_them() {
        let shape = ReleaseDisclosure::new(
            ReleaseAct::new(at(30), Hand::Person),
            "pursuit-1",
            Some("the key visual".into()),
            3,
            ReleaseAct::new(at(10), Hand::Rule),
        );

        assert_eq!(shape.released.at, at(30));
        assert_eq!(shape.released.by, Hand::Person);
        assert_eq!(shape.closed.by, Hand::Rule);
        assert_eq!(shape.rounds, 3);
        assert_eq!(shape.intent_title.as_deref(), Some("the key visual"));
    }
}
