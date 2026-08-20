//! `DigitalSourceType` — the one field a synthetic file is obliged to
//! carry, and the closed set of values this corpus can honestly assert.
//!
//! The vocabulary is IPTC's, published as a controlled vocabulary at
//! `cv.iptc.org/newscodes/digitalsourcetype`. The *value* is the term's
//! URI, not its short name: that is what
//! `Iptc4xmpExt:DigitalSourceType` is typed as (URI, per the IPTC Photo
//! Metadata Standard 2025.1 specification), and it is what a C2PA
//! `c2pa.actions` assertion carries in its `digitalSourceType` field.
//! One string serves both, which is the reason this type exists at all
//! rather than each emitter spelling its own.
//!
//! # Why a closed set, when `Modality` next door is an open slug
//!
//! An open slug is right when a new value is a data change with no code
//! behind it. This is the opposite case twice over. Each value here is a
//! *claim about how a file came to exist*, and the claims are not
//! interchangeable — the difference between `trainedAlgorithmicMedia`
//! and `compositeWithTrainedAlgorithmicMedia` is the difference between
//! "a model made this" and "a model altered a photograph", and a caller
//! that could pass an arbitrary string could assert either by typo.
//! Second, the receiving side is closed too: a validator reads the URI
//! against the published vocabulary, and a term IPTC does not define is
//! not a weaker claim, it is an unreadable one.
//!
//! # Why these five and not the whole vocabulary
//!
//! IPTC defines more terms than this — film and print digitisation,
//! several composite forms, a deprecated pair. A term is here when a
//! file in this corpus can arrive with the fact it names, because a
//! value nothing can produce is a value nothing tests. The three
//! digitisation terms (`negativeFilm` / `positiveFilm` / `print`)
//! describe a scanner Asterism has no knowledge of; they can be added
//! when something can establish them, and adding one is this enum plus
//! its URI.
//!
//! # Not asserting is a state
//!
//! There is no `Unknown` variant. An artefact whose origin nothing
//! established gets no `DigitalSourceType` property at all — the same
//! reading
//! [`attribution`](crate::domain::attribution)
//! gives an absent author: absence is a question nobody has
//! answered, and a vocabulary term meaning "we do not know" would be an
//! answer. It also matters legally in the one direction that is not
//! symmetric: a missing mark on a synthetic file is a gap, while a wrong
//! mark on one is a false statement.

use std::fmt;

/// Prefix every IPTC digital source type URI shares.
///
/// Read by [`parse`][DigitalSourceType::parse], which strips it to
/// recover the term. [`uri`] does
/// **not** build on it — a `const` cannot be concatenated with another
/// at compile time without a macro over literals, and returning
/// `&'static str` is worth more here than sharing one string, so the
/// five URIs are written out in full.
///
/// Which means the agreement between them and this constant is a
/// property held by `every_term_sits_under_the_iptc_vocabulary` rather
/// than by construction. The doc here used to claim the second: that
/// the terms "cannot disagree about the host". They can — a typo in one
/// of the five literals compiles — and what catches it is a test.
///
/// [`uri`]: DigitalSourceType::uri
const IPTC_CV: &str = "http://cv.iptc.org/newscodes/digitalsourcetype/";

/// How a file came to exist, in IPTC's vocabulary.
///
/// See the module docs for why the set is closed and why these five.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DigitalSourceType {
    /// A generative model produced the file. The ordinary case for this
    /// corpus, and the value EU AI Act Article 50 disclosure turns on.
    TrainedAlgorithmicMedia,
    /// A generative model altered material that did not come from one —
    /// img2img over a photograph, inpainting on a scan. The claim is
    /// weaker than [`TrainedAlgorithmicMedia`](Self::TrainedAlgorithmicMedia)
    /// and stronger than [`HumanEdits`](Self::HumanEdits), and Asterism
    /// can tell the difference only when a derivation edge says the
    /// parent was not itself synthetic.
    CompositeWithTrainedAlgorithmicMedia,
    /// An algorithm produced the file without a trained model — a
    /// procedural render, a plotted figure, a synthesised test fixture.
    /// Distinct from the two above because no training data stands
    /// behind it, which is the distinction the vocabulary exists to
    /// make.
    AlgorithmicMedia,
    /// A camera or recording device captured the file from something
    /// real. Asterism asserts this only when a probe found capture
    /// metadata; it is never the default for "we saw no generator".
    DigitalCapture,
    /// A human edited the file with tools that are not generative.
    /// Asserted, never inferred — see the record's own docs on why a
    /// human pass cannot be derived from the absence of a machine one.
    HumanEdits,
}

