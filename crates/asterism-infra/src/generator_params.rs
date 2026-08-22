//! Reading generator parameters out of stored metadata rows — the
//! judgement and the registry behind the
//! [`ParamExtractor`](asterism_core::domain::generator_params::ParamExtractor)
//! port.
//!
//! This is the layer that holds opinions. The core owns the outcome
//! vocabulary and the trait; `asterism-media-probe::a1111` owns the
//! grammar of one family's text; what lives here is every judgement
//! neither of them may make — which input key names a seed, which node
//! input is a checkpoint, what a two-element array means, and which
//! families are recognised at all.
//!
//! # The registry, and how a third family arrives
//!
//! Two families are recognised: ComfyUI, routed by the keys
//! [`generator_keys::COMFY`] writes, and AUTOMATIC1111, routed by
//! [`generator_keys::A1111`] — the same one-fact key list the
//! disclosure evidence rule reads, which is why it is imported rather
//! than restated. A family this registry does not recognise (InvokeAI,
//! NovelAI) reads as
//! [`NotApplicable`](ParamExtraction::NotApplicable) today; adding one
//! is a routing arm in [`params_of`](StoredParamExtractor::params_of),
//! a judgement function beside the two here, and — where the family's
//! text needs a grammar — a tokeniser in the media-probe crate on
//! `a1111`'s terms.
//!
//! # Extractability is decided per file
//!
//! The same input on the same node class holds a literal in one graph
//! and a link to another node's output in the next — any custom seed
//! node, any converted widget. A link is read as
//! [`Indirect`](ParamExtraction::Indirect), never resolved: the walk
//! that would resolve it is the future improvement the state exists to
//! leave findable. And where a graph carries more than one candidate
//! and they disagree, the answer is
//! [`Ambiguous`](ParamExtraction::Ambiguous) rather than a winner —
//! there is no evidence about which sampler produced the file, and a
//! wrong value in a signed claim is the unrecoverable direction. That
//! refusal is also why no "which sampler won" is recorded beside the
//! value: agreement extracts and disagreement refuses, so no choice is
//! ever made.

use std::collections::BTreeSet;

use asterism_core::domain::disclosure::generator_keys;
use asterism_core::domain::generator_params::{GeneratorParams, ParamExtraction, ParamExtractor};
use asterism_media_probe::a1111;
use serde_json::{Map, Value};

/// The ComfyUI input keys that carry a seed.
///
/// Keyed by input name rather than by a node-class allowlist, and the
/// choice is the denylist argument the probe port makes: a class list
/// goes stale the day a custom node pack arrives, and its error reads
/// "absent" about a value that is right there — the direction that
/// loses. A key scan finds the seed wherever the graph put it, and the
/// case where that is *too* wide — two nodes stating different seeds —
/// is exactly what [`Ambiguous`](ParamExtraction::Ambiguous) is for.
/// `noise_seed` is the spelling the advanced samplers and the separate
/// noise nodes use; the stock sampler that carries no seed key at all
/// takes its noise as an object built by one of those nodes, which is
/// where the scan then finds it.
const COMFY_SEED_KEYS: &[&str] = &["seed", "noise_seed"];

/// The ComfyUI input keys that carry a checkpoint name.
const COMFY_MODEL_KEYS: &[&str] = &["ckpt_name"];

/// The A1111 settings keys, as that family spells them.
///
/// `Model` and not `Model hash`: the two are different statements, and
/// the hash's absence from this list means it is never promoted into a
/// name.
const A1111_MODEL_KEY: &str = "Model";
const A1111_SEED_KEY: &str = "Seed";

/// The extractor over stored rows.
///
/// A unit struct because every input it reads arrives as an argument:
/// the judgement is in the code, the text is in the row, and nothing is
/// configured. It re-runs across the whole library without opening a
/// single file — the input is the canonical metadata object the
/// fingerprint job stored, not bytes.
pub struct StoredParamExtractor;

