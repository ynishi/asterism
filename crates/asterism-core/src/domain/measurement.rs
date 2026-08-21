//! `measurement` — what a fingerprint column's *status* can say, now
//! that the digest columns hold only digests.
//!
//! The name is the issue's own vocabulary: the whole defect was that a
//! reader could not tell a **measurement** from a note about why there
//! is no measurement, and these types are the two halves of that
//! sentence made storable — [`Measurement`] is one axis's stored
//! triple, [`MeasurementStatus`] the word that says which of the two
//! it is holding.
//!
//! The vocabulary the three hash columns used to carry inline — a
//! digest, or a marker string explaining why there is none
//! (`unsupported:<mime>`, `unsupported:empty-span`,
//! `unhashable:no-bytes`, …) — is split in two. The digest column holds
//! a digest or NULL; a status column beside it holds one of the words
//! below, non-nullable; and a reason column holds the free-text part
//! (the media type, an I/O error) where a status has one.
//!
//! # Why the marker vocabulary moved out of the value slot
//!
//! Every reader of the old columns had to know the marker grammar
//! before it could tell a measurement from a note about why there is no
//! measurement — `is_duplicate_key`, `is_axis_answer`,
//! `needs_fingerprint` and `needs_content_walk` all existed to make
//! that distinction, two of them restated in SQL. The established shape
//! for the same distinction elsewhere (`getxattr(2)`'s `ENOTSUP` /
//! `ENODATA` / value) is a nullable payload beside a non-nullable
//! status, and the marker-in-the-value design was safe only for as long
//! as the column never crossed an application boundary. Issue #17 is
//! the record of that decision.
//!
//! # One vocabulary, three columns
//!
//! The three axes share one status set for the reason they shared one
//! marker set: most of these words say something about the artefact
//! rather than about which measurement was attempted. Which subset a
//! writer can produce differs — the file axis streams and never walks,
//! so it only ever says `pending` / `computed` / `no-bytes` / `failed`
//! — but a reader faces one closed set wherever it looks.

use crate::error::DomainError;

/// What one fingerprint axis's status column says about the digest
/// column beside it.
///
/// Exactly one of these is stored per axis per material, as the
/// [`as_str`](Self::as_str) spelling. The digest column holds digests
/// and nothing else — no marker string survives in it — but "digest"
/// and "`computed`" are not quite one test: a digest can sit under a
/// non-`computed` status when it is a superseded-generation
/// measurement the V92 conversion (or a later version bump) declined
/// to destroy, kept for the walk to overwrite. A fresh write from the
/// fingerprint pass does pair them exactly. The reason column is
/// populated under [`Unsupported`](Self::Unsupported) (the format's
/// name) and [`Failed`](Self::Failed) (the I/O error), and NULL
/// elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasurementStatus {
    /// Nobody has looked yet. The state a material is inserted in, and
    /// the one the fingerprint walk exists to drain.
    Pending,
    /// The bytes were read and the digest column holds what they hash
    /// to. The only status a fresh write pairs a digest with — the
    /// type-level doc above says why a stored row can differ.
    Computed,
    /// No probe reads this format — or the one that claimed it found
    /// the bytes were something else. The reason column carries the
    /// format's name as far as it is known (a declared mime, or
    /// `unknown`). A final answer: it changes when a probe lands, and
    /// the migration that ships the probe flips these rows back to
    /// [`Pending`](Self::Pending) (V72 and V76 did exactly that for
    /// JPEG, one marker generation earlier).
    Unsupported,
    /// A probe claimed the format and walked to nothing: no pixel
    /// chunk, or a structure that ended early. A truncated file and a
    /// bomb both land here; the file's size tells them apart.
    EmptySpan,
    /// The format is walkable and the job declined to hand the probe
    /// the bytes, because reading the file whole would cost more memory
    /// than the job spends. The one status a *policy* change clears
    /// rather than a change to the file.
    TooLarge,
    /// The unfinished half of a deferred migration, named row by row:
    /// the schema step that added a versioned column wrote this over
    /// every pre-existing row, and the data step that follows selects
    /// exactly these and replaces each with what the bytes say. A row
    /// still carrying it afterwards is one whose original could not be
    /// opened.
    NotWalked,
    /// There are no bytes to read and never will be: a record inside a
    /// container file, or a locator that is not on this disk. A final
    /// answer — the row keeps whatever axes were answerable and leaves
    /// the walk.
    NoBytes,
    /// The bytes should be there and could not be read: a file that has
    /// moved, a disk that was not plugged in. The reason column carries
    /// the I/O error. **Not** a final answer — the walk retries these
    /// on every pass, because the answer can change when the disk comes
    /// back — but not open work either: the progress count excludes
    /// them, and they are surfaced as their own number instead
    /// (`unreadable_material_count`). Recording the failure is what
    /// lets the "still fingerprinting" notice reach zero while the
    /// retry stays visible instead of silent.
    Failed,
}

impl MeasurementStatus {
    /// Every status, for tests and adapters that need to walk the set.
    pub const ALL: &[MeasurementStatus] = &[
        MeasurementStatus::Pending,
        MeasurementStatus::Computed,
        MeasurementStatus::Unsupported,
        MeasurementStatus::EmptySpan,
        MeasurementStatus::TooLarge,
        MeasurementStatus::NotWalked,
        MeasurementStatus::NoBytes,
        MeasurementStatus::Failed,
    ];

