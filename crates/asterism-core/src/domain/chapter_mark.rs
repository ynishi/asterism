//! `ChapterMark` — one entry in a chapter list: a named section of an
//! Asset's material.
//!
//! A container declares how its content is divided (MP4's `chpl` and
//! `chap`, Matroska's `Chapters` segment, an MP3's ffmetadata
//! `[CHAPTER]` blocks). That declaration is data the file carries, and
//! before this type it had nowhere to land: an import either threw it
//! away or flattened it into free-text notes, where it was
//! indistinguishable from something a person had written.
//!
//! A chapter belongs to a [`MaterialLayer`](crate::domain::material_layer)
//! with role `Structure`, not to the asset. The layer is what says
//! whether this list is the file's own, a person's, or a job's — which
//! is the whole reason re-reading a file can replace one list without
//! touching another.
//!
//! Not a [`MaterialMark`](crate::domain::material_mark). A mark is a
//! note fastened to a position ("look at this"); a chapter is a claim
//! about how the material is *divided* ("this section starts here").
//! They share [`TimelineSpan`] and nothing else: the two carry
//! different fields, answer to different layer roles, and the rules
//! below differ from that aggregate's precisely where the difference in
//! meaning is.

use crate::domain::material_mark::TimelineSpan;
use crate::domain::value::{ChapterMarkId, MaterialLayerId};
use crate::error::DomainError;

/// One named section of a material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterMark {
    /// Surrogate id (UUID v7; chapters are read in `ord` order, so the
    /// embedded timestamp only serves as a tie-break).
    pub id: ChapterMarkId,
    /// Layer the chapter belongs to. Always a `Structure` layer — the
    /// role is the layer's fact, and duplicating it here would make two
    /// answers to one question.
    pub layer_id: MaterialLayerId,
    /// Where the section sits on the material's playback timeline.
    ///
    /// Reused from `material_mark` rather than restated: the axis, its
    /// origin and its refusals ("an interval covering nothing is not a
    /// mark", the storable range) are the same facts about the same
    /// timeline, and two spellings of one axis would be the worse of
    /// the two problems.
    pub span: TimelineSpan,
    /// The section's title as the container declares it — **empty is
    /// legal** (see [`Self::validate`]).
    pub label: String,
    /// Reading order within the layer.
    ///
    /// Carried rather than derived from `span`, because the two need
    /// not agree: a container is free to declare its chapters in an
    /// order of its own, and the list a person reads is the one the
    /// file states.
    pub ord: u32,
}

impl ChapterMark {
    /// Places a chapter in a structure layer.
    pub fn new(
        layer_id: MaterialLayerId,
        span: TimelineSpan,
        label: impl Into<String>,
        ord: u32,
    ) -> Result<Self, DomainError> {
        let chapter = Self {
            id: ChapterMarkId::new(),
            layer_id,
            span,
            label: label.into(),
            ord,
        };
        chapter.validate()?;
        Ok(chapter)
    }

    /// Rebuilds a chapter read back from storage.
    ///
    /// Adapters route every row through here rather than assembling the
    /// struct field by field, matching
    /// [`MaterialMark::rehydrate`](crate::domain::material_mark::MaterialMark::rehydrate):
    /// the read door is where a rule this aggregate grows later takes
    /// effect on rows that are already stored, and a `Result` here now
    /// is what keeps that from being a signature change then.
    ///
    /// Failures are `Validation`; an adapter restates them as `Infra`,
    /// since finding such a row stored is an infrastructure fact.
    pub fn rehydrate(
        id: ChapterMarkId,
        layer_id: MaterialLayerId,
        span: TimelineSpan,
        label: String,
        ord: u32,
    ) -> Result<Self, DomainError> {
        let chapter = Self {
            id,
            layer_id,
            span,
            label,
            ord,
        };
        chapter.validate()?;
        Ok(chapter)
    }

