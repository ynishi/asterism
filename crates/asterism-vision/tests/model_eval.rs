//! The model-gated evaluation harness (#112): fixtures × a real
//! package.
//!
//! Runs only when both opt-ins are present — the `fixtures` and `onnx`
//! features at compile time, and `ASTERISM_TEST_MODEL_DIR` (a package
//! directory) at run time. CI has neither, so the suite is a no-op
//! there by construction; the measurements this produces are read by a
//! person and recorded on the issue, which is the P2-5 step.
//!
//! What it asserts is *ordering*, deliberately: a look-alike must beat
//! a hard negative, a matching tag must beat a disjoint one, noise
//! must sit below the scene family. The absolute numbers — recall@k,
//! the score floor — are printed as one JSON line for the record, not
//! asserted, because pinning them is the measurement's job, not the
//! harness's.
#![cfg(all(feature = "fixtures", feature = "onnx"))]

use asterism_vision::encoder::Encoder;
use asterism_vision::fixtures::relations::{RelatedScenes, RelationStream, unrelated_queries_en};
use asterism_vision::fixtures::scene::{self, SceneSpec, noise_image};
use asterism_vision::package::ModelPackage;

const SEED: u64 = 42;
const BASES: usize = 24;

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn encoder_or_skip() -> Option<Encoder> {
    let dir = match std::env::var("ASTERISM_TEST_MODEL_DIR") {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => {
            eprintln!("model eval skipped: ASTERISM_TEST_MODEL_DIR is unset");
            return None;
        }
    };
    let package = ModelPackage::open(&dir).expect("the named package must open");
    Some(Encoder::load(&package).expect("the package must load"))
}

fn encode_scene(encoder: &mut Encoder, spec: &SceneSpec) -> Vec<f32> {
    let img = scene::render(spec).expect("render");
    encoder
        .encode_image(img.as_raw(), img.width(), img.height())
        .expect("encode")
}

