//! `DisclosureRecord` — everything one exported file is going to say
//! about where it came from, decided before either emitter runs.
//!
//! The two emitters write into different places for different readers.
//! The XMP packet carries the IPTC properties a platform reads to decide
//! whether a file is synthetic; the C2PA manifest carries a signed claim
//! plus the lineage this library holds. They must not disagree, and the
//! cheapest way to make disagreement impossible is for both to be
//! rendered from one value that was assembled once.
//!
//! # What is here and what is deliberately not
//!
//! This is a *statement*, not a projection of a row. The application
//! service builds it out of what the database holds — material metadata
//! for the generator and the prompt, derivation edges for the parents,
//! attribution for the operator — and every judgement (is this
//! synthetic, is the parent a photograph, may the prompt be disclosed)
//! is made there, on the way in. Nothing downstream of this type
//! re-decides anything: an emitter that had a policy would be a second
//! place the answer could differ from the first.
//!
//! # The two audiences, and why the identifiers only reach one
//!
//! Asterism's own identifiers — the asset id, the dispatch the file left
//! through, the parents it was made from — go into the C2PA manifest's
//! custom assertion and **not** into the XMP packet. Two reasons, and
//! the second is the load-bearing one:
//!
//! 1. IPTC has no property for them. Inventing an `Iptc4xmpExt` field
//!    would put a private vocabulary into a namespace that is not this
//!    repository's to extend.
//! 2. The manifest is where lineage belongs by design, and it is signed.
//!    An id in an unsigned XMP packet is an id anybody can rewrite,
//!    which makes it useless for exactly the job it would be there for
//!    — finding the row again — while still being present in every
//!    published file.
//!
//! The sidecar the file exporter already writes carries the same
//! identifiers in the clear for the receiver that has no C2PA reader
//! (`asterism-contract::sidecar`), so nothing is lost by keeping them
//! out of the packet.
//!
//! # Who wrote the prompt is not disclosed
//!
//! IPTC 2025.1 defines `AIPromptWriterName`, and this record does not
//! carry it. The property names a person, and IPTC is explicit that the
//! person who wrote the prompt is not thereby the image's creator —
//! which is why it has a field of its own rather than riding
//! `dc:creator`. Nothing in this application states who wrote a prompt:
//! the prompt reaching a record is read back out of the container the
//! file arrived in, and a dispatch may run against text written by
//! somebody else, generated, or rewritten across rounds. Filling the
//! property from the asset's author or from the operator would assert
//! something nobody stated, in a file that cannot be taken back once
//! published — the asymmetry [`PromptDisclosure`] already turns on, and
//! a name is a stronger claim than the text is.
//!
//! So the field, its setter argument and the emitter branch are absent
//! rather than present-and-unreachable. If a surface for stating it ever
//! exists, this is where it returns, under the same withholding control
//! the prompt has.
//!
//! [`PromptDisclosure`]: super::PromptDisclosure
//!
//! # A human pass is asserted, never inferred
//!
//! [`DigitalSourceType::HumanEdits`] is the one value a caller has to
//! state. Nothing observable distinguishes "a person worked on this" from
//! "no generator metadata was found", and the copyright question that
//! makes the distinction worth recording turns on the human layer being
//! evidenced rather than assumed. Deriving it from an absence would
//! manufacture exactly the evidence it is supposed to record.

use super::source_type::DigitalSourceType;

/// What one exported artefact will disclose.
///
/// Built through [`for_asset`](Self::for_asset) and the `with_*` chain
/// so that adding a field cannot silently produce a record where a
/// caller's positional argument moved.
///
/// Deliberately no `Default`: it would construct the one state the
/// field docs forbid — a record with an empty `asset_id` — and that
/// record reaches a signed assertion nobody can correct afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisclosureRecord {
    /// How the file came to exist. `None` means nothing established it
    /// — no property is written, which is a different statement from
    /// any term (`source_type` module docs).
    pub source_type: Option<DigitalSourceType>,
    /// `Iptc4xmpExt:AISystemUsed` — the engine and/or model that
    /// generated the file ("ComfyUI", a checkpoint name).
    pub ai_system: Option<String>,
    /// `Iptc4xmpExt:AISystemVersionUsed` — that engine's version, when
    /// the container stated one.
    pub ai_system_version: Option<String>,
    /// `Iptc4xmpExt:AIPromptInformation` — the prompt the generator was
    /// given.
    ///
    /// Disclosing this is a decision the service makes, not a property
    /// of the data: a prompt is written text that travels with every
    /// copy of the file once it is embedded, and it cannot be taken back
    /// out of a file already published. The emitters here write it when
    /// it is present and never populate it themselves.
    pub prompt: Option<String>,
    /// Asterism's id for the asset this file is. Manifest-only (module
    /// docs).
    pub asset_id: String,
    /// The dispatch this file left through, when it left through one.
    /// Manifest-only.
    pub dispatch_id: Option<String>,
    /// Asset ids this file was derived from, in the order the edges were
    /// read. Manifest-only.
    pub parents: Vec<String>,
    /// Human-readable title for the manifest. Absent is fine — C2PA
    /// treats it as a label, and an empty one is better than a
    /// manufactured one.
    pub title: Option<String>,
}

