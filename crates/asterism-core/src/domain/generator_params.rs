//! `generator_params` — what an extraction concluded about the
//! parameters a generator recorded, and the port the conclusion comes
//! through.
//!
//! The values in question — the checkpoint a run loaded, the seed it
//! sampled with — are not stored as fields. They sit inside the
//! free-text metadata values the import path copied verbatim
//! ([`material_meta`](crate::domain::material_meta)'s rule: strings,
//! unparsed), and reading them out is a parser, not a mapping. What is
//! extractable is decided **per file, not per generator family**: the
//! same input on the same node class holds a literal in one graph and a
//! link to another node's output in the next. That is why this is a
//! port with an outcome vocabulary rather than a lookup table.
//!
//! # The three layers, and which one this is
//!
//! The split is the one [`probe`](crate::domain::probe) argues for —
//! "the parser is the part that grows per format, and it is the part
//! that belongs furthest from here":
//!
//! | layer | holds |
//! |---|---|
//! | this module, in the core | the outcome vocabulary and the trait |
//! | `asterism-media-probe` | the pure grammar — the A1111 line tokeniser, a function of a string with no opinion about which keys matter |
//! | `asterism-infra` | the judgement and the registry — which input key names a seed, what a two-element array means, which families are recognised at all |
//!
//! # Not on the artefact probe
//!
//! That port is keyed by container mime and selected before any byte is
//! read, and a generator family is not a mime — one `image/png` may be
//! ComfyUI, A1111, InvokeAI or NovelAI, knowable only after the
//! metadata is read. It already refused a third axis once, for raw
//! metadata, on the same reasoning
//! ([`ArtefactProbe::meta_raw_of`](crate::domain::probe::ArtefactProbe::meta_raw_of)).
//! The input here is the **stored metadata rows** — the canonical
//! object [`read_evidence`](crate::domain::disclosure::read_evidence)
//! already parses — so an extractor re-runs across the whole library
//! without opening a single file.
//!
//! # Workflow identity is not here
//!
//! It is not extractable from either family — the ComfyUI graph mints
//! no run id, and A1111's grammar has no such field — so there is no
//! `workflow` member of [`GeneratorParams`] to be perpetually absent.
//! If a workflow identity ever reaches the manifest it is a value the
//! user supplies, on the hand-assertion footing
//! [`asserted_source_type`](crate::domain::disclosure::asserted_source_type)
//! established, not an extraction.
//!
//! # Extraction does not touch the meta axis
//!
//! The digest and its canonical form say one thing — *the container
//! carried this text* — and extraction changes neither that input nor
//! that definition. An extractor reads the stored values; it never
//! rewrites them, so every digest stands exactly as it did before the
//! extractor existed.

/// What an extraction concluded about one parameter.
///
/// Six states rather than `Option<String>`, for the reason
/// [`MaterialMeta`](crate::domain::material_meta::MaterialMeta) gives
/// three: the ways of having no value lead somewhere different, and
/// collapsing them loses the difference for good. The load-bearing
/// distinction is [`Indirect`](Self::Indirect) against
/// [`Absent`](Self::Absent) — a value behind a graph link is
/// recoverable later by a walk, and a reading that filed it as absent
/// would leave a future improvement no way to find the rows it should
/// revisit. The same argument the raw-metadata column makes for keeping
/// its "not captured" state.
///
/// Ambiguity refuses rather than guesses: a missing value is a gap, a
/// wrong value is a false statement, and only the second is
/// unrecoverable once it reaches a signed claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamExtraction {
    /// The value, exactly as the container stated it — a checkpoint
    /// name as written, a seed as its literal text. Verbatim for the
    /// reason the canonical form keeps values unparsed: re-rendering
    /// would put a serialiser's habits into a statement about what the
    /// container said.
    Extracted(String),
    /// This material is not one the extraction applies to: no
    /// recognised generator family wrote its metadata, or what a family
    /// wrote is not readable as that family's shape.
    NotApplicable,
    /// A recognised family's metadata was read and the value is
    /// genuinely not there.
    Absent,
    /// The value exists but sits behind a reference this extraction
    /// does not follow — an input holding a link to another node's
    /// output rather than a literal, or an editor graph with no API
    /// graph beside it. Recoverable by a later walk; the rows to
    /// revisit when one arrives.
    Indirect,
    /// More than one candidate value and no evidence to choose between
    /// them. Refused rather than guessed (type docs).
    Ambiguous,
    /// No extraction has run over this row. The resting state of a row
    /// the feature has not visited, never the answer of an extractor
    /// that ran.
    NotYet,
}

impl ParamExtraction {
    /// The value, when one was extracted — for callers that must not
    /// act on any other state as though it were a value.
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Extracted(value) => Some(value),
            _ => None,
        }
    }
}

/// One extraction's conclusion about every parameter it answers for.
///
/// One field per parameter rather than a map: the parameters are a
/// fixed vocabulary this module owns, and a map would let a caller ask
/// about a parameter nobody extracts and receive an invented answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratorParams {
    /// The model or checkpoint the run loaded.
    pub model: ParamExtraction,
    /// The seed the run sampled with.
    pub seed: ParamExtraction,
}

impl GeneratorParams {
    /// The resting state of a row no extraction has visited.
    pub fn not_yet() -> Self {
        Self {
            model: ParamExtraction::NotYet,
            seed: ParamExtraction::NotYet,
        }
    }

    /// The conclusion for material the extraction does not apply to —
    /// no metadata at all, or metadata no recognised family wrote.
    pub fn not_applicable() -> Self {
        Self {
            model: ParamExtraction::NotApplicable,
            seed: ParamExtraction::NotApplicable,
        }
    }
}

/// Reads generator parameters out of stored metadata.
///
/// The port the judgement layer implements. Input is the canonical
/// metadata object a probe stored
/// ([`material_meta`](crate::domain::material_meta)'s form), which is
/// what makes an implementation a pure function over rows: no I/O, no
/// file, no registry of containers — those all happened when the row
/// was written.
pub trait ParamExtractor: Send + Sync {
    /// What these stored metadata values say about the parameters.
    ///
    /// A value that does not parse as the canonical object yields
    /// [`GeneratorParams::not_applicable`] rather than an error, the
    /// same reading
    /// [`read_evidence`](crate::domain::disclosure::read_evidence)
    /// gives an unreadable blob: a file nothing was established about.
    fn params_of(&self, meta_kv: &str) -> GeneratorParams;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only an extraction carries a value; every refusal and every
    /// marker reads as none.
    #[test]
    fn only_the_extracted_state_carries_a_value() {
        assert_eq!(
            ParamExtraction::Extracted("cetus-mix".into()).value(),
            Some("cetus-mix")
        );
        for state in [
            ParamExtraction::NotApplicable,
            ParamExtraction::Absent,
            ParamExtraction::Indirect,
            ParamExtraction::Ambiguous,
            ParamExtraction::NotYet,
        ] {
            assert_eq!(state.value(), None, "for {state:?}");
        }
    }

    /// The two constructors are two different statements: one says the
    /// question was never asked, the other says it was asked of
    /// material it does not apply to.
    #[test]
    fn the_resting_state_and_the_refusal_are_different_statements() {
        assert_eq!(GeneratorParams::not_yet().model, ParamExtraction::NotYet);
        assert_eq!(GeneratorParams::not_yet().seed, ParamExtraction::NotYet);
        assert_ne!(
            GeneratorParams::not_yet(),
            GeneratorParams::not_applicable()
        );
    }
}