impl ParamExtractor for StoredParamExtractor {
    fn params_of(&self, meta_kv: &str) -> GeneratorParams {
        let Ok(fields) =
            serde_json::from_str::<std::collections::BTreeMap<String, String>>(meta_kv)
        else {
            return GeneratorParams::not_applicable();
        };

        let comfy = generator_keys::COMFY
            .iter()
            .any(|key| fields.contains_key(*key))
            .then(|| {
                comfy_params(
                    fields
                        .get(generator_keys::COMFY_API_GRAPH)
                        .map(String::as_str),
                )
            });
        let a1111 = fields
            .get(generator_keys::A1111)
            .map(|blob| a1111_params(blob));

        match (comfy, a1111) {
            (None, None) => GeneratorParams::not_applicable(),
            (Some(one), None) | (None, Some(one)) => one,
            // A container carrying both families' keys is making two
            // statements about one file; they answer jointly, refusing
            // where they disagree.
            (Some(comfy), Some(a1111)) => GeneratorParams {
                model: merge(comfy.model, a1111.model),
                seed: merge(comfy.seed, a1111.seed),
            },
        }
    }
}

/// The ComfyUI judgement, over the API-format graph.
///
/// `graph` is the value under the `prompt` keyword — the graph the run
/// executed, where inputs hold literals or links. `None` is an export
/// carrying only the editor graph (`workflow`): the values likely
/// exist in it, but reading that second grammar is a walk this pass
/// does not make, which is [`Indirect`](ParamExtraction::Indirect) by
/// that state's own definition — behind a reference this extraction
/// does not follow, and findable again when something does.
fn comfy_params(graph: Option<&str>) -> GeneratorParams {
    let Some(graph) = graph else {
        return GeneratorParams {
            model: ParamExtraction::Indirect,
            seed: ParamExtraction::Indirect,
        };
    };
    // An unreadable graph is not this family's shape after all, and
    // "read, and genuinely not there" would be a false statement about
    // text nothing read.
    let Ok(Value::Object(nodes)) = serde_json::from_str::<Value>(graph) else {
        return GeneratorParams::not_applicable();
    };
    GeneratorParams {
        model: scan(&nodes, COMFY_MODEL_KEYS),
        seed: scan(&nodes, COMFY_SEED_KEYS),
    }
}

/// One key family's reading of every node input in a graph.
fn scan(nodes: &Map<String, Value>, keys: &[&str]) -> ParamExtraction {
    let mut literals = BTreeSet::new();
    let mut links = false;
    let mut unrecognised = false;
    for node in nodes.values() {
        let Some(inputs) = node.get("inputs").and_then(Value::as_object) else {
            continue;
        };
        for key in keys {
            match inputs.get(*key) {
                None => {}
                // A link is a two-element array — the node id whose
                // output feeds this input, and which output. The value
                // is one graph hop away, and this pass does not walk.
                Some(Value::Array(link)) if link.len() == 2 => links = true,
                // Verbatim, per the outcome type's contract: a number's
                // own JSON text, a string as written.
                Some(Value::Number(n)) => {
                    literals.insert(n.to_string());
                }
                Some(Value::String(s)) => {
                    literals.insert(s.clone());
                }
                // A shape this judgement does not know is a candidate
                // it cannot read, and extracting past one would be a
                // guess about what it says.
                Some(_) => unrecognised = true,
            }
        }
    }
    if unrecognised {
        return ParamExtraction::Ambiguous;
    }
    match literals.len() {
        // A link beside a literal is an unresolved candidate: the walk
        // that would read it could agree or disagree, so the literal
        // does not stand unchecked — the same precedence [`merge`]
        // gives the two families, settled once and applied in both.
        1 if links => ParamExtraction::Indirect,
        1 => ParamExtraction::Extracted(literals.into_iter().next().expect("len checked")),
        0 if links => ParamExtraction::Indirect,
        0 => ParamExtraction::Absent,
        // Several nodes state several values and nothing says which
        // one produced the file (module docs).
        _ => ParamExtraction::Ambiguous,
    }
}

/// The A1111 judgement, over the tokenised settings line.
fn a1111_params(blob: &str) -> GeneratorParams {
    let settings = a1111::settings(blob);
    GeneratorParams {
        model: setting_of(&settings, A1111_MODEL_KEY),
        seed: setting_of(&settings, A1111_SEED_KEY),
    }
}

/// One key's reading of a settings line: the value where the line
/// states one, refusal where it states two that differ.
fn setting_of(settings: &[(String, String)], key: &str) -> ParamExtraction {
    let values: BTreeSet<&str> = settings
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
        .collect();
    match values.len() {
        0 => ParamExtraction::Absent,
        1 => ParamExtraction::Extracted(values.into_iter().next().expect("len checked").into()),
        _ => ParamExtraction::Ambiguous,
    }
}