impl DisclosureRecord {
    /// Starts a record for one asset.
    ///
    /// The asset id is the only mandatory field: a record that could not
    /// name what it describes could not be re-applied later, which is
    /// the property the database-is-the-source-of-truth requirement
    /// rests on.
    pub fn for_asset(asset_id: impl Into<String>) -> Self {
        Self {
            source_type: None,
            ai_system: None,
            ai_system_version: None,
            prompt: None,
            asset_id: asset_id.into(),
            dispatch_id: None,
            parents: Vec::new(),
            title: None,
        }
    }

    /// Sets the digital source type.
    pub fn with_source_type(mut self, source_type: DigitalSourceType) -> Self {
        self.source_type = Some(source_type);
        self
    }

    /// Sets the generating system, and its version when one is known.
    pub fn with_ai_system(mut self, system: impl Into<String>, version: Option<String>) -> Self {
        self.ai_system = Some(system.into());
        self.ai_system_version = version;
        self
    }

    /// Sets the prompt.
    ///
    /// See the field docs: reaching for this is a disclosure decision,
    /// and it is the caller's. Who wrote the prompt is not part of it —
    /// the module docs say why there is no argument for it here.
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    /// Names the dispatch this file left through.
    pub fn with_dispatch(mut self, dispatch_id: impl Into<String>) -> Self {
        self.dispatch_id = Some(dispatch_id.into());
        self
    }

    /// Names the assets this one was derived from.
    pub fn with_parents(mut self, parents: Vec<String>) -> Self {
        self.parents = parents;
        self
    }

    /// Sets the manifest title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Whether the XMP packet would carry anything at all.
    ///
    /// The identifiers do not count — they are manifest-only, so a
    /// record holding nothing but an asset id renders an empty packet,
    /// and writing an empty packet into a file is a modification that
    /// buys nothing. The caller checks this rather than the emitter
    /// refusing, because "there was nothing to say" is a normal outcome
    /// and not an error.
    pub fn discloses_anything(&self) -> bool {
        self.source_type.is_some()
            || self.ai_system.is_some()
            || self.ai_system_version.is_some()
            || self.prompt.is_some()
    }

    /// The same record with everything but the obligation dropped.
    ///
    /// JPEG's XMP packet has to fit one APP1 segment, which leaves it
    /// `asterism_disclosure_format::embed::JPEG_MAX_PACKET` bytes —
    /// 65,504, the segment's 65,533-byte payload less the 29-byte XMP
    /// identifier that has to go in front of it. A ComfyUI prompt can be
    /// larger than that on its own. The ExtendedXMP mechanism exists for
    /// the overflow and is not implemented here, so the fallback is to
    /// write less rather than to write a split packet a reader may not
    /// reassemble.
    ///
    /// The constant rather than a number repeated here: this doc is what
    /// a caller reads when deciding how long a prompt to allow, and it
    /// said 65,533 while the writer enforced 65,504 — a packet in
    /// between was refused by a limit the documentation did not have.
    ///
    /// What survives is the digital source type and the system that
    /// produced the file. That ordering is not a guess about what
    /// matters: the source type is the machine-readable mark Article 50
    /// requires, and the prompt is context. Dropping the mark to keep
    /// the context would fail the obligation to preserve a nicety.
    ///
    /// The caller is expected to record that it fell back — a file whose
    /// prompt was dropped and a file that never had one are otherwise
    /// indistinguishable afterwards.
    ///
    /// When even this does not fit, [`obligation`](Self::obligation) is
    /// the tier below.
    pub fn essential(&self) -> Self {
        Self {
            source_type: self.source_type,
            ai_system: self.ai_system.clone(),
            ai_system_version: self.ai_system_version.clone(),
            prompt: None,
            asset_id: self.asset_id.clone(),
            dispatch_id: self.dispatch_id.clone(),
            parents: self.parents.clone(),
            title: self.title.clone(),
        }
    }

