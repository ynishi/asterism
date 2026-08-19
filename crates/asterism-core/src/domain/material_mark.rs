//! `MaterialMark` — a mark placed into an Asset's **material**: the
//! coordinate space the asset's content carries, rather than the asset
//! as a row.
//!
//! An asset names one work. Its material is what that
//! work is made of, and a material has somewhere to point *inside*:
//! a time axis `[0, duration_ms)` for video and audio, a plane for
//! images and frames. [`MaterialAnchor`] is that "where", and the mark
//! is one note fastened to it.
//!
//! "Material" is [`Material`](crate::domain::material)'s word
//! (asset-model v4): the physical-original layer of an asset. The mark
//! nevertheless stores `asset_id`, not a material reference, and that
//! is not an inconsistency: materials are aggregate-internal —
//! identified by `(owning asset, ord)` and never referenced from
//! outside the aggregate — and the axis the anchor measures is the
//! asset's playback presentation of its primary original (`ord == 0`),
//! the same axis `asset.duration_ms` describes.
//!
//! Not a comment. [`AssetComment`](crate::domain::asset_comment) is a
//! thread hanging off an Asset as a whole and reads in posting order;
//! a mark points into the material and reads in the material's own
//! order. The two answer different questions ("what was said about
//! this" versus "what is here"), which is why this is a separate
//! aggregate rather than a nullable position column on the comment row.
//!
//! Design notes:
//!
//! - **The anchor is the axis, not the type.** A second coordinate
//!   space (a rectangle on an image, say) arrives as another
//!   [`MaterialAnchor`] variant, not another aggregate — the shape
//!   W3C Annotation gives its target + selector. Before this split the
//!   coordinate space was baked into the type name, and every new one
//!   cost a type, a table and an adapter.
//! - **Position is mandatory** (`anchor`, not `Option<MaterialAnchor>`)
//!   — a mark with no position is a comment, and that already exists.
//! - **The body carries the whole content.** No tag / kind axis: a tag
//!   axis would make "a mark with a tag and no body" expressible, and
//!   the non-empty `body` rule is the one invariant worth keeping while
//!   the requirement is still moving. Adding tags later is a join
//!   table, with this table unchanged.
//! - **Author is [`CommentAuthor`]**, reused rather than restated. The
//!   `Comment` in that name reads oddly here, but two spellings of the
//!   same author vocabulary would be the worse of the two problems.
//!   `body` / `author` / `created_at` / `edited_at` deliberately carry
//!   the same names and types as on `AssetComment`, so that the shared
//!   note vocabulary can be lifted out mechanically when there is a
//!   reason to.
//! - **`body` is a public field** (as on `AssetComment`), so a record
//!   update can empty it. The rule is therefore enforced at every door:
//!   at construction, on the way into storage
//!   ([`MaterialMark::validate`], which an adapter's `save` calls),
//!   and on the way back out ([`MaterialMark::rehydrate`]). The
//!   schema deliberately holds no `body` CHECK, because SQL's `trim` is
//!   a weaker predicate than Rust's and a weaker mirror is worse than
//!   none — so if the write door let a value past, the read door would
//!   be the first thing to see it, and by then it is a stored row that
//!   the only listing verb refuses.

use chrono::{DateTime, Utc};

use crate::domain::asset_comment::CommentAuthor;
use crate::domain::value::{AssetId, MaterialLayerId, MaterialMarkId};
use crate::error::DomainError;

/// Largest millisecond value that survives storage. The column is a
/// SQLite STRICT `INTEGER` — signed 64-bit — so `u64`'s upper half has
/// nowhere to land.
const MAX_STORABLE_MS: u64 = i64::MAX as u64;

/// Where on a timeline a mark sits. `end_ms == None` means "an
/// instant".
///
/// `None` names **the moment itself**, not `[start_ms, end of media)`.
/// The `t=10` of Media Fragments URI 1.0 does mean "from 10 to the
/// end", but that is a URI's outward wording; reading it into this type
/// would mean no one can say what a mark covers without also holding
/// `asset.duration_ms`, and this layer does not have it.
///
/// `Some(e)` is the half-open interval `[start_ms, e)`. `e == start_ms`
/// is refused because an interval covering nothing is not a mark. It is
/// **not** refused to normalise the spelling of an instant: on a
/// continuous axis `Some(start_ms + 1)` is a different thing (1 ms
/// wide) rather than a second spelling of the same thing, so no
/// normalisation is available to achieve.
///
/// **The axis is continuous**; milliseconds are the unit of record, not
/// a quantum. `HTMLMediaElement.currentTime` hands the writer a real
/// number to begin with. So an instant has no width, and asking "is the
/// playhead inside this mark" against one requires the caller to pick a
/// tolerance — this type does not carry one.
///
/// The refusal of `e == start_ms` is a deliberate disagreement with
/// `reject_inverted_band` (`application/mapping.rs:99`), which accepts
/// `min == max` as a one-value band. That is a closed band over a query
/// axis; this is a half-open interval over a timeline. Different
/// question, not a different answer to the same one.
///
/// **Origin**: `start_ms` counts from the start of the playback
/// timeline — the same zero `asset.duration_ms` measures against and
/// the same one `currentTime` reports. A non-zero container start PTS
/// or an embedded timecode does not enter into it. Writer (a playback
/// position in the UI) and reader (a seek back to it) share that zero,
/// so the round trip closes. Values on another origin (an ffprobe PTS,
/// say) must be converted before they arrive here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineSpan {
    start_ms: u64,
    end_ms: Option<u64>,
}

