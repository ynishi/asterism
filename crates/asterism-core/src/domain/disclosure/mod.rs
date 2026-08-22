//! What an artefact discloses about how it was made, and the rule that
//! decides it.
//!
//! Three things live here, and they are one concept: the vocabulary
//! ([`DigitalSourceType`], [`DisclosureRecord`], [`Stamped`]), the rule
//! that turns stored metadata into a statement ([`record_for`]), and the
//! policy the rule takes ([`PromptDisclosure`]). Rendering a record into
//! an XMP packet or a C2PA manifest definition is not here — that is
//! `asterism-disclosure-format`, and putting a packet's bytes beside the
//! term it carries would be the container formats leaking into the
//! vocabulary.
//!
//! # Why the vocabulary is in the core and not in a leaf crate
//!
//! It was in one, beside the renderers, and that was wrong in a way that
//! only showed as the feature grew. The renderers need `pngmeta` and a
//! CRC — a chunk walker — so the core's dependency graph acquired the
//! container parser this crate's own manifest records evicting. And the
//! read-back side ("what does this file currently say, and does it still
//! match its bytes") has to be modelled in the core, because a port
//! cannot return a type the core cannot name; with the write vocabulary
//! in a leaf crate, one concept would have been split across two places
//! the moment the second half arrived.
//!
//! The split that holds is by *reason to change*: what may be asserted
//! changes with IPTC, and how a packet is written into a PNG changes
//! with the container. The first is domain and is here; the second is an
//! adapter concern and is not.
//!
//! # The database is the source of truth, not the file
//!
//! Everything here is derived from stored values —
//! [`Material::meta_kv`](crate::domain::material::Material::meta_kv) and
//! the [`DerivedFrom`](crate::domain::edge::EdgeKind::DerivedFrom)
//! edges — and never from the exported file's own metadata. That is what
//! makes a manifest re-appliable after a downstream conversion strips
//! it: the answer was never in the file to begin with.
//!
//! # Evidence, not inference
//!
//! Each term is asserted only on evidence that something wrote:
//!
//! | evidence in the container | term |
//! |---|---|
//! | a generator's own keys, and no recorded parent declares a non-model origin | `trainedAlgorithmicMedia` |
//! | a generator's own keys, and some recorded parent **declares** a non-model origin | `compositeWithTrainedAlgorithmicMedia` |
//! | no generator keys, but EXIF names a camera | `digitalCapture` |
//! | none of the above | nothing is written |
//!
//! Declares, in the second row: a parent whose container says nothing
//! is unknown ([`ParentOrigin::Unknown`]), and unknown moves nothing —
//! it is a statement about what the caller knows, not about the file.
//!
//! The last row is the important one. An artefact nothing established
//! gets no `DigitalSourceType` property, which is a different statement
//! from every term in the vocabulary — the same reading
//! [`attribution`](crate::domain::attribution) gives an absent
//! author. A missing mark on a synthetic file is a gap; a wrong mark on
//! one is a false statement, and only the second is unrecoverable.
//!
//! Two terms have no automatic producer and are never inferred.
//! [`HumanEdits`](DigitalSourceType::HumanEdits)
//! would have to be inferred from the *absence* of a machine, which
//! manufactures exactly the evidence a copyright claim needs it to
//! record.
//! [`AlgorithmicMedia`](DigitalSourceType::AlgorithmicMedia)
//! needs a producer that says so about itself, and none in this corpus
//! does. Both are assertable by hand, and only by hand.
//!
//! # Asserted, and then signed verbatim
//!
//! The hand-assertion route ([`SOURCE_TYPE_KEY`],
//! [`asserted_source_type`]) is the person's own voice in the table
//! above, and it outranks every row of it: the certificate the
//! manifest is signed under is theirs, so their explicit statement is
//! the claim. The ordinary use is the artefact nothing established —
//! the scanned film, the file whose metadata a pipeline stripped —
//! where the assertion is the only voice there is. A parent carrying
//! one reads as *declared* ([`ParentOrigin::declared`]), never as
//! unknown.

pub mod outcome;
pub mod record;
pub mod source_type;

pub use outcome::{DISCLOSURE_NOTE_SCHEMA, Half, Skipped, Stamped};
pub use record::DisclosureRecord;
pub use source_type::DigitalSourceType;

use std::collections::BTreeMap;

use crate::domain::generator_params::GeneratorParams;

