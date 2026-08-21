//! `content_region` — what a reading of "the bytes that decide what
//! this artefact decodes to" can conclude, and what each conclusion is
//! stored as.
//!
//! [`content_hash`](crate::domain::content_hash) fingerprints a whole
//! file, which answers "are these the same file". Two exports of one
//! picture that differ only in a `tEXt` chunk — the ComfyUI workflow
//! blob, an exporter's timestamp, a caption written on re-save — are
//! not the same file and get different digests, while being the same
//! picture pixel for pixel. The content axis answers that second
//! question, under the separate, versioned tag
//! [`CONTENT_DIGEST_PREFIX`](crate::domain::content_hash::CONTENT_DIGEST_PREFIX).
//!
//! # Vocabulary here, container reading elsewhere
//!
//! **This module holds no format knowledge and reads no bytes.** The
//! reading is a container parser over input an importer collected from
//! outside, with the failure modes a parser has, and it is one
//! implementation per format — so it lives behind
//! [`probe::ArtefactProbe`](crate::domain::probe::ArtefactProbe), with
//! the byte-level walking in `asterism-media-probe` and each format's
//! judgement about its own container in the adapters beside it. What is
//! left here is the part every format's answer has to be phrased in:
//! three outcomes, four reserved markers, and the rule for labelling a
//! format nothing walked.
//!
//! The split is what keeps a second format from widening the domain
//! layer. Adding one adds a probe and a registry line; the words a
//! column can carry do not move, which matters because they are already
//! written into a live database.
//!
//! [`content_hash`](crate::domain::content_hash) is the neighbouring
//! vocabulary — which prefixes exist, which values are reserved, what a
//! caller's declaration is allowed to say — and the prefix itself is
//! defined once, over there, and imported.
//!
//! # "No digest" is not one state
//!
//! [`ContentRegion`] has three, and the caller cannot collapse them by
//! accident because there is no `Option` to unwrap. A file no probe
//! handles is [`ContentRegion::Unsupported`] and falls back to the file
//! axis, which still works on it. A file some probe does handle but
//! could not read to a region is [`ContentRegion::EmptySpan`], and the
//! distinction matters more than it looks: hashing a region of zero
//! bytes produces a perfectly real digest — the well-known SHA-256 of
//! nothing — and writing it would put every truncated PNG in one
//! duplicate group, each unrelated to the next. Measured on the mp4
//! side, where fragmented files walk to zero samples and produced
//! exactly that collision.

use crate::domain::measurement::{Measurement, MeasurementStatus};
use crate::domain::value::MimeType;

/// The **legacy stored prefix** of "no digest on this axis" — what the
/// hash columns carried before V92 moved the distinction into a status
/// column ([`MeasurementStatus`]) and the format's name into a reason column.
///
/// No runtime writer produces it any more. It is kept because it is
/// written into live databases: the V92 conversion maps every value
/// under it, and the frozen migrations that wrote it (V55, V64, V75)
/// spell it in their own SQL.
pub const UNSUPPORTED_PREFIX: &str = "unsupported:";

/// Legacy stored spelling of [`MeasurementStatus::EmptySpan`]: a probe claimed
/// the format and its reading yielded no region — a PNG with no `IDAT`
/// chunk, or one whose chunk structure ended before it was complete.
///
/// A truncated file and a bomb both land there, and the status reads as
/// the thing they have in common: there is no complete region to stand
/// behind. Splitting them was considered and dropped — the diagnosis is
/// what the file axis and the file's size already give. Kept for the
/// V92 conversion and the frozen migrations that wrote it.
pub const EMPTY_SPAN: &str = "unsupported:empty-span";

/// Legacy stored spelling of [`MeasurementStatus::TooLarge`]: the format *is*
/// one a probe walks and the job declined to hand it the bytes, because
/// reading the file whole would have cost more memory than the job is
/// willing to spend
/// ([`walks_content`](crate::domain::probe::ProbeGates::walks_content)
/// explains why the size question reaches the job at all).
///
/// Its own status rather than a spelling of the other two because the
/// statements differ: a truncated PNG has no complete region and the
/// file is wrong; a file skipped for size has a region that could be
/// computed and nothing about the file is wrong. It is also the only
/// status a *policy* change clears rather than a change to the file.
/// Kept for the V92 conversion.
pub const TOO_LARGE: &str = "unsupported:too-large";