impl TimelineSpan {
    /// Builds a span, refusing an empty or inverted interval and any
    /// value past [`MAX_STORABLE_MS`].
    ///
    /// The range check lives here, rather than in the adapter, so that
    /// **anything constructible is storable**. Were it left to `save`,
    /// a design-level mistake (a value out of range) would be reported
    /// as `DomainError::Infra`, which already means something else — a
    /// row that should not exist was found in the database — and a log
    /// reader could no longer tell the two apart.
    pub fn new(start_ms: u64, end_ms: Option<u64>) -> Result<Self, DomainError> {
        if start_ms > MAX_STORABLE_MS {
            return Err(DomainError::Validation(format!(
                "TimelineSpan start_ms {start_ms} is past the storable range (max {MAX_STORABLE_MS})"
            )));
        }
        if let Some(end) = end_ms {
            if end > MAX_STORABLE_MS {
                return Err(DomainError::Validation(format!(
                    "TimelineSpan end_ms {end} is past the storable range (max {MAX_STORABLE_MS})"
                )));
            }
            if end <= start_ms {
                return Err(DomainError::Validation(format!(
                    "TimelineSpan end_ms {end} must be greater than start_ms {start_ms} — \
                     an interval that covers nothing is not a mark; \
                     pass None for an instant"
                )));
            }
        }
        Ok(Self { start_ms, end_ms })
    }

    /// Start of the span, in milliseconds from the playback origin.
    pub fn start_ms(&self) -> u64 {
        self.start_ms
    }

    /// Exclusive end of the span; `None` for an instant.
    pub fn end_ms(&self) -> Option<u64> {
        self.end_ms
    }

    /// Whether this span names a moment rather than an interval.
    pub fn is_instant(&self) -> bool {
        self.end_ms.is_none()
    }
}

/// Where in a material a mark points.
///
/// One variant per coordinate space the material can offer. `Temporal`
/// is the only one implemented: the playback timeline of a
/// time-bearing asset. A rectangle on an image plane is the next
/// candidate and arrives here as `Spatial(Rect)` — a variant, a column
/// group and a `kind_slug`, with [`MaterialMark`] and its port
/// unchanged. That is the whole point of naming the anchor separately
/// from the mark.
///
/// The variant is what decides which storage columns are populated, so
/// a match on it is exhaustive at every encode site by construction:
/// adding a variant makes each of those sites a compile error rather
/// than a row with the wrong columns filled in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialAnchor {
    /// A point or interval on the material's playback timeline.
    Temporal(TimelineSpan),
}

impl MaterialAnchor {
    /// Slug used on the wire and in the `anchor_kind` column
    /// (`"temporal"`).
    ///
    /// One spelling, so the adapter and the contract layer cannot
    /// disagree with the DDL's `CHECK (anchor_kind IN (...))`.
    pub fn kind_slug(&self) -> &'static str {
        match self {
            Self::Temporal(_) => "temporal",
        }
    }
}