    /// The record cut down to the machine-readable mark alone.
    ///
    /// [`essential`](Self::essential) keeps the generating system, and
    /// the system name is read out of someone else's file — unbounded,
    /// so a large enough one overflows the JPEG segment a second time
    /// and takes the whole packet with it. This is the tier below:
    /// nothing in the packet but the digital source type, whose values
    /// are a fixed vocabulary of URIs and always fit. The manifest half
    /// is untouched for the same reason it is untouched in `essential` —
    /// the limit being escaped is JPEG's APP1 segment, which the
    /// manifest does not travel in.
    ///
    /// A record with no source type reduces to one that discloses
    /// nothing, and the fallback ladder ends there: with the mark absent
    /// there is nothing bounded left to write, and the caller keeps the
    /// failure rather than reporting an empty write as a success.
    pub fn obligation(&self) -> Self {
        Self {
            source_type: self.source_type,
            ai_system: None,
            ai_system_version: None,
            prompt: None,
            asset_id: self.asset_id.clone(),
            dispatch_id: self.dispatch_id.clone(),
            parents: self.parents.clone(),
            title: self.title.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_with_only_an_id_discloses_nothing() {
        // The id is manifest-only, so there is no XMP packet to write.
        // The caller has to be able to see that without rendering one.
        let record = DisclosureRecord::for_asset("asset-1");
        assert!(!record.discloses_anything());
        assert_eq!(record.asset_id, "asset-1");
    }

    #[test]
    fn any_iptc_field_on_its_own_is_a_disclosure() {
        // A generator name with no source type is a real state: the
        // container said what made it and nothing established what that
        // makes it. It still belongs in the packet.
        let record = DisclosureRecord::for_asset("asset-1").with_ai_system("ComfyUI", None);
        assert!(record.discloses_anything());
    }

    #[test]
    fn essential_keeps_the_obligation_and_drops_the_context() {
        let record = DisclosureRecord::for_asset("asset-1")
            .with_source_type(DigitalSourceType::TrainedAlgorithmicMedia)
            .with_ai_system("ComfyUI", Some("0.3.0".into()))
            .with_prompt("a very long prompt")
            .with_parents(vec!["parent-1".into()])
            .with_dispatch("dispatch-1");

        let reduced = record.essential();
        assert_eq!(
            reduced.source_type,
            Some(DigitalSourceType::TrainedAlgorithmicMedia),
            "the mark Article 50 requires is what survives"
        );
        assert_eq!(reduced.ai_system.as_deref(), Some("ComfyUI"));
        assert_eq!(reduced.ai_system_version.as_deref(), Some("0.3.0"));
        assert_eq!(reduced.prompt, None);
        // The manifest half is untouched: the size limit being fallen
        // back from is JPEG's APP1 segment, which the manifest does not
        // travel in.
        assert_eq!(reduced.parents, vec!["parent-1".to_string()]);
        assert_eq!(reduced.dispatch_id.as_deref(), Some("dispatch-1"));
    }

    #[test]
    fn essential_is_idempotent_so_a_second_fallback_cannot_erode_further() {
        let record = DisclosureRecord::for_asset("asset-1")
            .with_source_type(DigitalSourceType::TrainedAlgorithmicMedia)
            .with_prompt("p");
        assert_eq!(record.essential(), record.essential().essential());
    }

    #[test]
    fn obligation_keeps_the_mark_and_nothing_else_in_the_packet() {
        let record = DisclosureRecord::for_asset("asset-1")
            .with_source_type(DigitalSourceType::TrainedAlgorithmicMedia)
            .with_ai_system("ComfyUI", Some("0.3.0".into()))
            .with_prompt("a prompt")
            .with_parents(vec!["parent-1".into()])
            .with_dispatch("dispatch-1")
            .with_title("a title");

        let mark = record.obligation();
        assert_eq!(
            mark.source_type,
            Some(DigitalSourceType::TrainedAlgorithmicMedia)
        );
        assert_eq!(mark.ai_system, None, "the unbounded string is gone");
        assert_eq!(mark.ai_system_version, None);
        assert_eq!(mark.prompt, None);
        // The manifest half does not travel in the segment being escaped.
        assert_eq!(mark.asset_id, "asset-1");
        assert_eq!(mark.parents, vec!["parent-1".to_string()]);
        assert_eq!(mark.dispatch_id.as_deref(), Some("dispatch-1"));
        assert_eq!(mark.title.as_deref(), Some("a title"));
    }

    #[test]
    fn obligation_is_the_bottom_of_the_fallback_ladder() {
        let record = DisclosureRecord::for_asset("asset-1")
            .with_source_type(DigitalSourceType::TrainedAlgorithmicMedia)
            .with_ai_system("ComfyUI", None)
            .with_prompt("p");
        // Each tier only removes, and the bottom tier is a fixed point.
        assert_eq!(record.obligation(), record.essential().obligation());
        assert_eq!(record.obligation(), record.obligation().obligation());
        // A record with no mark reduces to one that discloses nothing.
        let unmarked = DisclosureRecord::for_asset("asset-1").with_ai_system("ComfyUI", None);
        assert!(!unmarked.obligation().discloses_anything());
    }
}
