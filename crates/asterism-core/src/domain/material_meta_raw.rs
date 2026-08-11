//! `material_meta_raw` — the container's metadata bytes, kept verbatim,
//! so that the rule which expands them can be rewritten afterwards.
//!
//! [`material_meta`](crate::domain::material_meta) renders what a probe
//! read into the canonical form and hashes it. This is the other half of
//! the same axis: the bytes that rendering was made *from*, stored as
//! they sat in the file. One is the answer, the other is the question it
//! was derived from.
//!
//! # What the rendering loses, measured
//!
//! `meta_kv` looks lossless for PNG — the `tEXt` values go into it as
//! text — and it is not, in two ways that were measured rather than
//! imagined:
//!
//! - **`tEXt` is Latin-1 by the spec** and the reader takes it through
//!   [`String::from_utf8_lossy`], which is a total function with one
//!   answer per input and therefore the right thing for a digest. An
//!   accented byte becomes `\u{fffd}` and **the original byte is gone**
//!   (`asterism_media_probe::png::text_fields` states this where it
//!   happens).
//! - **`zTXt` / `iTXt` / `tIME` / `eXIf` are not read at all.** They are
//!   outside the content region and outside the meta digest, which the
//!   PNG probe's own doc calls a stated gap that an `m2-` generation
//!   would close.
//!
//! Both are recoverable from the raw **without opening the file again**,
//! which is the whole of why this column is worth its bytes. Changing
//! how metadata is expanded is otherwise a re-read of somebody's entire
//! library, and that is a decision about their disk rather than a
//! consequence of shipping (the argument is written out at
//! [`needs_content_walk`](crate::domain::content_hash::needs_content_walk)).
//!
//! # Why a second column and not another key
//!
//! `meta_kv` **is** the digest's input — `canonical = render(fields)`,
//! `meta_hash = digest_of(canonical)` — so a key added there moves every
//! `m1-` value in the library, including the digests frozen as literals
//! in this workspace. The raw has to sit beside it or it redefines the
//! axis it exists to make revisable.
//!
//! # The stored vocabulary
//!
//! ```text
//! undefined:<base64>          the bytes
//! unsupported:too-large       a probe read them and the policy declined to keep them
//! unsupported:not-captured    the build that read this row kept no bytes
//! NULL                        nothing here keeps bytes for this format
//! ```
//!
//! **`undefined:` is the point of the prefix.** A value under it carries
//! bytes and *no claim about what they mean*: it is not a digest and not
//! a rendering, and the expansion rule that would give it meaning is
//! exactly the thing this column exists to let somebody replace. A
//! prefix naming a reading (`png-chunks:`, `exif:`) would be that claim,
//! and the first reading to change would leave every row labelled with
//! the one it was written under. What the prefix does have to do is
//! separate a payload from a statement, so that a reader never takes
//! `unsupported:too-large` for content.
//!
//! Base64 because the column is `TEXT` and the bytes are a container's,
//! which is to say arbitrary. The standard alphabet with canonical
//! padding, the same one [`series`](crate::domain::series) decodes a
//! character card with, so there is one answer in this workspace to what
//! base64 means.
//!
//! # Which bytes those are is not decided here
//!
//! Per format, by the probe that reads the container, on the same terms
//! as every other judgement about a corpus
//! ([`ArtefactProbe`](crate::domain::probe::ArtefactProbe)). "How much of
//! this container counts as metadata" has no answer that is true of every
//! file, and a ceiling on how much of it is worth keeping is a statement
//! about one format's structure — JPEG's `APP1` cannot exceed 64 KiB
//! because its segment length is two bytes, and PNG has no equivalent
//! bound, so PNG's probe states one.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::domain::content_region::TOO_LARGE;

/// The prefix on a value that carries bytes rather than a statement
/// about them — see the module doc for why the word is `undefined`.
pub const RAW_PREFIX: &str = "undefined:";

/// The value for a row a build that kept no bytes had already read.
///
/// **No reading produces it.** It is written once, by the migration that
/// adds the column, over every row that was in the library at that
/// moment: those rows hold a correct `meta_kv` and were fingerprinted by
/// a build with no raw to keep, and NULL would have said something else
/// — "nothing here keeps bytes for this format", which is what a video
/// legitimately stores.
///
/// The distinction is the only thing that makes the deferred set
/// selectable later. Whoever writes the pass that fills these in reads
/// this value and no other, the way
/// [`needs_content_walk`](crate::domain::content_hash::needs_content_walk)
/// reads `unsupported:not-walked` — and **that pass does not exist**.
/// Nothing in this build ever revisits a row carrying this, because the
/// bytes can only be got by reading the file and a released application
/// may not answer an update by reading somebody's whole disk.
pub const NOT_CAPTURED: &str = "unsupported:not-captured";

/// What a reading of a container's metadata bytes concluded.
///
/// Three states rather than `Option<Vec<u8>>`, for the reason
/// [`MaterialMeta`](crate::domain::material_meta::MaterialMeta) gives
/// about its own two markers: the ways of having no bytes lead somewhere
/// different. [`Absent`] means nothing here keeps bytes for this format,
/// and the column stays NULL; [`TooLarge`] means a probe found them and
/// the policy declined to spend the room, which is a fact about this
/// build's ceiling rather than about the file.
///
/// [`Absent`]: MetaRaw::Absent
/// [`TooLarge`]: MetaRaw::TooLarge
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaRaw {
    /// The metadata bytes, exactly as the container carried them.
    ///
    /// Owned rather than borrowed because a container's metadata is
    /// rarely contiguous — PNG carries chunks on both sides of its pixel
    /// data — so producing this means concatenating, and there is no
    /// slice of the input to hand back.
    Captured(Vec<u8>),
    /// A probe read the container and the bytes were past the ceiling it
    /// states. The marker is the content axis's word
    /// ([`TOO_LARGE`]) because it is the same sentence: nothing about
    /// the file is wrong and the policy declined to spend the room.
    TooLarge,
    /// Nothing here keeps bytes for this format — no probe claims the
    /// metadata axis for it, or the bytes were refused before a reading
    /// began.
    Absent,
}