/// Keys that only a generator writes, one per family.
///
/// Matched by presence rather than by value: what identifies a ComfyUI
/// export is *that it carries a `workflow`*, not what the workflow says.
/// A container that happens to carry a keyword of the same name from
/// somewhere else is a false positive this accepts — it costs a
/// `trainedAlgorithmicMedia` on a file that is not one, which is the
/// direction that would matter, so each entry is a keyword no general
/// tool writes rather than a plausible-sounding word.
///
/// Public because family membership is one fact with two readers: the
/// evidence rule here, and the parameter extraction registry
/// (`asterism-infra`), which routes a row to a family's judgement by
/// the same keys. A second list on that side would be a way for "what
/// counts as ComfyUI" to disagree with itself.
pub mod generator_keys {
    /// ComfyUI writes both of these: the API-format graph it executed
    /// and the editor graph it was authored in.
    pub const COMFY: &[&str] = &[COMFY_API_GRAPH, "workflow"];
    /// The first of [`COMFY`] under its own name: the API-format graph
    /// the run executed. Presence is all the evidence rule reads; the
    /// extraction registry also reads the *value*, and a bare
    /// `"prompt"` on that side would be the second copy of this fact
    /// the module doc above says not to keep.
    pub const COMFY_API_GRAPH: &str = "prompt";
    /// AUTOMATIC1111 and its forks write one text blob under this
    /// keyword — the prompt, the negative prompt and the sampler
    /// settings in one line-oriented value.
    pub const A1111: &str = "parameters";
}

/// EXIF tags that name the device that made an exposure, as the JPEG
/// probe addresses them (`exif:0x<tag>`; see that module on why the
/// address rather than a name).
mod capture_keys {
    /// `Make`.
    pub const MAKE: &str = "exif:0x010f";
    /// `Model`.
    pub const MODEL: &str = "exif:0x0110";
}

/// The generic `Software` keyword. Not evidence of a generator on its
/// own — an editor writes it too — but it is the best available name
/// for one once a generator key has established that there is one.
const SOFTWARE: &str = "Software";

/// What a parent's own container declares about how it was made.
///
/// Three values, not two. The `bool` this replaces read a parent that
/// declared nothing the same as one that declared a camera, and the
/// difference is the whole question: a declaration is somebody's claim,
/// carried into the manifest on their word, while absence is a question
/// nobody has answered. Converting the second into the first would put
/// words into a signed claim that no evidence said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentOrigin {
    /// The parent's container declares that a generator made it.
    Synthetic,
    /// The parent's container positively declares an origin outside a
    /// model — today, capture metadata naming a camera.
    NotSynthetic,
    /// The parent's container declares nothing either way. A statement
    /// about the caller's knowledge, not about the file.
    Unknown,
}

impl ParentOrigin {
    /// The origin a person's asserted term declares.
    ///
    /// An assertion is a declaration on the same footing as the
    /// container's own — that is the point of taking one — so it maps
    /// onto the same two sides the container evidence does, by the term
    /// vocabulary's own reading of "synthetic". It can never map to
    /// [`Unknown`](Self::Unknown): a person who asserted a term has
    /// answered the question.
    pub fn declared(ty: DigitalSourceType) -> Self {
        if ty.is_synthetic() {
            Self::Synthetic
        } else {
            Self::NotSynthetic
        }
    }
}

/// What a parent contributes to its child's disclosure.
///
/// The caller establishes the origin the same way this module
/// establishes the child's: from the parent's own stored metadata
/// ([`declared_origin`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentEvidence {
    /// The parent's id, as it will appear in the manifest.
    pub asset_id: String,
    /// What the parent's container declares about its origin.
    ///
    /// This is what separates "a model made this" from "a model altered
    /// a photograph": a synthetic child of a synthetic parent is still
    /// `trainedAlgorithmicMedia`, and only a parent *declared*
    /// non-synthetic turns it into a composite. An unknown parent moves
    /// nothing.
    pub origin: ParentOrigin,
}

/// What a container's metadata establishes about how a file was made.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContainerEvidence {
    /// A generator's own keys are present.
    pub generated: bool,
    /// The name of the system that made it, when one is stated.
    pub system: Option<String>,
    /// Human prompt text, when the container carried it as such (see
    /// [`read_evidence`] on why ComfyUI's `prompt` is not this).
    pub prompt: Option<String>,
    /// EXIF names a camera.
    pub captured: bool,
}

