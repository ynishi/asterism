//! `DuplicateConflict` — one open question of the form "these two rows
//! hold the same bytes; are they the same thing?".
//!
//! A fingerprint landing on bytes another asset already holds is a
//! fact, and [`EdgeKind::IdenticalTo`](crate::domain::edge::EdgeKind::IdenticalTo)
//! is where that fact is recorded. This is the other half: the *question*
//! the fact raises, parked where a person can answer it later
//! The two are deliberately separate —
//! the edge outlives every answer, the question stops being asked once
//! it has one.
//!
//! # Why a table and not the edge
//!
//! The edge cannot carry the queue. It is written on all three
//! strategies, including the two that ask nothing (`fold` acts
//! immediately, `separate` records and moves on), so "there is an
//! `identical_to` edge" is not the same statement as "somebody still has
//! to look at this". Deriving the queue from edges would also make a
//! resolution unrecordable: closing a question by deleting the edge
//! would destroy the byte-level fact, which is the one thing a `keep`
//! ruling explicitly preserves.
//!
//! # The pair, not the event
//!
//! A row here is keyed by the **unordered pair** ([`pair_key`]), even
//! though the fields remember which side arrived last. Detection is an
//! event and has a direction; the question is about the pair and does
//! not. Which row happens to be fingerprinted first depends on whether
//! the bytes arrived through an import or through the backfill walk, and
//! keying on that would put the same pair on the queue twice — once from
//! each end — for a user to answer twice.
//!
//! [`pair_key`]: DuplicateConflict::pair_key

use chrono::{DateTime, Utc};

use crate::domain::value::{AssetId, DuplicateConflictId, PersonaId};
use crate::error::DomainError;

/// Which fingerprint the two rows agreed on.
///
/// The same closed vocabulary
/// [`EdgeKind::IdenticalTo`](crate::domain::edge::EdgeKind::IdenticalTo)
/// documents for its label, as one type rather than two string literals:
/// the edge's label and this column are written from the same detection
/// and have to say the same word, and a queue row that disagreed with
/// its edge would be a conflict on an axis nothing computed.
///
/// # The three, and how they stand
///
/// ```text
///    Artefact  =  Content  +  Meta
/// ```
///
/// `Artefact` hashes every byte. `Content` hashes only the bytes that
/// survive into the decoded result, and `Meta` hashes the metadata that
/// definition drops — the exact complement, canonically rendered
/// ([`material_meta`](crate::domain::material_meta)). So `Artefact`
/// agreement implies both of the others, and **neither of the others
/// implies anything about the rest**: two frames off one workflow
/// differing only by a seed agree on `Meta` and not `Content`, and one
/// picture re-exported with a caption written in agrees on `Content`
/// and not `Meta`.
///
/// # `Artefact`, and the one spelling it has
///
/// The strongest axis hashes every byte, which is the **artefact** —
/// not a property of files, and not the `Source` (that word belongs to
/// the Value Object naming where a record came from). The slug follows
/// the identifier: the stored value **is** the axis rather than a
/// column name, so it moved with it (V64 rewrites `'file'` to
/// `'artefact'` in `duplicate_conflict.axis`, on the `identical_to`
/// edge's label, and in the declared-hash note). The columns those
/// digests live in (`material.content_hash`, `content_region_hash`,
/// `meta_hash`) are a different question and keep their names —
/// renaming a column is a migration bought for readability, renaming
/// the axis was free.
///
/// So there is no seam where two spellings meet. Each axis has one
/// word, in Rust, in SQLite, on the wire, and in `bindings.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateAxis {
    /// Every byte of the artefact (`sha256:` over the whole file).
    Artefact,
    /// Only the bytes that decide the decoded result.
    Content,
    /// Only the metadata the container carries about the artefact.
    Meta,
}