impl MetaRaw {
    /// The literal to store in `material.meta_raw`, or `None` for the
    /// column staying NULL.
    ///
    /// The encoding happens here rather than in the probe that produced
    /// the bytes, so there is one place in the tree that decides what
    /// this column's payload looks like. A probe decides *which* bytes;
    /// how they are written down is the column's business.
    pub fn stored_value(&self) -> Option<String> {
        match self {
            Self::Captured(bytes) => Some(format!("{RAW_PREFIX}{}", BASE64.encode(bytes))),
            Self::TooLarge => Some(TOO_LARGE.to_string()),
            Self::Absent => None,
        }
    }

    /// The bytes, when there are some — for a caller that must not treat
    /// a marker as a payload.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Captured(bytes) => Some(bytes),
            _ => None,
        }
    }
}

/// Reads a stored value back into the bytes it carries — **the whole
/// reason the column exists.**
///
/// Without this the raw is bytes nobody can use, so it is written here
/// beside the encoding rather than left for each consumer to spell: two
/// implementations of "strip the prefix and decode" is two chances to
/// disagree about the alphabet, and the one that drifts writes nothing
/// down.
///
/// `None` on every marker, on a value under no prefix this module wrote,
/// and on a payload that is not standard base64 with canonical padding.
/// A row whose column was hand-edited is a row nobody can make claims
/// about, and answering with a truncated decode would let a caller act
/// on part of a container's metadata as though it were all of it.
pub fn bytes_of(stored: &str) -> Option<Vec<u8>> {
    BASE64.decode(stored.strip_prefix(RAW_PREFIX)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::content_region::UNSUPPORTED_PREFIX;

    /// **The round trip, at the level the column is defined.**
    ///
    /// Everything above the column — a probe selecting chunks, a job
    /// writing a row — is worth nothing if the bytes cannot come back,
    /// so this is asserted on the two shapes that break a careless
    /// encoding: bytes that are not UTF-8 (the Latin-1 `tEXt` the
    /// rendering loses) and a length that is not a multiple of three
    /// (where padding is decided).
    #[test]
    fn the_bytes_come_back_exactly() {
        for (label, bytes) in [
            ("empty", Vec::new()),
            ("one byte, so the padding is two", vec![0xe9]),
            ("two bytes, so the padding is one", vec![0xe9, 0xff]),
            ("three bytes, so there is none", vec![0x00, 0x7f, 0xff]),
            ("not UTF-8 at any length", (0u8..=255).collect()),
        ] {
            let stored = MetaRaw::Captured(bytes.clone())
                .stored_value()
                .expect("captured bytes are a stored value");
            assert!(stored.starts_with(RAW_PREFIX), "{label}: {stored}");
            assert_eq!(bytes_of(&stored), Some(bytes.clone()), "{label}");
        }

        // The control: a payload that decodes to something else is a
        // different stored value. Without it the assertions above are
        // satisfied by an encoding that throws the input away.
        assert_ne!(
            MetaRaw::Captured(vec![1]).stored_value(),
            MetaRaw::Captured(vec![2]).stored_value()
        );
    }

    /// A marker is not a payload, in both directions.
    #[test]
    fn a_marker_carries_no_bytes_and_does_not_decode_as_any() {
        assert_eq!(
            MetaRaw::TooLarge.stored_value(),
            Some(TOO_LARGE.to_string())
        );
        assert_eq!(MetaRaw::Absent.stored_value(), None);
        assert!(MetaRaw::TooLarge.bytes().is_none());
        assert!(MetaRaw::Absent.bytes().is_none());

        // Every value that is not this module's payload reads as no
        // bytes — including the one the migration writes, which no
        // reading here produces.
        for marker in [
            TOO_LARGE,
            NOT_CAPTURED,
            "unsupported:image/jpeg",
            "m1-sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
            "",
        ] {
            assert_eq!(bytes_of(marker), None, "{marker}");
        }

        // A statement and a payload are told apart by the prefix, so the
        // two families must not share one.
        assert!(TOO_LARGE.starts_with(UNSUPPORTED_PREFIX));
        assert!(NOT_CAPTURED.starts_with(UNSUPPORTED_PREFIX));
        assert!(!RAW_PREFIX.starts_with(UNSUPPORTED_PREFIX));
        assert!(!NOT_CAPTURED.starts_with(RAW_PREFIX));
    }

    /// A hand-edited payload answers `None` rather than a truncated
    /// decode — the failure being refused is a caller reading part of a
    /// container's metadata as though it were all of it.
    #[test]
    fn a_payload_that_is_not_canonical_base64_is_not_read() {
        for broken in [
            "undefined:not base64 !@#",
            // One character short of a group: the standard engine wants
            // canonical padding.
            "undefined:AAAAA",
            "undefined:AA=A",
        ] {
            assert_eq!(bytes_of(broken), None, "{broken}");
        }
    }
}
