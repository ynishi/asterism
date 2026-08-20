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
//! Two terms have no automatic producer and are not reachable from here
//! at all.
//! [`HumanEdits`](DigitalSourceType::HumanEdits)
//! would have to be inferred from the *absence* of a machine, which
//! manufactures exactly the evidence a copyright claim needs it to
//! record.
//! [`AlgorithmicMedia`](DigitalSourceType::AlgorithmicMedia)
//! needs a producer that says so about itself, and none in this corpus
//! does. Both remain assertable by hand.

pub mod outcome;
pub mod record;
pub mod source_type;

pub use outcome::{DISCLOSURE_NOTE_SCHEMA, Half, Skipped, Stamped};
pub use record::DisclosureRecord;
pub use source_type::DigitalSourceType;

use std::collections::BTreeMap;

/// Keys that only a generator writes, one per family.
///
/// Matched by presence rather than by value: what identifies a ComfyUI
/// export is *that it carries a `workflow`*, not what the workflow says.
/// A container that happens to carry a keyword of the same name from
/// somewhere else is a false positive this accepts — it costs a
/// `trainedAlgorithmicMedia` on a file that is not one, which is the
/// direction that would matter, so each entry is a keyword no general
/// tool writes rather than a plausible-sounding word.
mod generator_keys {
    /// ComfyUI writes both of these: the API-format graph it executed
    /// and the editor graph it was authored in.
    pub const COMFY: &[&str] = &["prompt", "workflow"];
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
/// from; `parents` are its recorded `derived_from` edges, in the order
/// they were read; `prompts` decides whether the prompt is disclosed at
/// all ([`PromptDisclosure`]).
pub fn record_for(
    asset_id: &str,
    title: Option<&str>,
    dispatch_id: Option<&str>,
    meta_kv: Option<&str>,
    parents: &[ParentEvidence],
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

    if evidence.generated {
        // A parent *declared* non-synthetic is the whole difference
        // between the two terms — it is what makes this a model
        // altering material that did not come from one. Declared, not
        // merely undeclared: composite asserts that outside material is
        // in the file, and a parent nobody knows anything about cannot
        // put that assertion into a signed claim. When every parent is
        // synthetic or unknown, the term stays at what the child's own
        // container states.
        record = record.with_source_type(
            if parents
                .iter()
                .any(|p| p.origin == ParentOrigin::NotSynthetic)
            {
                DigitalSourceType::CompositeWithTrainedAlgorithmicMedia
            } else {
                DigitalSourceType::TrainedAlgorithmicMedia
            },
        );
        if let Some(system) = evidence.system {
            // No version: neither family states one under a keyword of
            // its own, and digging one out of a free-text blob would be
            // a guess written into a signed claim.
            record = record.with_ai_system(system, None);
        }
        if let (PromptDisclosure::Embed, Some(prompt)) = (prompts, evidence.prompt) {
            record = record.with_prompt(prompt);
        }
    } else if evidence.captured {
        record = record.with_source_type(DigitalSourceType::DigitalCapture);
    }

    record
}

#[cfg(test)]
mod tests {
    use super::*;

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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[parent("parent-1", ParentOrigin::NotSynthetic)],
            PromptDisclosure::Embed,
        );
        assert_eq!(
            record.source_type,
            Some(DigitalSourceType::CompositeWithTrainedAlgorithmicMedia)
        );
        assert_eq!(record.parents, vec!["parent-1".to_string()]);
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
            &[parent("parent-1", ParentOrigin::Unknown)],
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
            &[
                parent("parent-1", ParentOrigin::Unknown),
                parent("parent-2", ParentOrigin::NotSynthetic),
            ],
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
            &[
                parent("parent-1", ParentOrigin::Synthetic),
                parent("parent-2", ParentOrigin::Synthetic),
            ],
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
            &[],
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
            &[parent("parent-1", ParentOrigin::NotSynthetic)],
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
            let record = record_for("asset-1", None, None, meta_kv, &[], PromptDisclosure::Embed);
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
            PromptDisclosure::Withhold,
        );
        assert_eq!(withheld.prompt, None);
    }
}