impl DigitalSourceType {
    /// The term's URI — the value both emitters write.
    pub fn uri(&self) -> &'static str {
        match self {
            Self::TrainedAlgorithmicMedia => {
                "http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia"
            }
            Self::CompositeWithTrainedAlgorithmicMedia => {
                "http://cv.iptc.org/newscodes/digitalsourcetype/compositeWithTrainedAlgorithmicMedia"
            }
            Self::AlgorithmicMedia => {
                "http://cv.iptc.org/newscodes/digitalsourcetype/algorithmicMedia"
            }
            Self::DigitalCapture => "http://cv.iptc.org/newscodes/digitalsourcetype/digitalCapture",
            Self::HumanEdits => "http://cv.iptc.org/newscodes/digitalsourcetype/humanEdits",
        }
    }

    /// The term's short name — the last path segment of [`uri`](Self::uri).
    ///
    /// Not written into any file: both emitters carry the URI. This is
    /// for the places a person reads the value (a log line, an API
    /// response, a settings field), where the full URI is eight times
    /// the length and adds nothing.
    pub fn term(&self) -> &'static str {
        self.uri()
            .rsplit('/')
            .next()
            .expect("every term URI has a last segment")
    }

    /// Reads a stored value, accepting either spelling.
    ///
    /// Both are accepted because both are what arrives: the URI is what
    /// a file carries and what one Asterism instance writes for another
    /// to read back, while the short term is what a person types into a
    /// settings field or an override. Refusing either would make one of
    /// those two paths a trap, and they cannot collide — a term never
    /// contains a slash.
    ///
    /// Unknown values are refused rather than mapped to a default, for
    /// the reason
    /// [`ClaimRelation::parse`](crate::domain::provenance::ClaimRelation::parse)
    /// refuses its own: defaulting turns a typo into an assertion
    /// somebody has to disprove.
    pub fn parse(value: &str) -> Result<Self, UnknownSourceType> {
        let value = value.trim();
        let term = match value.strip_prefix(IPTC_CV) {
            Some(term) => term,
            // A URI under some other authority is not a term this
            // vocabulary defines, however familiar its last segment
            // looks. Only a bare term (no slash at all) falls through
            // to the match below.
            None if value.contains('/') => {
                return Err(UnknownSourceType {
                    value: value.to_string(),
                });
            }
            None => value,
        };
        match term {
            "trainedAlgorithmicMedia" => Ok(Self::TrainedAlgorithmicMedia),
            "compositeWithTrainedAlgorithmicMedia" => {
                Ok(Self::CompositeWithTrainedAlgorithmicMedia)
            }
            "algorithmicMedia" => Ok(Self::AlgorithmicMedia),
            "digitalCapture" => Ok(Self::DigitalCapture),
            "humanEdits" => Ok(Self::HumanEdits),
            _ => Err(UnknownSourceType {
                value: value.to_string(),
            }),
        }
    }

    /// Whether the term claims a generative model was involved.
    ///
    /// The question Article 50 asks — "is this synthetic content" — and
    /// the one a reader of a mixed corpus asks first. Kept here rather
    /// than as a `matches!` at each call site so that adding a term
    /// forces an answer for it.
    pub fn is_synthetic(&self) -> bool {
        match self {
            Self::TrainedAlgorithmicMedia | Self::CompositeWithTrainedAlgorithmicMedia => true,
            // Not synthetic in the sense the obligation means: no
            // trained model stands behind any of the three. An
            // algorithmic render is machine-made and discloses itself as
            // such through the term, which is a different claim.
            Self::AlgorithmicMedia | Self::DigitalCapture | Self::HumanEdits => false,
        }
    }
}