/// Joins the two families' answers for one parameter.
///
/// The precedence is the refusal-first ordering the outcome type
/// argues: disagreement anywhere is ambiguity, an unresolved reference
/// anywhere keeps the row revisitable rather than letting the other
/// family's literal stand unchecked, and only then does a lone
/// extraction win over the states that say nothing.
fn merge(a: ParamExtraction, b: ParamExtraction) -> ParamExtraction {
    use ParamExtraction::*;
    match (a, b) {
        (Extracted(x), Extracted(y)) => {
            if x == y {
                Extracted(x)
            } else {
                Ambiguous
            }
        }
        (Ambiguous, _) | (_, Ambiguous) => Ambiguous,
        (Indirect, _) | (_, Indirect) => Indirect,
        (Extracted(x), _) | (_, Extracted(x)) => Extracted(x),
        (Absent, _) | (_, Absent) => Absent,
        // Both family branches ran to produce the two answers, so
        // neither side can be `NotYet`; what remains is two statements
        // of not-applicable.
        _ => NotApplicable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(pairs: &[(&str, &str)]) -> String {
        let fields: std::collections::BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        serde_json::to_string(&fields).unwrap()
    }

    fn params_of(meta_kv: &str) -> GeneratorParams {
        StoredParamExtractor.params_of(meta_kv)
    }

    /// The ordinary ComfyUI export: a sampler holding a literal seed,
    /// a loader holding a literal checkpoint name.
    #[test]
    fn a_comfy_graph_with_literals_extracts_both_values() {
        let graph = r#"{
            "3": {"class_type": "KSampler",
                  "inputs": {"seed": 620206974400, "steps": 20, "model": ["4", 0]}},
            "4": {"class_type": "CheckpointLoaderSimple",
                  "inputs": {"ckpt_name": "cetus-mix_v4.safetensors"}}
        }"#;
        let params = params_of(&meta(&[("prompt", graph), ("workflow", "{}")]));
        assert_eq!(
            params.model,
            ParamExtraction::Extracted("cetus-mix_v4.safetensors".into())
        );
        assert_eq!(
            params.seed,
            ParamExtraction::Extracted("620206974400".into())
        );
    }

    /// The dominant cause of "sometimes": the input holds a link — a
    /// two-element array naming another node's output — and the value
    /// is one graph hop away. Any custom seed node or converted widget
    /// produces this shape in ordinary use.
    #[test]
    fn a_linked_seed_is_indirect_and_does_not_collapse_into_absent() {
        let graph = r#"{
            "3": {"class_type": "KSampler",
                  "inputs": {"seed": ["7", 0], "steps": 20}},
            "7": {"class_type": "PrimitiveNode", "inputs": {}}
        }"#;
        let params = params_of(&meta(&[("prompt", graph)]));
        assert_eq!(params.seed, ParamExtraction::Indirect);
        assert_ne!(params.seed, ParamExtraction::Absent);
    }

    /// The stock sampler with no seed key takes a noise object instead
    /// — and the node that builds the noise carries the seed, where the
    /// key scan finds it. A class allowlist would have stopped at the
    /// sampler and said absent.
    #[test]
    fn a_sampler_without_a_seed_key_is_answered_by_its_noise_node() {
        let graph = r#"{
            "8": {"class_type": "SamplerCustom",
                  "inputs": {"noise": ["9", 0], "cfg": 8}},
            "9": {"class_type": "RandomNoise",
                  "inputs": {"noise_seed": 42}}
        }"#;
        let params = params_of(&meta(&[("prompt", graph)]));
        assert_eq!(params.seed, ParamExtraction::Extracted("42".into()));
    }

    /// No seed-shaped input anywhere is the genuine absence.
    #[test]
    fn a_graph_with_no_seed_anywhere_is_absent() {
        let graph = r#"{
            "1": {"class_type": "LoadImage", "inputs": {"image": "in.png"}}
        }"#;
        let params = params_of(&meta(&[("prompt", graph)]));
        assert_eq!(params.seed, ParamExtraction::Absent);
        assert_eq!(params.model, ParamExtraction::Absent);
    }

    /// Two samplers, two seeds, no evidence about which produced the
    /// file: refused, not chosen — a missing value is a gap, a wrong
    /// one is a false statement.
    #[test]
    fn disagreeing_candidates_are_ambiguous_and_agreeing_ones_extract() {
        let disagree = r#"{
            "3": {"class_type": "KSampler", "inputs": {"seed": 1}},
            "5": {"class_type": "KSampler", "inputs": {"seed": 2}}
        }"#;
        assert_eq!(
            params_of(&meta(&[("prompt", disagree)])).seed,
            ParamExtraction::Ambiguous
        );

        let agree = r#"{
            "3": {"class_type": "KSampler", "inputs": {"seed": 7}},
            "5": {"class_type": "KSamplerAdvanced", "inputs": {"noise_seed": 7}}
        }"#;
        assert_eq!(
            params_of(&meta(&[("prompt", agree)])).seed,
            ParamExtraction::Extracted("7".into())
        );
    }

    /// A literal beside a link on the same key family is an unresolved
    /// candidate: the walk that would read the link could agree or
    /// disagree, so the literal does not stand unchecked — the same
    /// precedence `merge` gives the two families.
    #[test]
    fn a_literal_beside_a_link_is_indirect_rather_than_unchecked() {
        let graph = r#"{
            "3": {"class_type": "KSampler", "inputs": {"seed": 7}},
            "5": {"class_type": "KSampler", "inputs": {"seed": ["9", 0]}}
        }"#;
        assert_eq!(
            params_of(&meta(&[("prompt", graph)])).seed,
            ParamExtraction::Indirect
        );
    }

    /// An export carrying only the editor graph: the values likely
    /// exist, behind a grammar this pass does not read — revisitable,
    /// not absent.
    #[test]
    fn a_workflow_only_export_is_indirect_on_both_params() {
        let params = params_of(&meta(&[("workflow", r#"{"nodes": []}"#)]));
        assert_eq!(params.model, ParamExtraction::Indirect);
        assert_eq!(params.seed, ParamExtraction::Indirect);
    }

    /// A `prompt` value that is not a JSON object is not this family's
    /// shape after all.
    #[test]
    fn an_unreadable_graph_is_not_applicable_rather_than_absent() {
        for graph in ["not json", "[1, 2]", "42"] {
            assert_eq!(
                params_of(&meta(&[("prompt", graph)])),
                GeneratorParams::not_applicable(),
                "for {graph:?}"
            );
        }
    }

    /// The A1111 settings line names both values under its own keys —
    /// the most reliable of the family's fields.
    #[test]
    fn an_a1111_settings_line_extracts_model_and_seed() {
        let blob = "1girl, purple eyes\nNegative prompt: blurry\n\
                    Steps: 28, Seed: 620206974400, Model hash: abc123, \
                    Model: cetus-mix_v4, Lora hashes: \"client: a1, b: c2\"";
        let params = params_of(&meta(&[("parameters", blob)]));
        assert_eq!(
            params.model,
            ParamExtraction::Extracted("cetus-mix_v4".into())
        );
        assert_eq!(
            params.seed,
            ParamExtraction::Extracted("620206974400".into())
        );
    }

    /// A blob without the keys is read, and the values are genuinely
    /// not there. `Model hash` is not promoted into a model name.
    #[test]
    fn an_a1111_blob_without_the_keys_is_absent() {
        let blob = "a cat\nSteps: 20, Sampler: Euler a, Model hash: abc123";
        let params = params_of(&meta(&[("parameters", blob)]));
        assert_eq!(params.model, ParamExtraction::Absent);
        assert_eq!(params.seed, ParamExtraction::Absent);
    }

    /// A blob from neither family — a camera file, an editor's own
    /// keywords, unreadable metadata — is not what this extraction is
    /// about.
    #[test]
    fn material_no_family_wrote_is_not_applicable() {
        for meta_kv in [
            meta(&[("exif:0x010f", "FUJIFILM")]),
            meta(&[("Software", "GIMP")]),
            meta(&[]),
            "not json".to_string(),
        ] {
            assert_eq!(
                params_of(&meta_kv),
                GeneratorParams::not_applicable(),
                "for {meta_kv:?}"
            );
        }
    }

    /// A container carrying both families' keys answers jointly:
    /// agreement extracts, disagreement refuses.
    #[test]
    fn both_families_present_refuse_where_they_disagree() {
        let graph = r#"{"3": {"class_type": "KSampler", "inputs": {"seed": 7}}}"#;
        let agree = meta(&[("prompt", graph), ("parameters", "cat\nSeed: 7")]);
        assert_eq!(
            params_of(&agree).seed,
            ParamExtraction::Extracted("7".into())
        );

        let disagree = meta(&[("prompt", graph), ("parameters", "cat\nSeed: 8")]);
        assert_eq!(params_of(&disagree).seed, ParamExtraction::Ambiguous);
    }
}
