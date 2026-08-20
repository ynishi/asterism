//! When something happened, and who did it.
//!
//! One type rather than two loose fields, because the two always
//! arrive together and a record with only half of them answers
//! neither question: a time with nobody attached says an event
//! occurred and refuses to say whose it was, which is the shape a
//! history exists to avoid. Every node of a history carries an
//! [`Act`], and so does anything whose own description can move.
//!
//! # Two records, not one
//!
//! [`Meta`] is the second of those. A line's change points say what
//! happened to what the line carries; they say nothing about the line
//! being renamed, or its strategy being changed, and those are not
//! change points — putting them on the same chain would make "the
//! line moved" and "the line is described differently" the same
//! event, and neither question could be answered afterwards.
//!
//! So a description keeps its own two stamps: when it was made, and
//! the last time it moved. They are equal until something moves it,
//! which is a fact rather than a placeholder — nothing has happened to
//! it yet.
//!
//! # Whether it was allowed is not asked here
//!
//! Recording who did a thing is what a history owes. Deciding whether
//! they were permitted to needs to know what a person is, and that
//! question belongs to the layer that does.

use chrono::{DateTime, Utc};

// SHARED KERNEL: attribution is a boundary type — both sides record
// who did a thing, and neither owns the vocabulary for it.
use crate::domain::attribution::AttributionContext;

/// One thing somebody did, at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Act {
    at: DateTime<Utc>,
    by: AttributionContext,
}

impl Act {
    /// Records an act.
    pub fn new(at: DateTime<Utc>, by: &AttributionContext) -> Self {
        Self { at, by: by.clone() }
    }

    /// When it happened.
    pub fn at(&self) -> DateTime<Utc> {
        self.at
    }

    /// Who did it, and through what.
    pub fn by(&self) -> &AttributionContext {
        &self.by
    }
}

/// A thing's own history, for the part of it that no other history
/// records.
///
/// A line's change points say what happened to what the line carries.
/// They say nothing about the line itself being renamed, or its
/// strategy being changed, and those are not change points — putting
/// them on the same chain would make "the line moved" and "the line
/// was described differently" the same event. So the description
/// keeps its own two stamps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    created: Act,
    updated: Act,
}

impl Meta {
    /// Starts a description. Nothing has moved yet, so both stamps are
    /// the same act.
    pub fn opened(act: Act) -> Self {
        Self {
            created: act.clone(),
            updated: act,
        }
    }

    /// Records that the description moved.
    pub fn touched(&mut self, act: Act) {
        self.updated = act;
    }

    /// When it was made, and by whom.
    pub fn created(&self) -> &Act {
        &self.created
    }

    /// The last time its description moved. Equal to
    /// [`created`](Self::created) while nothing has.
    pub fn updated(&self) -> &Act {
        &self.updated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 20, 12, minute, 0).unwrap()
    }

    fn act(minute: u32) -> Act {
        Act::new(at(minute), &AttributionContext::owner_surface())
    }

    #[test]
    fn a_fresh_description_has_not_moved() {
        let meta = Meta::opened(act(0));

        assert_eq!(meta.created(), meta.updated());
    }

    #[test]
    fn touching_moves_the_second_stamp_and_leaves_the_first() {
        let mut meta = Meta::opened(act(0));

        meta.touched(act(5));

        assert_eq!(meta.created().at(), at(0));
        assert_eq!(meta.updated().at(), at(5));
    }
}