/// Reads the canonical metadata object a probe stored.
///
/// The input is [`material_meta`](crate::domain::material_meta)'s
/// canonical form: a JSON object of string → string, values exactly as
/// the container stated them. A value that does not parse as that
/// object yields no evidence rather than an error — an unreadable
/// metadata blob is a file nothing was established about, which is a
/// state this module already has a representation for.
///
/// # Why the prompt is only taken from one of the two families
///
/// `Iptc4xmpExt:AIPromptInformation` is defined as the information given
/// to the service *as prompts*. AUTOMATIC1111's `parameters` is exactly
/// that — the text a person typed, plus the settings, in one blob.
/// ComfyUI's `prompt` is not: it is the API-format node graph the run
/// executed, a JSON document that can reach hundreds of kilobytes and
/// whose relationship to anything a person typed depends on the graph.
/// Writing that into the property would put a workflow dump into every
/// exported file under a field that says it is a prompt, so it is left
/// alone and the graph stays where it already is — in the library.
pub fn read_evidence(meta_kv: &str) -> ContainerEvidence {
    let Ok(fields) = serde_json::from_str::<BTreeMap<String, String>>(meta_kv) else {
        return ContainerEvidence::default();
    };
    let comfy = generator_keys::COMFY
        .iter()
        .any(|key| fields.contains_key(*key));
    let a1111 = fields.contains_key(generator_keys::A1111);

    ContainerEvidence {
        generated: comfy || a1111,
        // The stated name when there is one, and a family name when
        // there is not: a file that carries a `workflow` and no
        // `Software` still came out of something, and "unknown" would
        // be less true than the family the keywords belong to.
        system: match fields.get(SOFTWARE) {
            Some(software) if !software.trim().is_empty() => Some(software.clone()),
            _ if comfy => Some("ComfyUI".to_string()),
            _ => None,
        },
        prompt: fields.get(generator_keys::A1111).cloned(),
        captured: fields.contains_key(capture_keys::MAKE)
            || fields.contains_key(capture_keys::MODEL),
    }
}

/// What a container's metadata declares about the file's origin.
///
/// The parent-side reading of [`read_evidence`], exposed on its own
/// because it is what a caller asks about a *parent* — establishing
/// [`ParentEvidence::origin`] without materialising the rest. A blob
/// whose keys declare a generator is [`Synthetic`](ParentOrigin);
/// one that names a camera and no generator is
/// [`NotSynthetic`](ParentOrigin); no metadata, an unreadable blob, or
/// keys that say neither is [`Unknown`](ParentOrigin) — and stays so,
/// because absence is not a declaration.
pub fn declared_origin(meta_kv: Option<&str>) -> ParentOrigin {
    let Some(meta_kv) = meta_kv else {
        return ParentOrigin::Unknown;
    };
    let evidence = read_evidence(meta_kv);
    if evidence.generated {
        ParentOrigin::Synthetic
    } else if evidence.captured {
        ParentOrigin::NotSynthetic
    } else {
        ParentOrigin::Unknown
    }
}

/// Key under which what became of an artefact's disclosure is recorded,
/// inside `extra.`[`_trace`](crate::domain::provenance::TRACE_KEY).
///
/// Here rather than at the writing call site for the reason
/// `DECLARED_HASH_NOTE_KEY` is beside the digest it describes: the key
/// belongs to the module that owns the concept, so that the writer and
/// anything asking "which artefacts carry a mark" cannot drift apart on
/// the spelling.
///
/// The value is [`Stamped::to_note`]
/// with the moment added by whoever recorded it.
pub const DISCLOSURE_NOTE_KEY: &str = "disclosure";

/// Key under which a person's source-type assertion is recorded, inside
/// `extra.`[`_trace`](crate::domain::provenance::TRACE_KEY).
///
/// This is the hand-assertion route the module docs promise: for an
/// artefact whose container declares nothing, the signer states what it
/// is, and the statement is signed verbatim — their claim, under their
/// certificate. Beside the other keys here for the reason
/// [`DISCLOSURE_NOTE_KEY`] is: the module that owns the concept owns
/// the spelling.
///
/// The value is an [`album_meta::entry`]-shaped statement — `value`
/// (the term's URI; the verb accepts the short name too but stores the
/// spelling every emitter writes), `source`, `operator`,
/// `declared_at_ms` — written by the declare verb and read back by
/// [`asserted_source_type`].
///
/// [`album_meta::entry`]: crate::domain::album_meta::entry
pub const SOURCE_TYPE_KEY: &str = "source_type";

/// The source type a person asserted on this asset, if any.
///
/// Reads the statement [`SOURCE_TYPE_KEY`] files. A value that does not
/// parse as a term reads as no assertion rather than an error: the
/// declare verb refuses unknown terms at the door, so an unreadable
/// stored value is damage, and damage must not fabricate a claim.
pub fn asserted_source_type(extra: &serde_json::Value) -> Option<DigitalSourceType> {
    extra
        .get(crate::domain::provenance::TRACE_KEY)?
        .get(SOURCE_TYPE_KEY)?
        .get("value")?
        .as_str()
        .and_then(|value| DigitalSourceType::parse(value).ok())
}