/// One mark in an Asset's material.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialMark {
    /// Surrogate id (UUID v7; ordering of marks is by `anchor`, so the
    /// embedded timestamp only serves as a tie-break).
    pub id: MaterialMarkId,
    /// Asset whose material the mark points into.
    pub asset_id: AssetId,
    /// Band the mark belongs to — a
    /// [`MaterialLayer`](crate::domain::material_layer) with role
    /// `Annotation`.
    ///
    /// Mandatory, not `Option`. A nullable column meaning "the default
    /// band" would put the same fact in two shapes — NULL and a row
    /// pointing at the default layer — and every reader would have to
    /// know both; that implied-semantics drift is the thing layers
    /// exist to remove. The service resolves the default band (creating
    /// one if the asset has none) before it constructs a mark, so a
    /// caller that names no layer still gets a real one.
    ///
    /// `asset_id` stays beside it rather than being reached through the
    /// layer: the anchor is measured against *this asset's* playback
    /// timeline, and the listing that reads it (`ORDER BY start_ms`
    /// over one asset, index-backed) is a per-asset question. Keeping
    /// both means the two can disagree — a mark whose layer belongs to
    /// another asset — which is a rule about another row and so is the
    /// schema's, not this type's (V78 doc comment in `migrations.rs`).
    pub layer_id: MaterialLayerId,
    /// Where in that material it points.
    pub anchor: MaterialAnchor,
    /// Free-form body. Non-empty after trimming — refused at
    /// construction and on read-back.
    pub body: String,
    /// Author (the User, or a specific Persona).
    pub author: CommentAuthor,
    /// When the mark was placed.
    pub created_at: DateTime<Utc>,
    /// When it was last edited; `None` while untouched.
    pub edited_at: Option<DateTime<Utc>>,
}

impl MaterialMark {
    /// Places a new mark. Refuses a body that is empty after trimming.
    pub fn new(
        asset_id: AssetId,
        layer_id: MaterialLayerId,
        anchor: MaterialAnchor,
        author: CommentAuthor,
        body: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let body = body.into();
        Self::check_body(&body)?;
        Ok(Self {
            id: MaterialMarkId::new(),
            asset_id,
            layer_id,
            anchor,
            body,
            author,
            created_at: now,
            edited_at: None,
        })
    }

    /// Rebuilds a mark read back from storage.
    ///
    /// Adapters route every row through here rather than assembling the
    /// struct field by field, because two of this aggregate's three
    /// rules are not in the schema: the schema holds no `body` CHECK
    /// (SQL `trim` removes only U+0020, so the mirror would be weaker
    /// than the rule), and no `edited_at >= created_at` constraint
    /// (matching `asset_comment`). [`Self::validate`] closes the write
    /// door of the adapter that has one; this closes the read door,
    /// which is the only thing standing between a row that arrived by
    /// some other route — a raw INSERT, a migration, a second writer —
    /// and a caller holding it as a valid mark.
    ///
    /// Failures are `Validation` — the caller (an adapter) is expected
    /// to restate them as `Infra`, since a row like this being present
    /// is an infrastructure fact, not a caller error.
    // Long by construction, and it crossed clippy's threshold when
    // `layer_id` landed: the list is the row's columns, and the two
    // ways of shortening it both cost more than the length. Grouping
    // them into a parameter struct would put a second name on the same
    // shape; handing some in through public fields afterwards is what
    // routing every row through this constructor exists to stop.
    // Same call as `Asset::from_persisted` and
    // `DispatchJob::from_persisted`, which carry this allow for the
    // same reason.
    #[allow(clippy::too_many_arguments)]
    pub fn rehydrate(
        id: MaterialMarkId,
        asset_id: AssetId,
        layer_id: MaterialLayerId,
        anchor: MaterialAnchor,
        author: CommentAuthor,
        body: String,
        created_at: DateTime<Utc>,
        edited_at: Option<DateTime<Utc>>,
    ) -> Result<Self, DomainError> {
        let mark = Self {
            id,
            asset_id,
            layer_id,
            anchor,
            body,
            author,
            created_at,
            edited_at,
        };
        mark.validate()?;
        Ok(mark)
    }

    /// Checks the two rules that live nowhere else.
    ///
    /// `anchor` is not among them: each anchor variant wraps a value
    /// object that keeps its fields private (`TimelineSpan` does), so
    /// an invalid one cannot be built and cannot be reached by a record
    /// update. `body`, `created_at` and `edited_at` are public fields,
    /// so they can, and neither the schema nor a constructor stands in
    /// the way of the update itself.
    ///
    /// Every door into and out of storage calls this: an adapter's
    /// `save` on the way in, [`Self::rehydrate`] on the way out. A write
    /// door that skipped it would let a caller store a row that the read
    /// door then refuses — and since a listing promotes its rows into a
    /// single `Result`, one such row is enough to make the whole
    /// listing fail.
    ///
    /// Failures are `Validation`: the value came from a caller, and
    /// nothing infrastructural has gone wrong. An adapter reading such a
    /// row back out restates it as `Infra`, because *finding* one
    /// already stored is an infrastructure fact.
    pub fn validate(&self) -> Result<(), DomainError> {
        Self::check_body(&self.body)?;
        let created_at = self.created_at;
        if let Some(edited) = self.edited_at
            && edited < created_at
        {
            return Err(DomainError::Validation(format!(
                "MaterialMark edited_at {edited} precedes created_at {created_at}"
            )));
        }
        Ok(())
    }