impl DuplicateAxis {
    /// The axes in order of strength — strongest first.
    ///
    /// Detection walks this and stops at the first agreement
    /// ([`detect_duplicate`](crate::application_support::duplicate_detection::detect_duplicate)).
    /// The list is here rather than at the detector because the order
    /// is a property of what the axes *mean*.
    ///
    /// # Two different reasons to stop, and only one of them is
    /// implication
    ///
    /// `Artefact` implies both of the others, so a pair that agrees on
    /// it and is also reported on `Content` would be the same finding
    /// stated more weakly — which is what [`parse`](Self::parse)
    /// refuses to do when it declines to default an unknown slug.
    ///
    /// **`Content` and `Meta` imply nothing about each other**, so
    /// stopping between them is not that argument. It is a different
    /// one, and it is about the queue rather than about the axes: a
    /// `duplicate_conflict` row is a *question put to a person about a
    /// pair*, and one pair is one question. A pair that agrees on both
    /// (same pixels, same embedded text, different bytes — a re-encode
    /// that preserved every chunk payload) would otherwise be asked
    /// twice and answered twice.
    ///
    /// What that costs is stated rather than hidden: for such a pair
    /// the queue records `Content` and the `Meta` agreement is not
    /// separately raised. The pair still reaches a person, and the fact
    /// itself is not lost — both digests are stored columns, so "these
    /// two also share their metadata" is a comparison anyone can make
    /// against `material.meta_hash`. Nothing that is discarded here is
    /// unrecoverable, which is the property that makes stopping
    /// defensible where implication does not.
    ///
    /// `Content` comes before `Meta` because it is the stronger claim
    /// about sameness: "the same picture" is evidence two rows are one
    /// thing, while "made the same way" is evidence about how they were
    /// produced and holds across a batch nobody would fold.
    ///
    /// # `Meta` is not a weaker identity axis — it is not one
    ///
    /// The paragraphs above are the record of why the walk stops, and
    /// that part is still true. The premise they rest on is not.
    ///
    /// **`Artefact = Content + Meta`.** The whole bytes are the picture
    /// plus the metadata written about it, so the independent axes are
    /// **two**, and `Artefact` is the name for both agreeing rather than
    /// a third comparison. This list puts three in parallel, and the
    /// detector walks it in order and stops at the first agreement — so
    /// **a pair reaches `Meta` only when the other two found nothing**,
    /// which is to say its two pictures are different. "Neither implies
    /// the other" is true of the digests and does not license the
    /// conclusion it was written for: metadata-alone agreement is not a
    /// weaker claim that two rows are one thing, it is a statement that
    /// they were made the same way, which is
    /// [`series`](crate::domain::series)'s and folds nothing.
    ///
    /// The consequence is live rather than theoretical: such a pair
    /// becomes a `duplicate_conflict` row, and answering one `folded`
    /// replaces a distinct picture with a tombstone. **Nothing in this
    /// build corrects it.** The shape it moves to — identity entered
    /// through `Content`, `Meta` consulted only to word the question,
    /// and this constant kept as the *vocabulary* of stored axes while a
    /// second list carries the ones identity walks — is not built here,
    /// and a database that already holds `axis = 'meta'` rows will need
    /// its own migration when it is.
    ///
    /// # An axis left off this list, and what does not notice
    ///
    /// Adding a variant to the enum stops the build in seven places,
    /// all of them exhaustive `match`es: [`as_str`](Self::as_str),
    /// [`digest_prefix`](crate::domain::content_hash::digest_prefix),
    /// [`reserved_values`](crate::domain::content_hash::reserved_values),
    /// the two sites that pick the axis's value off a fingerprint
    /// ([`detect_duplicate`](crate::application_support::duplicate_detection::detect_duplicate)
    /// and the hash job's `declared_axis_value`), the adapter's
    /// `axis_column`, and the domain-to-wire `axis_to_dto`. Between them
    /// they make the author name the slug, the tag, the exclusions,
    /// where the value comes from, the column and the wire word before
    /// anything compiles.
    ///
    /// [`parse`](Self::parse) is not among them: it matches on `&str`,
    /// so it keeps compiling and keeps refusing the new slug until
    /// somebody adds the arm by hand. Whoever is fixing `as_str` two
    /// lines above it almost certainly does — which is what opens the
    /// read path described below.
    ///
    /// **This list is not one of the seven.** It is a hand-written
    /// array, and Rust has no way on its own to require an axis to
    /// appear in one —
    /// that needs a variant-enumerating derive, which this workspace
    /// deliberately does not carry for a three-variant enum. So the gap
    /// is stated rather than guarded, because a gap nobody wrote down
    /// is found by the symptom instead.
    ///
    /// The symptom, if it happens: detection walks this list
    /// ([`detect_duplicate`](crate::application_support::duplicate_detection::detect_duplicate)),
    /// so an axis missing from it is **never detected**. The column
    /// fills, the digest is computed and stored, every query works, and
    /// no conflict is ever raised.
    /// [`axis_of`](crate::domain::content_hash::axis_of) reads the same
    /// list, so a stored value on that axis also reads as carrying no
    /// tag at all — a declaration naming it is refused, and the hash
    /// job skips the check. Nothing errors.
    ///
    /// The read path is the exception, and it is what the symptom
    /// actually looks like. The duplicate report takes its axis as a
    /// slug from the caller and resolves it with [`parse`](Self::parse),
    /// which reads the arms written there rather than this list. So once
    /// `parse` has learned the slug the report answers on the missing
    /// axis — real groups, real members, built from the column
    /// `axis_column` names — while the conflict queue stays permanently
    /// empty on it. The two halves disagree about whether the axis is
    /// live rather than the axis being uniformly absent, which is the
    /// more findable failure of the two and the one to expect first.
    ///
    /// What does help is a tripwire on the other direction:
    /// `axis_and_resolution_slugs_round_trip` pins this array's
    /// contents literally, so *adding* an axis here fails that
    /// assertion and brings whoever did it to the assertion that also
    /// checks the slug round-trips. That catches the edit, not its
    /// absence.
    pub const STRONGEST_FIRST: &'static [Self] = &[Self::Artefact, Self::Content, Self::Meta];