/// Whether an artefact's prompt is disclosed in the exported file.
///
/// # Why this is a parameter and not a constant
///
/// The prompt is the one field here whose disclosure is a *choice*. The
/// source type states what the file is, and there is one true answer;
/// the AI system names the tool, and it is already in the container the
/// user is exporting. The prompt is different: it is free text that a
/// person wrote, and in the one family that supplies it —
/// AUTOMATIC1111's `parameters` — the blob is not only the prompt. It
/// carries the negative prompt, the sampler settings, the seed, and the
/// names and hashes of the checkpoint and every LoRA. A model trained
/// locally and named after a person or a client puts that name into
/// every copy that leaves the machine.
///
/// [`DisclosureRecord::with_prompt`] already says this is the caller's
/// call — "a decision the service makes, not a property of the data …
/// it cannot be taken back out of a file already published". Until this
/// existed there was nowhere to make it: the mapping filled the field
/// whenever the evidence had one, so the policy was a constant nobody
/// had chosen.
///
/// # There is no default
///
/// Deliberately: an enum with a `Default` is a policy chosen by whoever
/// wrote the `derive`. The caller states it, and the composition root is
/// where an installation's answer belongs.
///
/// # What that answer should be, and why
///
/// [`Withhold`](Self::Withhold), with [`Embed`](Self::Embed) as an
/// explicit choice per installation. Four independent reasons, none of
/// them this repository's opinion:
///
/// 1. **The property is not defined to hold it.** IPTC defines
///    `AIPromptInformation` as "the information that was given to the
///    generative AI service as 'prompt(s)'", and elaborates only that
///    it may include positive and negative statements. A sampler, a
///    seed and a LoRA hash were not given as prompts. The model has its
///    own property (`AISystemUsed`), and nothing in 2025.1 has a place
///    for the rest — so there is no reading under which the standard
///    expects them here.
/// 2. **C2PA names this failure mode.** Harms Modelling §6.1 lists
///    "inadvertent disclosure of information" by claim generators that
///    "automatically add … information that may be sensitive", and the
///    UX Recommendations require that recording personally identifiable
///    information be opt-in. Their one carve-out — the thing an
///    implementer may record over a creator's preference — is the *fact*
///    of AI origin, not the prompt.
/// 3. **The obligation does not ask for it.** EU AI Act Article 50(2)
///    requires that synthetic output be "marked in a machine-readable
///    format and detectable as artificially generated or manipulated".
///    The source type alone discharges that. Embedding the prompt adds
///    no compliance and creates a data-protection exposure that would
///    not otherwise exist.
/// 4. **It can destroy the disclosure it travels with.** A JPEG XMP
///    packet caps at 65,502 bytes, and the XMP specification's rule for
///    an oversized packet is to move out the largest top-level
///    properties first — which is this one. Some writers fail the write
///    outright instead. A prompt large enough takes the source type
///    with it.
///
/// The asymmetry that decides which way to lean is the one this module
/// already states for terms — a missing mark is a gap, a wrong statement
/// is unrecoverable. Withholding a prompt can be undone by re-applying;
/// publishing one cannot be undone at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDisclosure {
    /// The record carries no prompt, whatever the container held.
    Withhold,
    /// The prompt is disclosed as the container stated it.
    ///
    /// Verbatim: this module does not parse the blob, so it cannot
    /// separate the typed text from the settings beside it. Choosing
    /// this discloses both.
    Embed,
}

