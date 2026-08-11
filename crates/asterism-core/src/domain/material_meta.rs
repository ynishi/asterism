//! `material_meta` — the canonical form the metadata a container
//! carries *about* an artefact is rendered into, the digest taken over
//! that form, and what a reading of it can conclude.
//!
//! [`content_region`](crate::domain::content_region) is defined as the
//! bytes that survive into the decoded result, so **the metadata it
//! drops is the exact complement**, and until this module nothing
//! hashed that complement. The three axes stand in one relation:
//!
//! ```text
//!    Artefact  =  Content  +  Meta
//! ```
//!
//! `Artefact` agreement implies both of the others; neither of the
//! others implies anything about the rest. That is what makes this axis
//! worth carrying rather than folding into one of them — a generator
//! emits both shapes routinely. One picture re-exported with a caption
//! written into it is `Content` and not `Artefact`; a batch off one
//! workflow whose frames differ only by seed is `Meta` and not
//! `Content`.
//!
//! # The canonical form
//!
//! A **JSON object, key → value**, keys sorted, no whitespace — the
//! same rendering discipline
//! [`SourceLocator::to_storage`](crate::domain::source_locator::SourceLocator::to_storage)
//! follows, and for the same reason: the digest is an equality test on
//! the rendered string, so two equal metadata sets must render
//! identically. `serde_json` over a [`BTreeMap`] emits keys in order and
//! compact by default, so both properties are facts about the
//! declaration rather than about a `format!` string. **Nothing here
//! hand-writes JSON.**
//!
//! Two rules live inside the form, and both decide what the digest
//! *means*.
//!
//! ## Values stay as the container stated them — strings, unparsed
//!
//! A ComfyUI `parameters` chunk happens to hold JSON. Parsing it in
//! order to re-render it would put number formatting and nested key
//! order into the digest's definition, so two files the container calls
//! identical could stop matching on a serialiser's habits. The digest
//! says one thing: *the container carried this text*. The type is what
//! enforces it — the map is `BTreeMap<String, String>` end to end, and
//! there is no `from_str` anywhere on this path.
//!
//! If that proves too strict — the same workflow re-saved by a tool
//! that reformats — the answer is a **new prefix**, and it is written
//! down beside the prefix itself
//! ([`META_DIGEST_PREFIX`](crate::domain::content_hash::META_DIGEST_PREFIX)).
//!
//! ## Album's own fields never enter it
//!
//! Title, labels, `register_note`, ratings: those are what a person
//! wrote *here*, and a digest that moved when somebody renamed a
//! picture would be measuring the library rather than the artefact.
//! Structurally enforced by the input:
//! [`ArtefactProbe::meta_of`](crate::domain::probe::ArtefactProbe::meta_of)
//! takes the artefact's bytes and nothing else, so no library-side value
//! has a route in.
//!
//! # A digest is the entrance, not the body
//!
//! Exact equality is the wrong question for metadata on its own: a
//! batch off one workflow differs by a seed, and a digest over the
//! whole of it separates precisely the rows that belong together. The
//! hash answers "made identically" cheaply and indexably; the useful
//! question — "made the same way apart from *this*" — is a comparison
//! over the structured value. So both are stored: the digest is the
//! index, and [`MaterialMeta::canonical`] — the same bytes that were
//! hashed — is what a person reads and a field comparison walks.
//!
//! # Which containers are read, and how, is not decided here
//!
//! This module holds the form and the digest; the reading of any
//! particular container is one implementation per format behind
//! [`ArtefactProbe`](crate::domain::probe::ArtefactProbe). That includes
//! the questions a reader cannot avoid answering — which chunks or boxes
//! count as metadata at all, and how each decodes to a string — because
//! answering them carelessly redefines the axis rather than widening it,
//! and the argument for a given answer is an argument about a specific
//! container. A format no probe reads gets
//! [`ContentRegion`]-shaped markers rather than a digest, and falls back
//! to the artefact axis, which still works on it.
//!
//! [`ContentRegion`]: crate::domain::content_region::ContentRegion
//! [`BTreeMap`]: std::collections::BTreeMap

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::domain::content_hash::META_DIGEST_PREFIX;
use crate::domain::content_region::{EMPTY_SPAN, UNKNOWN_FORMAT, UNSUPPORTED_PREFIX};
use crate::domain::value::MimeType;

