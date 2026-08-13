//! Turning what the library stored into what a file will disclose.
//!
//! [`asterism_provenance`] owns the vocabulary and both renderings of
//! it, and is deliberately unable to decide anything. This module is
//! where the deciding happens: given the metadata a container carried
//! and the edges the library recorded, which IPTC term is true of this
//! artefact, and which of the 2025.1 AI properties can be filled in.
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
//! | a generator's own keys, and every recorded parent is itself synthetic (or there are none) | `trainedAlgorithmicMedia` |
//! | a generator's own keys, and some recorded parent is **not** synthetic | `compositeWithTrainedAlgorithmicMedia` |
//! | no generator keys, but EXIF names a camera | `digitalCapture` |
//! | none of the above | nothing is written |
//!
//! The last row is the important one. An artefact nothing established
//! gets no `DigitalSourceType` property, which is a different statement
//! from every term in the vocabulary — the same doctrine
//! [`attribution`](crate::domain::attribution) states for an absent
//! author. A missing mark on a synthetic file is a gap; a wrong mark on
//! one is a false statement, and only the second is unrecoverable.
//!
//! Two terms have no automatic producer and are not reachable from here
//! at all.
//! [`HumanEdits`](asterism_provenance::DigitalSourceType::HumanEdits)
//! would have to be inferred from the *absence* of a machine, which
//! manufactures exactly the evidence a copyright claim needs it to
//! record.
//! [`AlgorithmicMedia`](asterism_provenance::DigitalSourceType::AlgorithmicMedia)
//! needs a producer that says so about itself, and none in this corpus
//! does. Both remain assertable by hand.

use std::collections::BTreeMap;

use asterism_provenance::{DigitalSourceType, DisclosureRecord};

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

/// What a parent contributes to its child's disclosure.
///
/// Only one bit of it matters, and the caller establishes that bit the
/// same way this module establishes it for the child: by looking at the
/// parent's own stored metadata ([`is_synthetic`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentEvidence {
    /// The parent's id, as it will appear in the manifest.
    pub asset_id: String,
    /// Whether a generator produced the parent.
    ///
    /// This is what separates "a model made this" from "a model altered
    /// a photograph": a synthetic child of a synthetic parent is still
    /// `trainedAlgorithmicMedia`, and only a non-synthetic parent turns
    /// it into a composite.
    pub synthetic: bool,
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

/// Whether a container's metadata says a generator made the file.
///
/// The same question [`read_evidence`] answers, exposed on its own
/// because it is what a caller asks about a *parent* — establishing the
/// one bit of [`ParentEvidence`] without materialising the rest.
pub fn is_synthetic(meta_kv: Option<&str>) -> bool {
    meta_kv.is_some_and(|meta_kv| read_evidence(meta_kv).generated)
}

/// Builds the record for one artefact.
///
/// `meta_kv` is the canonical metadata of the material the file came
/// from; `parents` are its recorded `derived_from` edges, in the order
/// they were read.
pub fn record_for(
    asset_id: &str,
    title: Option<&str>,
    dispatch_id: Option<&str>,
    meta_kv: Option<&str>,
    parents: &[ParentEvidence],
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
        // A parent that is not itself synthetic is the whole difference
        // between the two terms — it is what makes this a model
        // altering material that did not come from one.
        record = record.with_source_type(if parents.iter().any(|p| !p.synthetic) {
            DigitalSourceType::CompositeWithTrainedAlgorithmicMedia
        } else {
            DigitalSourceType::TrainedAlgorithmicMedia
        });
        if let Some(system) = evidence.system {
            // No version: neither family states one under a keyword of
            // its own, and digging one out of a free-text blob would be
            // a guess written into a signed claim.
            record = record.with_ai_system(system, None);
        }
        if let Some(prompt) = evidence.prompt {
            record = record.with_prompt(prompt, None);
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

    fn parent(id: &str, synthetic: bool) -> ParentEvidence {
        ParentEvidence {
            asset_id: id.to_string(),
            synthetic,
        }
    }

    #[test]
    fn a_comfy_export_is_trained_algorithmic_media() {
        let meta_kv = meta(&[
            ("Software", "ComfyUI"),
            ("prompt", r#"{"3":{"inputs":{"seed":7}}}"#),
            ("workflow", "{}"),
        ]);
        let record = record_for("asset-1", None, None, Some(&meta_kv), &[]);
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
        let record = record_for("asset-1", None, None, Some(&meta_kv), &[]);
        assert_eq!(record.ai_system.as_deref(), Some("ComfyUI"));
    }

    #[test]
    fn an_a1111_export_discloses_the_prompt_it_carried_as_prompt_text() {
        let meta_kv = meta(&[(
            "parameters",
            "1girl, purple eyes\nNegative prompt: blurry\nSteps: 20",
        )]);
        let record = record_for("asset-1", None, None, Some(&meta_kv), &[]);
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
        let record = record_for("asset-1", None, None, Some(&meta_kv), &[]);
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
            &[parent("parent-1", false)],
        );
        assert_eq!(
            record.source_type,
            Some(DigitalSourceType::CompositeWithTrainedAlgorithmicMedia)
        );
        assert_eq!(record.parents, vec!["parent-1".to_string()]);
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
            &[parent("parent-1", true), parent("parent-2", true)],
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
        let record = record_for("asset-1", None, None, Some(&meta_kv), &[]);
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
            &[parent("parent-1", false)],
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
            let record = record_for("asset-1", None, None, meta_kv, &[]);
            assert_eq!(record.source_type, None, "for {meta_kv:?}");
            assert!(!record.discloses_anything(), "for {meta_kv:?}");
        }
    }

    #[test]
    fn the_parent_bit_is_read_the_same_way_the_child_is() {
        // `is_synthetic` is what a caller uses to build `ParentEvidence`,
        // so it has to agree with the evidence reader the child goes
        // through — otherwise a parent could be synthetic as a parent
        // and not as an asset.
        let generated = meta(&[("workflow", "{}")]);
        assert!(is_synthetic(Some(&generated)));
        assert!(!is_synthetic(Some(&meta(&[("Software", "GIMP")]))));
        assert!(!is_synthetic(None));
    }

    #[test]
    fn the_identifiers_travel_even_when_nothing_else_does() {
        // The manifest half is worth writing for a file nothing was
        // established about: the ids are what make it findable again.
        let record = record_for("asset-1", Some("  "), Some("dispatch-1"), None, &[]);
        assert_eq!(record.asset_id, "asset-1");
        assert_eq!(record.dispatch_id.as_deref(), Some("dispatch-1"));
        assert_eq!(record.title, None, "a blank title is not a title");
    }
}