impl fmt::Display for DigitalSourceType {
    /// Writes the URI — the form both emitters and every validator use.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.uri())
    }
}

/// A value that is not a term of the IPTC digital source type vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "{value:?} is not an IPTC digital source type this build knows \
     (expected one of trainedAlgorithmicMedia, \
     compositeWithTrainedAlgorithmicMedia, algorithmicMedia, \
     digitalCapture, humanEdits — as a bare term or under \
     http://cv.iptc.org/newscodes/digitalsourcetype/)"
)]
pub struct UnknownSourceType {
    /// The value as it arrived, trimmed.
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[DigitalSourceType] = &[
        DigitalSourceType::TrainedAlgorithmicMedia,
        DigitalSourceType::CompositeWithTrainedAlgorithmicMedia,
        DigitalSourceType::AlgorithmicMedia,
        DigitalSourceType::DigitalCapture,
        DigitalSourceType::HumanEdits,
    ];

    #[test]
    fn every_term_sits_under_the_iptc_vocabulary() {
        // The URI is the whole value: a term published under a different
        // authority is not this vocabulary's term, and a validator reads
        // the string rather than the intent.
        for ty in ALL {
            assert!(
                ty.uri().starts_with(IPTC_CV),
                "{ty:?} points outside the IPTC vocabulary: {}",
                ty.uri()
            );
            assert!(!ty.term().contains('/'), "{ty:?} term is a bare segment");
        }
    }

    #[test]
    fn both_spellings_round_trip() {
        // The URI is what a file carries; the term is what a person
        // types. Both are inputs this type receives, so both have to
        // come back as the same value.
        for ty in ALL {
            assert_eq!(DigitalSourceType::parse(ty.uri()).unwrap(), *ty);
            assert_eq!(DigitalSourceType::parse(ty.term()).unwrap(), *ty);
        }
    }

    #[test]
    fn a_copy_pasted_value_survives_its_whitespace() {
        // These travel through settings fields and shell arguments, the
        // same trip `provenance::parse` trims for.
        assert_eq!(
            DigitalSourceType::parse("  trainedAlgorithmicMedia\n").unwrap(),
            DigitalSourceType::TrainedAlgorithmicMedia
        );
    }

    #[test]
    fn a_term_under_another_authority_is_refused_not_matched_on_its_tail() {
        // The failure this guards is a plausible one: C2PA publishes its
        // own source types under `c2pa.org`, and two of them share a
        // last segment shape with IPTC's. Matching on the tail would
        // silently relabel one vocabulary's term as the other's.
        let err = DigitalSourceType::parse(
            "http://example.invalid/digitalsourcetype/trainedAlgorithmicMedia",
        )
        .unwrap_err();
        assert!(err.to_string().contains("not an IPTC digital source type"));
    }

    #[test]
    fn an_unknown_term_is_refused_rather_than_defaulted() {
        // Defaulting would turn a typo into a disclosure claim, and the
        // wrong direction of that error is a false statement about a
        // file rather than a missing one.
        assert!(DigitalSourceType::parse("negativeFilm").is_err());
        assert!(DigitalSourceType::parse("trained-algorithmic-media").is_err());
        assert!(DigitalSourceType::parse("").is_err());
    }

    #[test]
    fn synthetic_is_the_two_terms_a_model_stands_behind() {
        assert!(DigitalSourceType::TrainedAlgorithmicMedia.is_synthetic());
        assert!(DigitalSourceType::CompositeWithTrainedAlgorithmicMedia.is_synthetic());
        assert!(!DigitalSourceType::AlgorithmicMedia.is_synthetic());
        assert!(!DigitalSourceType::DigitalCapture.is_synthetic());
        assert!(!DigitalSourceType::HumanEdits.is_synthetic());
    }

    #[test]
    fn display_is_the_uri_so_a_format_string_cannot_emit_the_short_form() {
        // Both emitters interpolate this value. If `Display` were the
        // term, an `Iptc4xmpExt:DigitalSourceType` typed as a URI would
        // receive a bare word and validate as malformed.
        assert_eq!(
            DigitalSourceType::TrainedAlgorithmicMedia.to_string(),
            "http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia"
        );
    }
}