/// Legacy stored spelling of [`MeasurementStatus::NotWalked`]: written to the
/// content column of every material that existed before the column did
/// — **the unfinished half of a migration, named row by row.** Kept for
/// the V92 conversion and the frozen migrations that wrote it; the
/// reasoning below is the record of why the state exists at all.
///
/// Not a walk outcome: no [`ContentRegion`] maps to it and no probe
/// returns it. It is written once, by the schema migration that adds
/// the column.
///
/// # Why the migration writes it instead of computing the values
///
/// Adding a column is one statement. Filling this one in means reading
/// every original off disk, which cannot run inside the migration's
/// transaction and cannot be finished before the app is usable. So the
/// value half is deferred — and a deferred migration needs to record
/// *which rows it was deferred for*, or it cannot be finished later.
/// This marker is that record.
///
/// Leaving the column `NULL` would have recorded nothing. The predicate
/// that finds unfingerprinted materials ("the content column holds no
/// answer") matches `NULL`, so every pre-existing row would have gone to
/// the ordinary fingerprint walk — the pass for material that has just
/// arrived — and the first launch after the upgrade would have re-read
/// the whole corpus through it, indistinguishably from ordinary ingest
/// work, with the "still fingerprinting" notice describing a migration
/// nobody was told about. The marker says the true thing ("nothing
/// walked these bytes"), keeps the two passes apart, and leaves a set
/// the migration can select.
///
/// # What clears it
///
/// The **next step of the same migration chain**, which reads the files
/// and writes what it finds:
/// [`needs_content_walk`](crate::domain::content_hash::needs_content_walk)
/// selects exactly these rows and each one is replaced by a digest, or
/// by whichever marker the walk really produced. Finishing a migration
/// is the application's own responsibility rather than an errand handed
/// to the user, so nothing has to be started by hand, and the two steps
/// are never observed apart: no launch happens between them.
///
/// A row still carrying this afterwards is one whose original could not
/// be opened — moved, deleted, on a disk that was not connected. The
/// statement stays exactly true of it, and it keeps its file-axis digest
/// and its file-axis grouping, so what is missing is the improvement,
/// never a row.
///
/// The marker is written once more if the region definition is ever
/// versioned up (`cr2-`), and **that case does not get to reuse the
/// migration-time read**: it would make every row a target on a library
/// of any size, which a released build may not do to somebody who
/// installed an update. See
/// [`needs_content_walk`](crate::domain::content_hash::needs_content_walk).
pub const NOT_WALKED: &str = "unsupported:not-walked";

/// Format label used when a probe refused the bytes and nothing named
/// what they are.
///
/// Honest rather than guessed: a probe that has just failed its own
/// signature check knows the bytes are not its format and no more, and
/// this crate has no sniffing table to ask instead. The label's job is
/// to make the row readable, not to classify it — the value's meaning
/// is carried by [`UNSUPPORTED_PREFIX`].
///
/// Public because every probe reaches for it on that branch, and the
/// alternative is each of them spelling the word itself: a row refused
/// by one adapter would then stop matching a row refused by another,
/// on a difference nobody meant to introduce.
pub const UNKNOWN_FORMAT: &str = "unknown";

/// What a reading of an artefact's bytes concluded.
///
/// Three states rather than `Option<String>`, because the two ways of
/// having no digest lead somewhere different: [`Unsupported`] means the
/// file axis is the answer for this row, [`EmptySpan`] means this file
/// has no answer on either axis worth grouping on. A caller holding an
/// `Option` treats them alike by writing the shorter branch.
///
/// [`Unsupported`]: ContentRegion::Unsupported
/// [`EmptySpan`]: ContentRegion::EmptySpan
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentRegion {
    /// A digest over the region: `cr1-sha256:<64 lowercase hex>`.
    Digest(String),
    /// No probe walks this format, or the one that claimed it found the
    /// bytes were something else. The payload is the format's name as
    /// far as it is known — a declared mime, or [`UNKNOWN_FORMAT`].
    Unsupported(String),
    /// Walkable format, no region: no pixel chunk, or a structure that
    /// ended early.
    EmptySpan,
}