    /// Slug stored in `duplicate_conflict.axis` and on the edge's label.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Artefact => "artefact",
            Self::Content => "content",
            Self::Meta => "meta",
        }
    }

    /// Parses an axis slug, refusing the unknown for the reason
    /// [`FoldPolicy::parse`](crate::domain::value::FoldPolicy::parse)
    /// records: a value outside the closed set is a corrupt row, and
    /// reading it as `Artefact` would claim a stronger agreement than
    /// anything measured.
    ///
    /// `"file"` is refused with everything else, and that is the point
    /// rather than an oversight. It was this axis's slug until V64
    /// rewrote every stored one, so after the migration nothing writes
    /// it and nothing holds it — which makes an arriving `"file"` two
    /// things worth hearing about rather than reading past: a database
    /// that did not go through V64, or a caller compiled against a
    /// vocabulary this build does not have. Accepting it as an alias
    /// would answer both silently, and would keep a second spelling
    /// alive on a wire that has no downstream to protect.
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "artefact" => Ok(Self::Artefact),
            "content" => Ok(Self::Content),
            "meta" => Ok(Self::Meta),
            other => Err(DomainError::Validation(format!(
                "unknown duplicate axis: {other:?}"
            ))),
        }
    }
}

/// How an open question was answered.
///
/// Two values, because a person looking at a pair has two things to say:
/// they are one thing ([`Folded`](Self::Folded)) or they are two
/// ([`Kept`](Self::Kept)). There is deliberately no third value for "one
/// of them went away" — a row that has been folded or thrown out has not
/// answered anything, and the disappearance is re-derived on every read
/// (see [`DuplicateConflict::is_open`]) rather than frozen into a
/// verdict a restore from the trash would have to undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    /// The two were ruled one thing and the newer row was folded into
    /// the keeper.
    Folded,
    /// The two were ruled separate things. The losing side of that
    /// ruling is nothing — both rows stay, and the row a person ruled
    /// on carries [`FoldPolicy::Keep`](crate::domain::value::FoldPolicy::Keep)
    /// so the question is not raised again.
    Kept,
}