    /// Holds no rule of its own — and the two it conspicuously does
    /// *not* hold are the point.
    ///
    /// **An empty `label` is accepted**, where
    /// [`MaterialMark`](crate::domain::material_mark)'s `body` is
    /// refused. That aggregate's body is the whole content of something
    /// a person chose to write, so a blank one is a mark that says
    /// nothing. A chapter's label is container metadata: the section
    /// exists because the file declares a division at that timestamp,
    /// and plenty of files declare untitled ones (an `chpl` entry with
    /// an empty string, a Matroska `ChapterAtom` with no
    /// `ChapterDisplay`). Refusing those would mean an import either
    /// drops a section the file really has or invents a title for it,
    /// and both are worse than an empty string.
    ///
    /// **An instant span is accepted** — `end_ms == None`, meaning "the
    /// section starts here" with no stated end. MP4's `chpl` declares
    /// start times only; the end of a chapter is the start of the next
    /// one, which is a fact about *other rows* and so not something a
    /// single chapter can be required to carry.
    ///
    /// What remains is enforced by construction: `span` is a value
    /// object with private fields, so an inverted or unstorable
    /// interval cannot be reached even by a record update, and `ord` is
    /// a `u32` against an `INTEGER` column. Every door still calls this
    /// — `new`, `rehydrate`, and an adapter's `save` — so a rule added
    /// later lands in one place rather than in three.
    pub fn validate(&self) -> Result<(), DomainError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two shapes the aggregate deliberately accepts, and the
    /// fields it keeps apart.
    ///
    /// The fixtures are the ones the containers actually produce: an
    /// untitled section (Matroska `ChapterAtom` with no
    /// `ChapterDisplay`) and a start-only section (MP4 `chpl`, where
    /// the end is the next entry's start). Asserted here rather than in
    /// the parser, because it is this type's rule that decides whether
    /// the parser has anywhere to put them.
    #[test]
    fn accepts_an_untitled_section_and_one_with_no_stated_end() {
        let layer = MaterialLayerId::new();

        let untitled = ChapterMark::new(layer, TimelineSpan::new(0, Some(30_000)).unwrap(), "", 0)
            .expect("a file may declare a section without naming it");
        assert_eq!(untitled.label, "");
        assert_eq!(untitled.span.end_ms(), Some(30_000));

        let start_only =
            ChapterMark::new(layer, TimelineSpan::new(30_000, None).unwrap(), "Two", 1)
                .expect("chpl declares start times; the end is the next entry's start");
        assert!(start_only.span.is_instant());
        assert_eq!(start_only.label, "Two");
        assert_eq!(start_only.ord, 1);
        assert_ne!(untitled.id, start_only.id, "each section is its own row");
    }

    /// Reading order is the layer's, not the timeline's.
    ///
    /// `ord` and `span` are separate fields because a container may
    /// declare its sections out of timeline order; a `ChapterMark` that
    /// derived one from the other could not hold that file.
    #[test]
    fn ord_is_carried_rather_than_derived_from_the_position() {
        let layer = MaterialLayerId::new();
        let late_but_first =
            ChapterMark::new(layer, TimelineSpan::new(60_000, None).unwrap(), "A", 0).unwrap();
        let early_but_second =
            ChapterMark::new(layer, TimelineSpan::new(10_000, None).unwrap(), "B", 1).unwrap();
        assert!(late_but_first.span.start_ms() > early_but_second.span.start_ms());
        assert!(late_but_first.ord < early_but_second.ord);
    }

    /// `rehydrate` accepts what the constructor accepts, id and all.
    #[test]
    fn rehydrate_round_trips_a_stored_row() {
        let id = ChapterMarkId::new();
        let layer = MaterialLayerId::new();
        let chapter = ChapterMark::rehydrate(
            id,
            layer,
            TimelineSpan::new(5_000, Some(9_000)).unwrap(),
            String::new(),
            4,
        )
        .expect("an untitled stored section is a section");
        assert_eq!(chapter.id, id);
        assert_eq!(chapter.layer_id, layer);
        assert_eq!(chapter.ord, 4);
    }
}