/// What a reading of an artefact's metadata concluded.
///
/// Three states rather than `Option<String>`, for the reason
/// [`ContentRegion`](crate::domain::content_region::ContentRegion)
/// gives: the two ways of having no digest lead somewhere different.
/// [`Unsupported`] means no probe looked at this container at all;
/// [`EmptySpan`] means one did and the container carries no metadata.
///
/// [`Unsupported`]: MaterialMeta::Unsupported
/// [`EmptySpan`]: MaterialMeta::EmptySpan
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterialMeta {
    /// A digest over the canonical form, and the form itself.
    ///
    /// The two travel together because they are one measurement: the
    /// digest is a hash *of* `canonical`, and a caller holding one
    /// without the other could store a digest whose body says something
    /// else.
    Digest {
        /// `m1-sha256:<64 lowercase hex>`.
        digest: String,
        /// The rendered object the digest was taken over.
        canonical: String,
    },
    /// No probe reads the metadata of this format, or the one that
    /// claimed it found the bytes were something else. The payload is
    /// the format's name as far as it is known — a declared mime, or
    /// [`UNKNOWN_FORMAT`].
    Unsupported(String),
    /// Walkable format, no metadata: a PNG with no `tEXt` chunk, or one
    /// whose chunk structure ended before it was complete.
    ///
    /// Not a digest over `{}`, for the reason the content axis refuses
    /// a digest over zero bytes: every metadata-less PNG in the library
    /// would share one value and land in a single duplicate group,
    /// each member unrelated to the next.
    EmptySpan,
}

impl MaterialMeta {
    /// The literal to store in the material's meta-digest column.
    pub fn stored_value(&self) -> String {
        match self {
            Self::Digest { digest, .. } => digest.clone(),
            Self::Unsupported(format) => format!("{UNSUPPORTED_PREFIX}{format}"),
            Self::EmptySpan => EMPTY_SPAN.to_string(),
        }
    }

    /// The canonical object, when there is one — the value stored
    /// beside the digest and read by a field comparison.
    ///
    /// `None` on both markers: there is no object, and writing `{}`
    /// for one would say the container was read and carried nothing,
    /// which is false of [`Unsupported`](Self::Unsupported).
    pub fn canonical(&self) -> Option<&str> {
        match self {
            Self::Digest { canonical, .. } => Some(canonical),
            _ => None,
        }
    }

    /// The digest, when there is one — for callers that must not act on
    /// a marker as though it were a fingerprint.
    pub fn digest(&self) -> Option<&str> {
        match self {
            Self::Digest { digest, .. } => Some(digest),
            _ => None,
        }
    }
}

/// Renders a metadata set into the canonical form — **the only place
/// the form is produced.**
///
/// `serde_json` over a [`BTreeMap`] gives sorted keys and no whitespace
/// without being asked, which is what makes the two properties the
/// digest depends on facts about the type rather than about a caller's
/// care. Values are written exactly as they arrived; see the module
/// doc for why that is the rule and not an omission.
///
/// Infallible in practice — a map of strings always serialises — and
/// the empty rendering (`{}`) is returned rather than a panic if it
/// ever were not, because a probe never hands this an empty map
/// ([`MaterialMeta::EmptySpan`] answers that case first).
pub fn render(fields: &BTreeMap<String, String>) -> String {
    serde_json::to_string(fields).unwrap_or_else(|_| "{}".to_string())
}

/// The digest of an already-rendered canonical form.
///
/// Split from [`render`] so that the one thing hashed is the one thing
/// stored: a caller cannot hash a map and store a differently rendered
/// string, because there is only one rendering and this takes it.
pub fn digest_of(canonical: &str) -> String {
    let digest = Sha256::digest(canonical.as_bytes());
    let mut value = String::with_capacity(META_DIGEST_PREFIX.len() + digest.len() * 2);
    value.push_str(META_DIGEST_PREFIX);
    for byte in digest {
        use std::fmt::Write;
        // Infallible for String.
        let _ = write!(value, "{byte:02x}");
    }
    value
}