    /// The stored spelling — what the status column holds and what SQL
    /// conditions compare against.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Computed => "computed",
            Self::Unsupported => "unsupported",
            Self::EmptySpan => "empty-span",
            Self::TooLarge => "too-large",
            Self::NotWalked => "not-walked",
            Self::NoBytes => "no-bytes",
            Self::Failed => "failed",
        }
    }

    /// Reads a stored spelling back. Refused rather than degraded, like
    /// `role` and `on_duplicate`: the set is closed, and a value this
    /// build does not name is a row written by a build this one does
    /// not understand — carrying it as some default would silently
    /// reclassify it.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        Self::ALL
            .iter()
            .copied()
            .find(|status| status.as_str() == raw)
            .ok_or_else(|| {
                DomainError::Validation(format!("unknown fingerprint axis status {raw:?}"))
            })
    }
}

/// One axis's stored triple: the status, the digest when there is one,
/// and the reason when the status carries one.
///
/// What [`ContentRegion`](crate::domain::content_region::ContentRegion)
/// and [`MaterialMeta`](crate::domain::material_meta::MaterialMeta)
/// render to for storage, and what
/// [`MaterialFingerprint`](crate::domain::repository::MaterialFingerprint)
/// carries per axis — one shape for "what the columns hold", so a
/// writer cannot store a digest under a status that says there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurement {
    /// The status column's value.
    pub status: MeasurementStatus,
    /// The digest column's value — `Some` exactly when `status` is
    /// [`Computed`](MeasurementStatus::Computed), **as a fresh write**. A
    /// stored row can hold the pair a version bump leaves behind
    /// (a superseded digest under a non-`computed` status); see the
    /// type-level doc on [`MeasurementStatus`].
    pub digest: Option<String>,
    /// The reason column's value — the format under
    /// [`Unsupported`](MeasurementStatus::Unsupported), the I/O error under
    /// [`Failed`](MeasurementStatus::Failed), `None` elsewhere.
    pub reason: Option<String>,
}

impl Measurement {
    /// A digest that was computed.
    pub fn computed(digest: String) -> Self {
        Self {
            status: MeasurementStatus::Computed,
            digest: Some(digest),
            reason: None,
        }
    }

    /// A status with no digest and no reason.
    pub const fn bare(status: MeasurementStatus) -> Self {
        Self {
            status,
            digest: None,
            reason: None,
        }
    }

    /// A format nothing reads, named as far as it is known.
    pub fn unsupported(format: String) -> Self {
        Self {
            status: MeasurementStatus::Unsupported,
            digest: None,
            reason: Some(format),
        }
    }

    /// A read that failed, carrying what the I/O layer said.
    pub fn failed(reason: String) -> Self {
        Self {
            status: MeasurementStatus::Failed,
            digest: None,
            reason: Some(reason),
        }
    }

    /// The digest, when there is one — for callers that must not act
    /// on a status as though it were a fingerprint.
    pub fn digest(&self) -> Option<&str> {
        self.digest.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_round_trips_through_its_stored_spelling() {
        for status in MeasurementStatus::ALL.iter().copied() {
            assert_eq!(MeasurementStatus::parse(status.as_str()).unwrap(), status);
        }
        // The set is closed: an unknown spelling is refused, not
        // defaulted — a default would silently reclassify a row
        // written by a build this one does not know.
        assert!(MeasurementStatus::parse("unsupported:video/mp4").is_err());
        assert!(MeasurementStatus::parse("").is_err());
        assert!(MeasurementStatus::parse("PENDING").is_err());
    }

    #[test]
    fn no_stored_spelling_needs_quoting_in_sql() {
        // The spellings are interpolated into SQL conditions as quoted
        // literals; one that grew an apostrophe would end the string
        // early. Same contract the digest prefixes hold for GLOB.
        for status in MeasurementStatus::ALL.iter().copied() {
            assert!(!status.as_str().contains('\''), "{}", status.as_str());
            assert!(!status.as_str().is_empty());
        }
    }

    #[test]
    fn the_constructors_pair_payloads_with_the_statuses_that_carry_them() {
        let computed = Measurement::computed("sha256:abc".into());
        assert_eq!(computed.status, MeasurementStatus::Computed);
        assert_eq!(computed.digest(), Some("sha256:abc"));
        assert_eq!(computed.reason, None);

        let unsupported = Measurement::unsupported("video/mp4".into());
        assert_eq!(unsupported.status, MeasurementStatus::Unsupported);
        assert_eq!(unsupported.digest(), None);
        assert_eq!(unsupported.reason.as_deref(), Some("video/mp4"));

        let failed = Measurement::failed("No such file or directory".into());
        assert_eq!(failed.status, MeasurementStatus::Failed);
        assert_eq!(failed.digest(), None);

        let bare = Measurement::bare(MeasurementStatus::EmptySpan);
        assert_eq!(bare.digest(), None);
        assert_eq!(bare.reason, None);
    }
}