/// One pass over the fixture set: every ordering claim the corpus
/// design makes, checked against the real encoder, and the numbers
/// printed for the record.
#[test]
fn orderings_hold_and_measurements_print() {
    let Some(mut encoder) = encoder_or_skip() else {
        return;
    };
    let specs: Vec<RelatedScenes> = RelationStream::new(SEED).take(BASES).collect();

    // Encode everything once.
    let bases: Vec<Vec<f32>> = specs
        .iter()
        .map(|s| encode_scene(&mut encoder, &s.scene))
        .collect();
    let noise: Vec<Vec<f32>> = (0..3)
        .map(|i| {
            let img = noise_image(SEED ^ 0xC0FF_EE00 ^ i, 640, 640);
            encoder
                .encode_image(img.as_raw(), img.width(), img.height())
                .expect("encode noise")
        })
        .collect();

    let mut lookalike_beats_negative = 0usize;
    let mut lookalike_pairs = 0usize;
    let mut sibling_beats_negative = 0usize;
    let mut sibling_pairs = 0usize;
    let mut lookalike_scores = Vec::new();
    let mut sibling_scores = Vec::new();
    let mut negative_scores = Vec::new();
    let mut stranger_scores = Vec::new();
    let mut noise_scores = Vec::new();

    for (i, spec) in specs.iter().enumerate() {
        let base = &bases[i];
        let negative = spec
            .hard_negative
            .as_ref()
            .map(|n| cosine(base, &encode_scene(&mut encoder, n)));
        if let Some(alike) = &spec.lookalike {
            let score = cosine(base, &encode_scene(&mut encoder, alike));
            lookalike_scores.push(score);
            if let Some(neg) = negative {
                lookalike_pairs += 1;
                if score > neg {
                    lookalike_beats_negative += 1;
                }
            }
        }
        if let Some(sibling) = &spec.semantic_sibling {
            let score = cosine(base, &encode_scene(&mut encoder, sibling));
            sibling_scores.push(score);
            if let Some(neg) = negative {
                sibling_pairs += 1;
                if score > neg {
                    sibling_beats_negative += 1;
                }
            }
        }
        if let Some(neg) = negative {
            negative_scores.push(neg);
        }
        // Every other base is a stranger; sample the next one.
        if let Some(other) = bases.get((i + 1) % bases.len())
            && i + 1 != bases.len()
        {
            stranger_scores.push(cosine(base, other));
        }
        for n in &noise {
            noise_scores.push(cosine(base, n));
        }
    }

    // Tag matching: a base's own EN tag must beat the disjoint tags of
    // its hard negative.
    let mut tag_own_beats_disjoint = 0usize;
    let mut tag_pairs = 0usize;
    let mut own_tag_scores = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        let Some(negative) = &spec.hard_negative else {
            continue;
        };
        let own = &spec.scene.tags_en()[0];
        let disjoint = &negative.tags_en()[0];
        let own_score = cosine(&bases[i], &encoder.encode_text(own).expect("tag"));
        let disjoint_score = cosine(&bases[i], &encoder.encode_text(disjoint).expect("tag"));
        own_tag_scores.push(own_score);
        tag_pairs += 1;
        if own_score > disjoint_score {
            tag_own_beats_disjoint += 1;
        }
    }

    // Honest failure: unrelated queries against every base.
    let mut unrelated_query_scores = Vec::new();
    for query in unrelated_queries_en() {
        let q = encoder.encode_text(&query).expect("query");
        for base in &bases {
            unrelated_query_scores.push(cosine(base, &q));
        }
    }

    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let max = |v: &[f32]| v.iter().copied().fold(f32::MIN, f32::max);
    println!(
        "{}",
        serde_json::json!({
            "schema": "vision-model-eval-v1",
            "model_id": encoder.model_id(),
            "seed": SEED,
            "bases": BASES,
            "lookalike": { "mean": mean(&lookalike_scores), "beats_negative": lookalike_beats_negative, "pairs": lookalike_pairs },
            "sibling": { "mean": mean(&sibling_scores), "beats_negative": sibling_beats_negative, "pairs": sibling_pairs },
            "hard_negative_mean": mean(&negative_scores),
            "stranger_mean": mean(&stranger_scores),
            "noise": { "mean": mean(&noise_scores), "max": max(&noise_scores) },
            "tag": { "own_mean": mean(&own_tag_scores), "own_beats_disjoint": tag_own_beats_disjoint, "pairs": tag_pairs },
            "unrelated_query": { "mean": mean(&unrelated_query_scores), "max": max(&unrelated_query_scores) },
        })
    );

    // The ordering claims. Aggregate, not per pair: an encoder is
    // allowed an off day on one scene, not on the population.
    assert!(
        lookalike_beats_negative * 10 >= lookalike_pairs * 8,
        "look-alikes must beat hard negatives on at least 80% of pairs \
         ({lookalike_beats_negative}/{lookalike_pairs})"
    );
    assert!(
        tag_own_beats_disjoint * 10 >= tag_pairs * 8,
        "own tags must beat disjoint tags on at least 80% of pairs \
         ({tag_own_beats_disjoint}/{tag_pairs})"
    );
    assert!(
        mean(&noise_scores) < mean(&lookalike_scores),
        "noise must sit below the scene family"
    );
}

/// The dim the manifest declares is the dim the towers produce — the
/// P0 record left 768 as inferred-from-defaults, and this is where the
/// inference is replaced by an assertion against the real model.
#[test]
fn declared_dim_is_asserted_against_the_model() {
    let Some(mut encoder) = encoder_or_skip() else {
        return;
    };
    let img = scene::noise_image(1, 64, 64);
    let v = encoder
        .encode_image(img.as_raw(), 64, 64)
        .expect("encode succeeds only when the dims agree");
    assert_eq!(v.len(), encoder.dim() as usize);
    let t = encoder.encode_text("a red circle").expect("text tower");
    assert_eq!(t.len(), encoder.dim() as usize);
}