/// The outcome for an artefact that is **not** going to be read — the
/// value a caller stores when every probe's
/// [`walks_meta`](crate::domain::probe::ProbeGates::walks_meta) said
/// no.
///
/// Split out so the label is computed in one place: a caller spelling
/// `format!("unsupported:{mime}")` itself would have to spell
/// [`UNKNOWN_FORMAT`] too, and a row skipped before the read would stop
/// matching one refused after it.
///
/// The word for an unnamed format is the one
/// [`content_region`](crate::domain::content_region) defines, and
/// deliberately so: one artefact refused on both axes should read the
/// same way in both columns.
pub fn unsupported_format(declared_mime: Option<&MimeType>) -> MaterialMeta {
    let claimed = declared_mime
        .map(MimeType::as_str)
        .filter(|m| !m.is_empty());
    MaterialMeta::Unsupported(claimed.unwrap_or(UNKNOWN_FORMAT).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::content_hash;

    fn mime(raw: &str) -> MimeType {
        MimeType::parse(raw)
    }

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    /// The form, stated as a literal: sorted keys, no whitespace, and
    /// values exactly as the container stated them.
    #[test]
    fn the_canonical_form_is_sorted_compact_and_unparsed() {
        // One value is a JSON document — the case the "strings,
        // unparsed" rule exists for. The map is a `BTreeMap`, so the
        // sort is the type's rather than the caller's.
        let workflow = r#"{"seed": 7, "cfg": 1.50}"#;
        let canonical = render(&fields(&[
            ("workflow", workflow),
            ("prompt", "a cat"),
            ("Software", "ComfyUI"),
        ]));

        assert_eq!(
            canonical,
            r#"{"Software":"ComfyUI","prompt":"a cat","workflow":"{\"seed\": 7, \"cfg\": 1.50}"}"#,
            "sorted by key, no whitespace, and the workflow's own spacing kept verbatim"
        );
        // The rule stated as the failure it prevents: re-rendering the
        // workflow through a JSON serialiser would drop the space after
        // `:` and normalise `1.50`, so a file the container calls
        // identical would stop matching.
        assert!(
            canonical.contains(r#"\"cfg\": 1.50"#),
            "the value is the container's text, not a re-serialisation: {canonical}"
        );

        let digest = digest_of(&canonical);
        assert!(digest.starts_with(META_DIGEST_PREFIX));
        assert_eq!(digest.len(), META_DIGEST_PREFIX.len() + 64);
        assert!(
            digest[META_DIGEST_PREFIX.len()..]
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );

        // An empty text is a value, and a key's absence is a different
        // set from a key with nothing in it.
        assert_eq!(render(&fields(&[("Empty", "")])), r#"{"Empty":""}"#);
        assert_ne!(
            digest_of(&render(&fields(&[("Empty", "")]))),
            digest_of(&render(&BTreeMap::new()))
        );
    }

    /// The three outcomes render to three different columns' worth of
    /// value, and only the digest carries a body.
    #[test]
    fn each_outcome_stores_its_own_literal_and_only_one_carries_an_object() {
        let canonical = render(&fields(&[("prompt", "a cat")]));
        let measured = MaterialMeta::Digest {
            digest: digest_of(&canonical),
            canonical: canonical.clone(),
        };
        assert_eq!(measured.stored_value(), digest_of(&canonical));
        assert_eq!(measured.canonical(), Some(canonical.as_str()));
        assert_eq!(measured.digest(), Some(digest_of(&canonical).as_str()));

        let unsupported = MaterialMeta::Unsupported("video/mp4".to_string());
        assert_eq!(unsupported.stored_value(), "unsupported:video/mp4");
        assert!(unsupported.canonical().is_none());
        assert!(unsupported.digest().is_none());

        assert_eq!(MaterialMeta::EmptySpan.stored_value(), EMPTY_SPAN);
        assert!(MaterialMeta::EmptySpan.canonical().is_none());
        assert!(MaterialMeta::EmptySpan.digest().is_none());
    }

    /// `EmptySpan` is not the digest of `{}`.
    ///
    /// The digest of the empty rendering is a perfectly real value, so a
    /// reading that produced one for every metadata-less file would put
    /// them all in a single duplicate group. It is reserved rather than
    /// produced, and this asserts the two strings are not the same one.
    #[test]
    fn the_empty_span_marker_is_not_the_digest_of_an_empty_object() {
        assert_eq!(
            digest_of("{}"),
            content_hash::META_EMPTY,
            "the reserved value is the digest of the empty rendering"
        );
        assert_ne!(MaterialMeta::EmptySpan.stored_value(), digest_of("{}"));
        assert!(
            !MaterialMeta::EmptySpan
                .stored_value()
                .starts_with(META_DIGEST_PREFIX)
        );
    }

    /// A meta digest reads as this axis and as neither of the others.
    ///
    /// Reading one across axes would claim an agreement nothing
    /// measured.
    #[test]
    fn a_meta_digest_cannot_be_read_as_another_axis() {
        use crate::domain::duplicate_conflict::DuplicateAxis;

        let digest = digest_of(&render(&fields(&[("prompt", "a cat")])));
        assert!(content_hash::is_duplicate_key(DuplicateAxis::Meta, &digest));
        for other in [DuplicateAxis::Artefact, DuplicateAxis::Content] {
            assert!(!content_hash::is_duplicate_key(other, &digest));
        }
    }

    /// The label for a file nothing is going to read, and the word it
    /// shares with the content axis.
    #[test]
    fn the_label_for_an_unread_file_matches_the_content_axis_word() {
        use crate::domain::content_region;

        assert_eq!(
            unsupported_format(Some(&mime("video/mp4"))),
            MaterialMeta::Unsupported("video/mp4".to_string())
        );

        for raw in [None, Some("   "), Some("video/mp4"), Some("text/plain")] {
            let parsed = raw.map(MimeType::parse);
            let declared = parsed.as_ref();
            assert_eq!(
                unsupported_format(declared).stored_value(),
                content_region::unsupported_format(declared).stored_value(),
                "{raw:?}: one artefact reads the same way in both columns"
            );
            assert!(
                unsupported_format(declared)
                    .stored_value()
                    .starts_with(UNSUPPORTED_PREFIX)
            );
        }
    }
}