    fn check_body(body: &str) -> Result<(), DomainError> {
        if body.trim().is_empty() {
            return Err(DomainError::Validation(
                "MaterialMark body must not be empty".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two ends of `TimelineSpan::new`.
    ///
    /// The interval axis: `end < start` and `end == start` are both
    /// refused, `None` and `start + 1` both accepted — the pair
    /// `None` / `Some(start + 1)` is the one that would collapse if
    /// anyone decided an instant should be stored as a 1 ms interval.
    ///
    /// The range axis: `i64::MAX as u64 + 1` is refused, which is what
    /// makes "constructible implies storable" true.
    #[test]
    fn span_rejects_empty_inverted_and_unstorable() {
        assert!(TimelineSpan::new(1_000, Some(999)).is_err(), "inverted");
        assert!(TimelineSpan::new(1_000, Some(1_000)).is_err(), "empty");
        assert!(
            TimelineSpan::new(0, Some(0)).is_err(),
            "empty at the origin"
        );

        let instant = TimelineSpan::new(1_000, None).unwrap();
        assert!(instant.is_instant());
        assert_eq!(instant.start_ms(), 1_000);
        assert_eq!(instant.end_ms(), None);

        let narrow = TimelineSpan::new(1_000, Some(1_001)).unwrap();
        assert!(
            !narrow.is_instant(),
            "a 1 ms interval is an interval, not another spelling of the instant"
        );
        assert_eq!(narrow.end_ms(), Some(1_001));

        let unstorable = i64::MAX as u64 + 1;
        assert!(
            TimelineSpan::new(unstorable, None).is_err(),
            "start past the signed 64-bit column"
        );
        assert!(
            TimelineSpan::new(0, Some(unstorable)).is_err(),
            "end past the signed 64-bit column"
        );
        assert!(
            TimelineSpan::new(i64::MAX as u64, None).is_ok(),
            "the edge itself fits"
        );
    }

    /// The anchor's slug is the one the `anchor_kind` column stores.
    ///
    /// Trivial while there is one variant, and that is the moment to
    /// write it: the assertion is what a second variant lands on, and
    /// the column's `CHECK (anchor_kind IN (...))` is the thing on the
    /// other side of it.
    #[test]
    fn anchor_kind_slug_names_the_column_value() {
        let span = TimelineSpan::new(0, None).unwrap();
        assert_eq!(MaterialAnchor::Temporal(span).kind_slug(), "temporal");
    }

    /// `body` is refused when trimming empties it — including on
    /// whitespace that SQL's `trim` would leave alone, which is the
    /// reason the schema carries no `body` CHECK.
    #[test]
    fn rejects_empty_body() {
        let asset = AssetId::new();
        let layer = MaterialLayerId::new();
        let anchor = MaterialAnchor::Temporal(TimelineSpan::new(0, None).unwrap());
        let now = Utc::now();
        let place =
            |body: &str| MaterialMark::new(asset, layer, anchor, CommentAuthor::User, body, now);
        assert!(place("").is_err());
        assert!(place("   ").is_err(), "spaces");
        assert!(place("\t").is_err(), "a tab — SQL trim() keeps this one");
        assert!(
            place("\u{3000}").is_err(),
            "an ideographic space — likewise"
        );
        let mark = place("here").unwrap();
        assert_eq!(mark.body, "here");
        assert_eq!(mark.edited_at, None);
        assert_eq!(mark.anchor, anchor);
    }

    /// `rehydrate` applies the rules the schema cannot.
    #[test]
    fn rehydrate_rejects_rows_the_constructor_would_refuse() {
        let id = MaterialMarkId::new();
        let asset = AssetId::new();
        let layer = MaterialLayerId::new();
        let anchor = MaterialAnchor::Temporal(TimelineSpan::new(0, Some(5)).unwrap());
        let created = DateTime::from_timestamp_millis(2_000).unwrap();
        let earlier = DateTime::from_timestamp_millis(1_000).unwrap();

        assert!(
            MaterialMark::rehydrate(
                id,
                asset,
                layer,
                anchor,
                CommentAuthor::User,
                "\t".into(),
                created,
                None
            )
            .is_err(),
            "a body the domain refuses stays refused on the way back in"
        );
        assert!(
            MaterialMark::rehydrate(
                id,
                asset,
                layer,
                anchor,
                CommentAuthor::User,
                "ok".into(),
                created,
                Some(earlier)
            )
            .is_err(),
            "edited before created"
        );
        assert!(
            MaterialMark::rehydrate(
                id,
                asset,
                layer,
                anchor,
                CommentAuthor::User,
                "ok".into(),
                created,
                Some(created)
            )
            .is_ok(),
            "edited at the same instant is fine"
        );
    }
}