impl ConflictResolution {
    /// Slug stored in `duplicate_conflict.resolution`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Folded => "folded",
            Self::Kept => "kept",
        }
    }

    /// Parses a resolution slug (unknown values are refused rather than
    /// read as either answer).
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "folded" => Ok(Self::Folded),
            "kept" => Ok(Self::Kept),
            other => Err(DomainError::Validation(format!(
                "unknown conflict resolution: {other:?}"
            ))),
        }
    }
}

/// Why a pair that a lane asked to fold was put on the queue instead.
///
/// The exclusions stop an **automatic** fold
/// and nothing else: the pair still goes in front of a person, and the
/// manual merge verb is deliberately not bound by them (a person
/// looking at two rows can see what the rule was protecting). That
/// leaves a gap this column closes. Without it the panel shows a
/// question indistinguishable from any other `ask`, and somebody folds
/// by hand exactly the pair the rule declined to fold — never having
/// been told the rule existed.
///
/// `None` on a row means no automatic fold was declined: either nobody
/// asked for one ([`OnDuplicate::Ask`](crate::domain::value::OnDuplicate::Ask),
/// which is most rows) or the pass itself was the reason (a conflict
/// the backfill found is never folded on its own, and that is a fact
/// about *when the pair was noticed*, not about the pair — a panel
/// warning drawn from it would be advice about nothing the two rows
/// have in common).
///
/// One value per row, not a set: the row records why the fold did not
/// happen, and the first rule that answered is a sufficient answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldExclusion {
    /// The two are connected through `derived_from` — one is an
    /// ancestor of the other, or both descend from something in common
    /// — or the graph around them was too large to walk far enough to
    /// say otherwise.
    Lineage,
    /// At least one of the rows is the output of an export run
    /// (`source_kind` under
    /// [`DISPATCH_PREFIX`](crate::domain::value::SourceKind::DISPATCH_PREFIX)).
    Dispatch,
}

impl FoldExclusion {
    /// Slug stored in `duplicate_conflict.fold_exclusion`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lineage => "lineage",
            Self::Dispatch => "dispatch",
        }
    }

    /// Parses an exclusion slug, refusing the unknown for the reason
    /// [`DuplicateAxis::parse`] records: reading an unrecognised word
    /// as one of these would put a warning in front of a person that
    /// names a rule nothing applied.
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "lineage" => Ok(Self::Lineage),
            "dispatch" => Ok(Self::Dispatch),
            other => Err(DomainError::Validation(format!(
                "unknown fold exclusion: {other:?}"
            ))),
        }
    }
}

/// One raised — and possibly answered — duplicate question.
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateConflict {
    /// Surrogate id (UUID v7).
    pub id: DuplicateConflictId,
    /// The persona both rows belong to. Matching never crosses personas
    /// by design, so one field covers the pair.
    pub persona_id: PersonaId,
    /// The row whose fingerprint raised the question — the newer
    /// arrival, and the `from` side of the edge written with it.
    pub newcomer: AssetId,
    /// The row that already held these bytes, oldest first among the
    /// holders.
    pub incumbent: AssetId,
    /// Which fingerprint agreed.
    pub axis: DuplicateAxis,
    /// The digest the two share, stored so the queue can be read
    /// without re-hydrating either material.
    pub content_hash: String,
    /// Why an automatic fold was declined, when one was asked for.
    /// `None` = none was declined ([`FoldExclusion`]).
    pub fold_exclusion: Option<FoldExclusion>,
    /// When the match was observed.
    pub detected_at: DateTime<Utc>,
    /// When somebody answered. `None` = still on the queue.
    pub resolved_at: Option<DateTime<Utc>>,
    /// The answer. Set with [`resolved_at`](Self::resolved_at) and never
    /// without it.
    pub resolution: Option<ConflictResolution>,
}

