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
use asterism_vision::fixtures::relations::{
    RelatedScenes, RelationStream, unrelated_queries_en, unrelated_queries_ja,
};
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

/// One language's tag-vs-image measurement.
#[derive(Default)]
struct TagMeasure {
    own_scores: Vec<f32>,
    own_beats_disjoint: usize,
    pairs: usize,
}

impl TagMeasure {
    fn observe(&mut self, encoder: &mut Encoder, base: &[f32], own: &str, disjoint: &str) {
        let own_score = cosine(base, &encoder.encode_text(own).expect("tag"));
        let disjoint_score = cosine(base, &encoder.encode_text(disjoint).expect("tag"));
        self.own_scores.push(own_score);
        self.pairs += 1;
        if own_score > disjoint_score {
            self.own_beats_disjoint += 1;
        }
    }

    fn json(&self) -> serde_json::Value {
        let mean = self.own_scores.iter().sum::<f32>() / self.own_scores.len().max(1) as f32;
        serde_json::json!({
            "own_mean": mean,
            "own_beats_disjoint": self.own_beats_disjoint,
            "pairs": self.pairs,
        })
    }
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

    // Tag matching, both languages: a base's own tag must beat the
    // disjoint tags of its hard negative. JA is measured separately —
    // the P0 record left SigLIP 2's Japanese behaviour unverified, and
    // this is the verification.
    let mut tag = TagMeasure::default();
    let mut tag_ja = TagMeasure::default();
    for (i, spec) in specs.iter().enumerate() {
        let Some(negative) = &spec.hard_negative else {
            continue;
        };
        tag.observe(
            &mut encoder,
            &bases[i],
            &spec.scene.tags_en()[0],
            &negative.tags_en()[0],
        );
        tag_ja.observe(
            &mut encoder,
            &bases[i],
            &spec.scene.tags_ja()[0],
            &negative.tags_ja()[0],
        );
    }

    // Honest failure: unrelated queries against every base, in both
    // languages.
    let mut unrelated_query_scores = Vec::new();
    for query in unrelated_queries_en() {
        let q = encoder.encode_text(&query).expect("query");
        for base in &bases {
            unrelated_query_scores.push(cosine(base, &q));
        }
    }
    let mut unrelated_query_ja_scores = Vec::new();
    for query in unrelated_queries_ja() {
        let q = encoder.encode_text(&query).expect("query");
        for base in &bases {
            unrelated_query_ja_scores.push(cosine(base, &q));
        }
    }

    // Threshold sweep for the tag-suggestion floor: every base scored
    // against the whole fixture vocabulary (EN and JA together — the
    // production job scores one flat Tag list), ground truth = the
    // base's own tags in both languages. Reported per threshold so the
    // floor is picked from a curve, not a pair of means.
    let vocabulary: Vec<String> = {
        let mut seen = Vec::new();
        for spec in &specs {
            for tag in spec.scene.tags_en().into_iter().chain(spec.scene.tags_ja()) {
                if !seen.contains(&tag) {
                    seen.push(tag);
                }
            }
        }
        seen
    };
    let vocab_vectors: Vec<Vec<f32>> = vocabulary
        .iter()
        .map(|name| encoder.encode_text(name).expect("vocab"))
        .collect();
    let mut sweep = Vec::new();
    for threshold in [0.06f32, 0.08, 0.10, 0.12, 0.14, 0.16] {
        let (mut tp, mut fp, mut fn_) = (0usize, 0usize, 0usize);
        for (i, spec) in specs.iter().enumerate() {
            let truth: Vec<&String> = vocabulary
                .iter()
                .filter(|name| {
                    spec.scene.tags_en().contains(name) || spec.scene.tags_ja().contains(name)
                })
                .collect();
            for (name, vector) in vocabulary.iter().zip(&vocab_vectors) {
                let hit = cosine(&bases[i], vector) >= threshold;
                let is_true = truth.contains(&name);
                match (hit, is_true) {
                    (true, true) => tp += 1,
                    (true, false) => fp += 1,
                    (false, true) => fn_ += 1,
                    (false, false) => {}
                }
            }
        }
        sweep.push(serde_json::json!({
            "threshold": threshold,
            "precision": tp as f32 / (tp + fp).max(1) as f32,
            "recall": tp as f32 / (tp + fn_).max(1) as f32,
            "tp": tp, "fp": fp, "fn": fn_,
        }));
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
            "tag": tag.json(),
            "tag_ja": tag_ja.json(),
            "unrelated_query": { "mean": mean(&unrelated_query_scores), "max": max(&unrelated_query_scores) },
            "unrelated_query_ja": { "mean": mean(&unrelated_query_ja_scores), "max": max(&unrelated_query_ja_scores) },
            "tag_threshold_sweep": sweep,
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
        tag.own_beats_disjoint * 10 >= tag.pairs * 8,
        "own EN tags must beat disjoint tags on at least 80% of pairs \
         ({}/{})",
        tag.own_beats_disjoint,
        tag.pairs
    );
    assert!(
        mean(&noise_scores) < mean(&lookalike_scores),
        "noise must sit below the scene family"
    );
    // JA is reported, not asserted: the P0 record calls Japanese
    // behaviour unverified, and a measurement that fails an assertion
    // would hide the number this run exists to produce.
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