/// Builds the record for one artefact.
///
/// `meta_kv` is the canonical metadata of the material the file came
/// from; `params` is what the extraction port read out of that same
/// metadata ([`GeneratorParams`], [`GeneratorParams::not_yet`] where
/// nothing ran); `parents` are its recorded `derived_from` edges, in
/// the order they were read; `asserted` is the person's own source-type
/// statement when one is recorded ([`asserted_source_type`]); `prompts`
/// decides whether the prompt is disclosed at all
/// ([`PromptDisclosure`]).
#[allow(clippy::too_many_arguments)] // Each argument is a different stored source with a different owner; grouping them would invent a name for "everything the service read" without making any call site clearer.
pub fn record_for(
    asset_id: &str,
    title: Option<&str>,
    dispatch_id: Option<&str>,
    meta_kv: Option<&str>,
    params: &GeneratorParams,
    parents: &[ParentEvidence],
    asserted: Option<DigitalSourceType>,
    prompts: PromptDisclosure,
) -> DisclosureRecord {
    let evidence = meta_kv.map(read_evidence).unwrap_or_default();

    let mut record = DisclosureRecord::for_asset(asset_id);
    record = record.with_parents(parents.iter().map(|p| p.asset_id.clone()).collect());
    if let Some(dispatch_id) = dispatch_id {
        record = record.with_dispatch(dispatch_id);
    }
    if let Some(title) = title.filter(|t| !t.trim().is_empty()) {
        record = record.with_title(title);
    }

    let evidence_type = if evidence.generated {
        // A parent *declared* non-synthetic is the whole difference
        // between the two terms — it is what makes this a model
        // altering material that did not come from one. Declared, not
        // merely undeclared: composite asserts that outside material is
        // in the file, and a parent nobody knows anything about cannot
        // put that assertion into a signed claim. When every parent is
        // synthetic or unknown, the term stays at what the child's own
        // container states.
        Some(
            if parents
                .iter()
                .any(|p| p.origin == ParentOrigin::NotSynthetic)
            {
                DigitalSourceType::CompositeWithTrainedAlgorithmicMedia
            } else {
                DigitalSourceType::TrainedAlgorithmicMedia
            },
        )
    } else if evidence.captured {
        Some(DigitalSourceType::DigitalCapture)
    } else {
        None
    };

    // The person's assertion outranks the container's word. The
    // certificate the manifest is signed under is theirs, so their
    // explicit statement is the claim — signed verbatim, whatever the
    // container carries. The ordinary use is the artefact nothing
    // established, where the assertion is the only voice; the override
    // is the person correcting a container that is wrong about their
    // own work, which is theirs to do and theirs to answer for.
    let source_type = asserted.or(evidence_type);
    if let Some(source_type) = source_type {
        record = record.with_source_type(source_type);
    }

    // The generator's own statements ride only under a term that admits
    // a generator. `AISystemUsed` or a prompt beside a term the signer
    // chose precisely to say "no model made this" would put the
    // container's contradiction into their signed claim.
    if evidence.generated && source_type.is_some_and(|ty| ty.is_synthetic()) {
        if let Some(system) = evidence.system {
            // No version: neither family states one under a keyword of
            // its own, and digging one out of a free-text blob would be
            // a guess written into a signed claim.
            record = record.with_ai_system(system, None);
        }
        if let (PromptDisclosure::Embed, Some(prompt)) = (prompts, evidence.prompt) {
            record = record.with_prompt(prompt);
        }
        // The extracted parameters ride under the same switch as the
        // prompt, not beside it. They are read out of the very blob the
        // switch was written to contain — the checkpoint name is the
        // "model trained locally and named after a person or a client"
        // in [`PromptDisclosure`]'s own argument — so a record that
        // withheld the blob and then stated its most identifying field
        // as a structured value would have moved the leak, not closed
        // it. Under `Embed` the blob already discloses both, and the
        // structured copy adds no new statement. Only an extraction
        // carries a value; every refusal and marker attaches nothing.
        if prompts == PromptDisclosure::Embed {
            if let Some(model) = params.model.value() {
                record = record.with_model(model);
            }
            if let Some(seed) = params.seed.value() {
                record = record.with_seed(seed);
            }
        }
    }

    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::generator_params::ParamExtraction;

    fn meta(pairs: &[(&str, &str)]) -> String {
        let fields: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        serde_json::to_string(&fields).unwrap()
    }

    fn parent(id: &str, origin: ParentOrigin) -> ParentEvidence {
        ParentEvidence {
            asset_id: id.to_string(),
            origin,
        }
    }

    #[test]
    fn a_comfy_export_is_trained_algorithmic_media() {
        let meta_kv = meta(&[
            ("Software", "ComfyUI"),
            ("prompt", r#"{"3":{"inputs":{"seed":7}}}"#),
            ("workflow", "{}"),
        ]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(
            record.source_type,
            Some(DigitalSourceType::TrainedAlgorithmicMedia)
        );
        assert_eq!(record.ai_system.as_deref(), Some("ComfyUI"));
    }

    #[test]
    fn a_workflow_with_no_software_keyword_still_names_its_family() {
        // "Something made this and did not say what" is less true than
        // the family the keywords belong to, and the keywords are the
        // evidence either way.
        let meta_kv = meta(&[("workflow", "{}")]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(record.ai_system.as_deref(), Some("ComfyUI"));
    }

    #[test]
    fn an_a1111_export_discloses_the_prompt_it_carried_as_prompt_text() {
        let meta_kv = meta(&[(
            "parameters",
            "1girl, purple eyes\nNegative prompt: blurry\nSteps: 20",
        )]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(
            record.source_type,
            Some(DigitalSourceType::TrainedAlgorithmicMedia)
        );
        assert_eq!(
            record.prompt.as_deref(),
            Some("1girl, purple eyes\nNegative prompt: blurry\nSteps: 20")
        );
    }

    #[test]
    fn a_comfy_graph_is_not_disclosed_as_a_prompt() {
        // The property says it holds what was given as prompts. A node
        // graph is not that, and it can reach hundreds of kilobytes —
        // writing it there would put a workflow dump into every
        // exported file under a field that claims to be a prompt.
        let meta_kv = meta(&[("prompt", r#"{"3":{"inputs":{"text":"1girl"}}}"#)]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(record.prompt, None);
        assert_eq!(
            record.source_type,
            Some(DigitalSourceType::TrainedAlgorithmicMedia),
            "it is still evidence that a generator made the file"
        );
    }

    #[test]
    fn a_generated_child_of_a_photograph_is_a_composite() {
        let meta_kv = meta(&[("workflow", "{}")]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[parent("parent-1", ParentOrigin::NotSynthetic)],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(
            record.source_type,
            Some(DigitalSourceType::CompositeWithTrainedAlgorithmicMedia)
        );
        assert_eq!(record.parents, vec!["parent-1".to_string()]);
    }

    #[test]
    fn an_assertion_speaks_where_nothing_was_established() {
        // The ordinary use of the hand-assertion route: a container
        // that declares nothing, and a person who knows what the file
        // is. Their term is the record's term.
        let record = record_for(
            "asset-1",
            None,
            None,
            None,
            &GeneratorParams::not_yet(),
            &[],
            Some(DigitalSourceType::DigitalCapture),
            PromptDisclosure::Embed,
        );
        assert_eq!(record.source_type, Some(DigitalSourceType::DigitalCapture));
    }

    #[test]
    fn an_assertion_outranks_the_container() {
        // The certificate is the signer's, so their explicit statement
        // is the claim. And the generator's own statements do not ride
        // under a term chosen to say no model made this: an
        // `AISystemUsed` beside `humanEdits` would sign the
        // contradiction.
        let meta_kv = meta(&[(
            "parameters",
            "1girl, purple eyes\nNegative prompt: blurry\nSteps: 20",
        )]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[],
            Some(DigitalSourceType::HumanEdits),
            PromptDisclosure::Embed,
        );
        assert_eq!(record.source_type, Some(DigitalSourceType::HumanEdits));
        assert_eq!(record.ai_system, None);
        assert_eq!(record.prompt, None);
    }

    #[test]
    fn a_synthetic_assertion_keeps_the_generators_own_statements() {
        // Asserting the term the evidence already implies changes
        // nothing else: the system name and the prompt are the
        // container's own statements and a synthetic term admits them.
        let meta_kv = meta(&[("workflow", "{}")]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[],
            Some(DigitalSourceType::TrainedAlgorithmicMedia),
            PromptDisclosure::Embed,
        );
        assert_eq!(
            record.source_type,
            Some(DigitalSourceType::TrainedAlgorithmicMedia)
        );
        assert_eq!(record.ai_system.as_deref(), Some("ComfyUI"));
    }

    #[test]
    fn an_asserted_term_reads_back_and_damage_reads_as_no_assertion() {
        let extra = serde_json::json!({
            "_trace": { "source_type": { "value": "digitalCapture" } }
        });
        assert_eq!(
            asserted_source_type(&extra),
            Some(DigitalSourceType::DigitalCapture)
        );
        // The declare verb refuses unknown terms at the door, so an
        // unreadable stored value is damage — and damage must not
        // fabricate a claim.
        for damaged in [
            serde_json::json!({ "_trace": { "source_type": { "value": "notATerm" } } }),
            serde_json::json!({ "_trace": { "source_type": "digitalCapture" } }),
            serde_json::json!({ "_trace": {} }),
            serde_json::json!({}),
            serde_json::Value::Null,
        ] {
            assert_eq!(asserted_source_type(&damaged), None, "for {damaged}");
        }
    }

    #[test]
    fn a_declared_term_maps_onto_the_two_sides_a_container_declares() {
        assert_eq!(
            ParentOrigin::declared(DigitalSourceType::TrainedAlgorithmicMedia),
            ParentOrigin::Synthetic
        );
        assert_eq!(
            ParentOrigin::declared(DigitalSourceType::CompositeWithTrainedAlgorithmicMedia),
            ParentOrigin::Synthetic
        );
        // The vocabulary's own reading: no trained model stands behind
        // any of the three, so a child over them is a model altering
        // material that did not come from one.
        for ty in [
            DigitalSourceType::AlgorithmicMedia,
            DigitalSourceType::DigitalCapture,
            DigitalSourceType::HumanEdits,
        ] {
            assert_eq!(ParentOrigin::declared(ty), ParentOrigin::NotSynthetic);
        }
    }

    #[test]
    fn a_generated_child_of_an_unknown_parent_stays_trained_algorithmic_media() {
        // Composite asserts that material from outside a model is in
        // the file. A parent that declared nothing is not evidence of
        // that — absence converted into the assertion would be a signed
        // claim no evidence made.
        let meta_kv = meta(&[("workflow", "{}")]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[parent("parent-1", ParentOrigin::Unknown)],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(
            record.source_type,
            Some(DigitalSourceType::TrainedAlgorithmicMedia)
        );
        assert_eq!(record.parents, vec!["parent-1".to_string()]);
    }

    #[test]
    fn declared_origin_reads_a_declaration_and_never_invents_one() {
        // A generator's keys declare a generator; a camera's keys with
        // no generator declare a capture; anything else is unknown —
        // including a generator beside a camera, where the generator's
        // processing is the later word.
        assert_eq!(
            declared_origin(Some(&meta(&[("workflow", "{}")]))),
            ParentOrigin::Synthetic
        );
        assert_eq!(
            declared_origin(Some(&meta(&[("exif:0x010f", "Canon")]))),
            ParentOrigin::NotSynthetic
        );
        assert_eq!(
            declared_origin(Some(&meta(&[("workflow", "{}"), ("exif:0x010f", "Canon")]))),
            ParentOrigin::Synthetic
        );
        assert_eq!(declared_origin(Some(&meta(&[]))), ParentOrigin::Unknown);
        assert_eq!(declared_origin(Some("not json")), ParentOrigin::Unknown);
        assert_eq!(declared_origin(None), ParentOrigin::Unknown);
    }

    #[test]
    fn one_declared_non_synthetic_parent_composites_beside_unknowns() {
        // Unknown neighbours do not dilute a declaration: one parent
        // that declares outside material is enough, however many others
        // said nothing.
        let meta_kv = meta(&[("workflow", "{}")]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[
                parent("parent-1", ParentOrigin::Unknown),
                parent("parent-2", ParentOrigin::NotSynthetic),
            ],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(
            record.source_type,
            Some(DigitalSourceType::CompositeWithTrainedAlgorithmicMedia)
        );
    }

    #[test]
    fn a_generated_child_of_generated_parents_stays_trained_algorithmic_media() {
        // img2img over an earlier generation is not a composite: no
        // material that came from outside a model is in it.
        let meta_kv = meta(&[("workflow", "{}")]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[
                parent("parent-1", ParentOrigin::Synthetic),
                parent("parent-2", ParentOrigin::Synthetic),
            ],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(
            record.source_type,
            Some(DigitalSourceType::TrainedAlgorithmicMedia)
        );
        assert_eq!(record.parents.len(), 2);
    }

    #[test]
    fn a_camera_file_is_a_capture_when_no_generator_left_a_trace() {
        let meta_kv = meta(&[
            (capture_keys::MAKE, "FUJIFILM"),
            (capture_keys::MODEL, "X-T5"),
        ]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(record.source_type, Some(DigitalSourceType::DigitalCapture));
        assert_eq!(record.ai_system, None);
    }

    #[test]
    fn a_generator_outranks_retained_camera_tags() {
        // A photograph run through img2img keeps its EXIF. Reading that
        // as a capture would label a synthetic file as one taken with a
        // camera, which is the error direction that cannot be walked
        // back.
        let meta_kv = meta(&[(capture_keys::MAKE, "FUJIFILM"), ("workflow", "{}")]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[parent("parent-1", ParentOrigin::NotSynthetic)],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(
            record.source_type,
            Some(DigitalSourceType::CompositeWithTrainedAlgorithmicMedia)
        );
    }

    #[test]
    fn nothing_established_writes_no_term() {
        // Not a term meaning "unknown": no property at all.
        for meta_kv in [
            None,
            Some("{}"),
            Some("not json"),
            Some(r#"{"Software":"GIMP"}"#),
        ] {
            let record = record_for(
                "asset-1",
                None,
                None,
                meta_kv,
                &GeneratorParams::not_yet(),
                &[],
                None,
                PromptDisclosure::Embed,
            );
            assert_eq!(record.source_type, None, "for {meta_kv:?}");
            assert!(!record.discloses_anything(), "for {meta_kv:?}");
        }
    }

    #[test]
    fn the_parent_origin_is_read_the_same_way_the_child_is() {
        // `declared_origin` is what a caller uses to build
        // `ParentEvidence`, so it has to agree with the evidence reader
        // the child goes through — otherwise a parent could be
        // synthetic as a parent and not as an asset. And an editor's
        // `Software` keyword is not a declaration either way: GIMP on
        // its own leaves the origin unknown, the same reading the child
        // gets.
        let generated = meta(&[("workflow", "{}")]);
        assert_eq!(declared_origin(Some(&generated)), ParentOrigin::Synthetic);
        assert_eq!(
            declared_origin(Some(&meta(&[("Software", "GIMP")]))),
            ParentOrigin::Unknown
        );
    }

    #[test]
    fn the_identifiers_travel_even_when_nothing_else_does() {
        // The manifest half is worth writing for a file nothing was
        // established about: the ids are what make it findable again.
        let record = record_for(
            "asset-1",
            Some("  "),
            Some("dispatch-1"),
            None,
            &GeneratorParams::not_yet(),
            &[],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(record.asset_id, "asset-1");
        assert_eq!(record.dispatch_id.as_deref(), Some("dispatch-1"));
        assert_eq!(record.title, None, "a blank title is not a title");
    }

    #[test]
    fn withholding_the_prompt_leaves_the_rest_of_the_disclosure_intact() {
        // The whole point of the switch: it decides one field. A file
        // whose prompt is withheld still says it is synthetic and still
        // names the system, because those are not the contested part.
        let meta_kv = meta(&[
            (
                generator_keys::A1111,
                "a prompt\nNegative prompt: none\nModel: secret-v3",
            ),
            (SOFTWARE, "AUTOMATIC1111"),
        ]);

        let withheld = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[],
            None,
            PromptDisclosure::Withhold,
        );
        assert_eq!(withheld.prompt, None);
        assert_eq!(
            withheld.source_type,
            Some(DigitalSourceType::TrainedAlgorithmicMedia)
        );
        assert_eq!(withheld.ai_system.as_deref(), Some("AUTOMATIC1111"));
        assert!(
            withheld.discloses_anything(),
            "withholding the prompt is not withholding the disclosure"
        );

        let embedded = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[],
            None,
            PromptDisclosure::Embed,
        );
        assert!(embedded.prompt.is_some());
    }

    #[test]
    fn withholding_is_the_only_thing_that_keeps_the_settings_out() {
        // The blob is not separable here — this module does not parse
        // it — so `Embed` publishes the model name and the LoRA hashes
        // along with the text somebody typed. Pinned so that a later
        // change which starts parsing has to come past this test.
        let meta_kv = meta(&[(
            generator_keys::A1111,
            "cat\nNegative prompt: dog\nSteps: 28, Model: client-name-v3, \
             Lora hashes: \"client-name: a1b2c3d4\"",
        )]);

        let embedded = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[],
            None,
            PromptDisclosure::Embed,
        );
        let prompt = embedded.prompt.expect("embedded");
        assert!(prompt.contains("client-name-v3"));
        assert!(prompt.contains("Lora hashes"));

        let withheld = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &GeneratorParams::not_yet(),
            &[],
            None,
            PromptDisclosure::Withhold,
        );
        assert_eq!(withheld.prompt, None);
    }

    fn extracted() -> GeneratorParams {
        GeneratorParams {
            model: ParamExtraction::Extracted("client-name-v3".into()),
            seed: ParamExtraction::Extracted("620206974400".into()),
        }
    }

    #[test]
    fn extracted_params_ride_under_the_prompts_own_switch() {
        // The checkpoint name is read out of the very blob the
        // withholding switch was written to contain. A record that
        // withheld the blob and then stated the name as a structured
        // field would have moved the leak, not closed it.
        let meta_kv = meta(&[("workflow", "{}")]);

        let embedded = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &extracted(),
            &[],
            None,
            PromptDisclosure::Embed,
        );
        assert_eq!(embedded.model.as_deref(), Some("client-name-v3"));
        assert_eq!(embedded.seed.as_deref(), Some("620206974400"));

        let withheld = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &extracted(),
            &[],
            None,
            PromptDisclosure::Withhold,
        );
        assert_eq!(withheld.model, None);
        assert_eq!(withheld.seed, None);
    }

    #[test]
    fn extracted_params_do_not_ride_beside_a_term_that_denies_a_generator() {
        // The same rule the system name follows: a seed beside a term
        // the signer chose precisely to say "no model made this" would
        // put the container's contradiction into their signed claim.
        let meta_kv = meta(&[("workflow", "{}")]);
        let record = record_for(
            "asset-1",
            None,
            None,
            Some(&meta_kv),
            &extracted(),
            &[],
            Some(DigitalSourceType::HumanEdits),
            PromptDisclosure::Embed,
        );
        assert_eq!(record.model, None);
        assert_eq!(record.seed, None);
    }

    #[test]
    fn only_an_extraction_attaches_a_value() {
        // Every refusal and every marker is a statement about why there
        // is no value, and none of them is a value. Writing one into
        // the record would be the guess the vocabulary exists to
        // refuse.
        let meta_kv = meta(&[("workflow", "{}")]);
        for state in [
            ParamExtraction::NotApplicable,
            ParamExtraction::Absent,
            ParamExtraction::Indirect,
            ParamExtraction::Ambiguous,
            ParamExtraction::NotYet,
        ] {
            let params = GeneratorParams {
                model: state.clone(),
                seed: state.clone(),
            };
            let record = record_for(
                "asset-1",
                None,
                None,
                Some(&meta_kv),
                &params,
                &[],
                None,
                PromptDisclosure::Embed,
            );
            assert_eq!(record.model, None, "for {state:?}");
            assert_eq!(record.seed, None, "for {state:?}");
        }
    }
}