impl DuplicateConflict {
    /// Raises a question about a pair, refusing a row against itself.
    ///
    /// The self-pair is refused here rather than filtered by the caller
    /// for the reason
    /// [`ConstellationEdge::new`](crate::domain::edge::ConstellationEdge::new)
    /// refuses its own: the lookup that finds holders of a digest
    /// returns the asset that was just fingerprinted along with the
    /// others, so "did I exclude myself" is a mistake every caller can
    /// make once.
    ///
    /// `fold_exclusion` is a parameter rather than something set on the
    /// value afterwards because it is part of what the question records
    /// — a row raised without it says an ordinary `ask` happened, and
    /// there is no later moment at which the detector still knows
    /// better.
    pub fn raise(
        persona_id: PersonaId,
        newcomer: AssetId,
        incumbent: AssetId,
        axis: DuplicateAxis,
        content_hash: impl Into<String>,
        fold_exclusion: Option<FoldExclusion>,
        detected_at: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        if newcomer == incumbent {
            return Err(DomainError::Validation(
                "a duplicate conflict must name two distinct assets".into(),
            ));
        }
        Ok(Self {
            id: DuplicateConflictId::new(),
            persona_id,
            newcomer,
            incumbent,
            axis,
            content_hash: content_hash.into(),
            fold_exclusion,
            detected_at,
            resolved_at: None,
            resolution: None,
        })
    }

    /// The unordered pair, smaller id first — the key one question is
    /// stored under, whichever end raised it.
    pub fn pair_key(&self) -> (AssetId, AssetId) {
        if self.newcomer <= self.incumbent {
            (self.newcomer, self.incumbent)
        } else {
            (self.incumbent, self.newcomer)
        }
    }

    /// The other side of the pair, given the row a person chose to
    /// keep — and a refusal when the id names neither.
    ///
    /// The keeper is named by whoever answers rather than derived here.
    /// Age picks the keeper for an *automatic* fold ([`orient`]), and
    /// this row exists precisely because that choice was handed to a
    /// person; re-deriving it would put the machine's answer back into
    /// the one moment somebody is overruling it. What the domain can
    /// still say is that the answer has to be about *this* pair, which
    /// is what this checks — a keeper from another pair would fold two
    /// rows nobody was asked about.
    ///
    /// [`orient`]: crate::application_support::duplicate_detection
    pub fn headstone_for(&self, keeper: &AssetId) -> Result<AssetId, DomainError> {
        if keeper == &self.newcomer {
            Ok(self.incumbent)
        } else if keeper == &self.incumbent {
            Ok(self.newcomer)
        } else {
            Err(DomainError::Validation(format!(
                "keeper {keeper} is not part of this conflict ({} / {})",
                self.newcomer, self.incumbent
            )))
        }
    }