impl ContentRegion {
    /// What to store for this outcome — the status column's word, the
    /// digest column's value, the reason column's payload
    /// ([`Measurement`]). The marker strings the outcomes used to render
    /// to are the pre-V92 form; see the constants above.
    pub fn record(&self) -> Measurement {
        match self {
            Self::Digest(value) => Measurement::computed(value.clone()),
            Self::Unsupported(format) => Measurement::unsupported(format.clone()),
            Self::EmptySpan => Measurement::bare(MeasurementStatus::EmptySpan),
        }
    }

    /// The digest, when there is one — for callers that must not act on
    /// a marker as though it were a fingerprint.
    pub fn digest(&self) -> Option<&str> {
        match self {
            Self::Digest(value) => Some(value),
            _ => None,
        }
    }
}

/// The outcome for an artefact that is **not** going to be read — the
/// value a caller stores when every probe's
/// [`walks_content`](crate::domain::probe::ProbeGates::walks_content)
/// said no.
///
/// Split out so the label is computed in one place. The caller that
/// skips the read still has to write something, and writing
/// `format!("unsupported:{mime}")` on its side would be a second
/// implementation of the vocabulary: it would spell
/// [`UNKNOWN_FORMAT`] itself for the `None` case, so a row skipped
/// before the read and a row refused after one would stop matching.
///
/// An empty claim is no claim: a column holding `""` (or whitespace,
/// which parses to the same thing) says nothing about the format, and
/// treating it as a named one would label the row `unsupported:` with
/// nothing after the colon. A probe refusing the same artefact after
/// reading it has to discard the claim the same way — that agreement is
/// what the caller relies on when it skips the read, and the probes
/// assert it.
///
/// Normalisation used to live here too (`IMAGE/PNG; charset=binary`
/// and `image/png` are one claim) and nowhere else, which made this the
/// only consumer that agreed with itself about spelling. It now happens
/// once, in [`MimeType::parse`], so the label is built from a form
/// already canonical.
pub fn unsupported_format(declared_mime: Option<&MimeType>) -> ContentRegion {
    let claimed = declared_mime
        .map(MimeType::as_str)
        .filter(|m| !m.is_empty());
    ContentRegion::Unsupported(claimed.unwrap_or(UNKNOWN_FORMAT).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::content_hash;

    fn mime(raw: &str) -> MimeType {
        MimeType::parse(raw)
    }

    /// The three outcomes render to three different stored triples, and
    /// only one of them carries a digest.
    #[test]
    fn each_outcome_stores_its_own_record_and_only_one_is_a_digest() {
        let value = format!("{}{}", content_hash::CONTENT_DIGEST_PREFIX, "a".repeat(64));
        let digest = ContentRegion::Digest(value.clone());
        assert_eq!(digest.record(), Measurement::computed(value));
        assert!(digest.digest().is_some());

        let unsupported = ContentRegion::Unsupported("video/mp4".to_string());
        assert_eq!(
            unsupported.record(),
            Measurement::unsupported("video/mp4".to_string())
        );
        assert!(unsupported.digest().is_none());

        assert_eq!(
            ContentRegion::EmptySpan.record(),
            Measurement::bare(MeasurementStatus::EmptySpan)
        );
        assert!(ContentRegion::EmptySpan.digest().is_none());
    }

    /// Every marker sits under the one prefix duplicate grouping asks
    /// about, and none of them can be read as a digest.
    ///
    /// The failure this prevents is a query that has to enumerate the
    /// markers: a fifth one added later would silently join the digests
    /// until somebody remembered to edit the `WHERE`.
    #[test]
    fn every_marker_is_excluded_by_the_one_prefix_rule() {
        for marker in [
            EMPTY_SPAN,
            TOO_LARGE,
            NOT_WALKED,
            &format!("{UNSUPPORTED_PREFIX}{UNKNOWN_FORMAT}"),
            "unsupported:image/jpeg",
        ] {
            assert!(marker.starts_with(UNSUPPORTED_PREFIX), "{marker}");
            assert!(!marker.starts_with(content_hash::CONTENT_DIGEST_PREFIX));
            assert!(!content_hash::is_duplicate_key(
                crate::domain::duplicate_conflict::DuplicateAxis::Content,
                marker
            ));
        }

        // …and the four are four, rather than spellings of one. The
        // count said four while the loop held three, so the fourth was
        // brought in rather than the sentence cut down: it is
        // `unsupported:unknown`, the one value above that is not a
        // constant here but a label built for a format nobody named.
        //
        // It belongs because the collision it rules out is real. The
        // three constants and that label are cleared by different
        // things — a re-walk, a policy change, a probe arriving, a
        // better guess at the format — so two of them sharing a spelling
        // would put rows waiting on different events into one bucket
        // that no single event empties. `unsupported:image/jpeg` stays
        // out: it stands for the whole `unsupported:<mime>` family,
        // whose members are distinct because mimes are.
        let unnamed = format!("{UNSUPPORTED_PREFIX}{UNKNOWN_FORMAT}");
        let markers = [EMPTY_SPAN, TOO_LARGE, NOT_WALKED, unnamed.as_str()];
        for (at, one) in markers.iter().enumerate() {
            for other in &markers[at + 1..] {
                assert_ne!(one, other);
            }
        }
    }

    /// The `EmptySpan` marker is not the digest of an empty region.
    ///
    /// A digest over zero bytes is a perfectly real value — the
    /// well-known SHA-256 of nothing — so a walker that produced one for
    /// every truncated file would put them all in a single duplicate
    /// group. This asserts the two are not the same string.
    #[test]
    fn the_empty_span_marker_is_not_the_digest_of_nothing() {
        let empty_region = format!(
            "{}{}",
            content_hash::CONTENT_DIGEST_PREFIX,
            content_hash::EMPTY
                .strip_prefix(content_hash::DIGEST_PREFIX)
                .expect("the empty digest carries its algorithm")
        );
        assert_ne!(EMPTY_SPAN, empty_region);
        assert!(!EMPTY_SPAN.starts_with(content_hash::CONTENT_DIGEST_PREFIX));
    }

    /// The label for a file nothing is going to read: the claim when
    /// there is one, [`UNKNOWN_FORMAT`] when there is not.
    ///
    /// The empty claim is the case worth having a test for. A column
    /// holding `""` or whitespace says nothing about the format, and
    /// treating it as a name would store `unsupported:` with nothing
    /// after the colon — a value that reads as a format called "".
    #[test]
    fn the_label_for_an_unread_file_is_the_claim_or_the_honest_word() {
        assert_eq!(
            unsupported_format(Some(&mime("video/mp4"))),
            ContentRegion::Unsupported("video/mp4".to_string())
        );
        // A parameterised or shouted mime is the same claim — settled at
        // the parse boundary, not by this function's own normalisation.
        assert_eq!(
            unsupported_format(Some(&mime("IMAGE/JPEG; charset=binary"))),
            ContentRegion::Unsupported("image/jpeg".to_string())
        );

        for raw in [None, Some("   "), Some("")] {
            let parsed = raw.map(MimeType::parse);
            assert_eq!(
                unsupported_format(parsed.as_ref()),
                ContentRegion::Unsupported(UNKNOWN_FORMAT.to_string()),
                "{raw:?}"
            );
            assert_eq!(
                unsupported_format(parsed.as_ref())
                    .record()
                    .reason
                    .as_deref(),
                Some(UNKNOWN_FORMAT),
                "{raw:?}: never a reason that says nothing"
            );
        }
    }
}