    /// Whether the question is still unanswered.
    ///
    /// A property of this row alone. It is **not** the whole of "should
    /// this appear on the queue": a pair whose sides have since been
    /// folded or thrown out is not worth asking about either, and that
    /// half is decided by the reader against the current state of the
    /// two rows
    /// ([`AssetRepository::list_open_duplicate_conflicts`](crate::domain::repository::AssetRepository::list_open_duplicate_conflicts)),
    /// because it can change back.
    pub fn is_open(&self) -> bool {
        self.resolved_at.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_and_resolution_slugs_round_trip() {
        for axis in DuplicateAxis::STRONGEST_FIRST {
            assert_eq!(DuplicateAxis::parse(axis.as_str()).unwrap(), *axis);
        }
        assert!(DuplicateAxis::parse("Artefact").is_err());
        assert!(DuplicateAxis::parse("bytes").is_err());
        // The detection writes one word twice — into the edge's label
        // and into the queue row's column. If these drift, a conflict
        // would name an axis its own edge denies.
        //
        // `Artefact` stored `file` until V64 rewrote the stored values.
        // The tripwire that used to pin the old slug (so nobody could
        // finish the rename without the migration) now points the other
        // way: the migration has landed, and `"file"` must not creep
        // back in as an alias — a build that answered to it would read a
        // database V64 never touched as though it had.
        assert_eq!(DuplicateAxis::Artefact.as_str(), "artefact");
        assert!(DuplicateAxis::parse("file").is_err());
        assert_eq!(DuplicateAxis::Content.as_str(), "content");
        // Every axis's slug is its name, lowercased. Stated per axis
        // rather than derived, because the failure being pinned is a
        // hand-written arm going to the wrong word.
        assert_eq!(DuplicateAxis::Meta.as_str(), "meta");
        // Strength order, stated once: a pair that agrees on more than
        // one of these is reported on the first that agrees.
        assert_eq!(
            DuplicateAxis::STRONGEST_FIRST,
            &[
                DuplicateAxis::Artefact,
                DuplicateAxis::Content,
                DuplicateAxis::Meta
            ]
        );

        for answer in [ConflictResolution::Folded, ConflictResolution::Kept] {
            assert_eq!(ConflictResolution::parse(answer.as_str()).unwrap(), answer);
        }
        assert!(ConflictResolution::parse("keep").is_err());

        for reason in [FoldExclusion::Lineage, FoldExclusion::Dispatch] {
            assert_eq!(FoldExclusion::parse(reason.as_str()).unwrap(), reason);
        }
        // The panel turns this word into a sentence about why the pair
        // was not folded, so an unknown one has no sentence to become.
        assert!(FoldExclusion::parse("derived").is_err());
        assert!(FoldExclusion::parse("").is_err());
    }

    #[test]
    fn a_pair_keys_the_same_way_from_either_end() {
        let a = AssetId::new();
        let b = AssetId::new();
        let persona = PersonaId::new();
        let hash = "sha256:abc";
        let one = DuplicateConflict::raise(
            persona,
            a,
            b,
            DuplicateAxis::Artefact,
            hash,
            None,
            chrono::Utc::now(),
        )
        .unwrap();
        // The mirror event: the same pair detected from the other side,
        // which is what the backfill walk produces if it reaches the
        // rows in the opposite order.
        let other = DuplicateConflict::raise(
            persona,
            b,
            a,
            DuplicateAxis::Artefact,
            hash,
            Some(FoldExclusion::Lineage),
            chrono::Utc::now(),
        )
        .unwrap();
        assert_eq!(one.pair_key(), other.pair_key());
        // And the direction is still readable off the row.
        assert_eq!(one.newcomer, a);
        assert_eq!(other.newcomer, b);

        // A freshly raised question is unanswered, and carries no
        // verdict to be read as one.
        assert!(one.is_open());
        assert!(one.resolution.is_none());

        // The reason a fold was declined rides on the row it was
        // declined for, and absence of one is a distinct statement
        // (nothing was declined) rather than a missing value.
        assert_eq!(one.fold_exclusion, None);
        assert_eq!(other.fold_exclusion, Some(FoldExclusion::Lineage));

        // A row against itself is not a pair. The lookup returns the
        // asset that was just fingerprinted along with the others, so
        // this is the mistake every caller can make once.
        assert!(
            DuplicateConflict::raise(
                persona,
                a,
                a,
                DuplicateAxis::Artefact,
                hash,
                None,
                chrono::Utc::now(),
            )
            .is_err()
        );
    }

    #[test]
    fn the_keeper_names_the_other_side_and_only_from_this_pair() {
        let a = AssetId::new();
        let b = AssetId::new();
        let outsider = AssetId::new();
        let conflict = DuplicateConflict::raise(
            PersonaId::new(),
            a,
            b,
            DuplicateAxis::Artefact,
            "sha256:abc",
            None,
            chrono::Utc::now(),
        )
        .unwrap();

        // Either side may be kept — the answer is a person's, not the
        // orientation's, so keeping the newcomer is as ordinary as
        // keeping the incumbent.
        assert_eq!(conflict.headstone_for(&a).unwrap(), b);
        assert_eq!(conflict.headstone_for(&b).unwrap(), a);

        // A third row is not an answer to this question. Accepting it
        // would fold two assets against a queue row that names neither.
        let refused = conflict.headstone_for(&outsider).unwrap_err();
        assert!(
            refused.to_string().contains("not part of this conflict"),
            "the refusal should say why: {refused}"
        );
    }
}
