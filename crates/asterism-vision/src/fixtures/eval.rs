//! The fixture evaluation, as one implementation (#112).
//!
//! Two callers grade a model against the fixture set: the model-gated
//! test harness (`tests/model_eval.rs`) and the provider-side
//! qualification tool. If they measured separately they would drift —
//! the floor the tool suggests and the ordering CI asserts must come
//! out of the same arithmetic — so the measurement lives here and both
//! call it.
//!
//! What this module computes is *numbers*; what to assert or suggest
//! from them belongs to the callers. The two floor heuristics are the
//! exception, kept here because they are part of the measurement's
//! meaning: the edge floor is the midpoint of the related family's
//! weakest mean (semantic siblings) and the unrelated family's
//! strongest (hard negatives); the tag floor is the highest swept
//! threshold whose recall still clears one half — the "last step
//! before recall halves" reading that picked 0.12 by hand. Both are
//! suggestions a person reviews, not verdicts.

use crate::encoder::Encoder;
use crate::fixtures::relations::{
    RelatedScenes, RelationStream, unrelated_queries_en, unrelated_queries_ja,
};
use crate::fixtures::scene::{self, SceneSpec, noise_image};

/// What to measure over.
#[derive(Debug, Clone, Copy)]
pub struct EvalConfig {
    /// Fixture-stream seed — the measurement's identity.
    pub seed: u64,
    /// How many base scenes to walk.
    pub bases: usize,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            bases: 24,
        }
    }
}

/// One row of the tag sweep. The raw counts travel with the derived
/// rates so a recorded qualification stays re-derivable without
/// re-running the measurement.
#[derive(Debug, Clone, Copy)]
pub struct SweepPoint {
    /// Suggestion threshold under test.
    pub threshold: f32,
    /// Fraction of hits that were true tags.
    pub precision: f32,
    /// Fraction of true tags that were hit.
    pub recall: f32,
    /// True positives at this threshold.
    pub tp: usize,
    /// False positives at this threshold.
    pub fp: usize,
    /// False negatives at this threshold.
    pub fn_: usize,
}

/// One language's tag-vs-image measurement.
#[derive(Debug, Clone, Copy, Default)]
pub struct TagMeasure {
    /// Mean cosine of an image against its own first tag.
    pub own_mean: f32,
    /// Pairs where the own tag beat the hard negative's disjoint tag.
    pub own_beats_disjoint: usize,
    /// Pairs measured.
    pub pairs: usize,
}

/// Everything one pass over the fixtures measured.
#[derive(Debug, Clone)]
pub struct EvalOutcome {
    /// The stream seed the numbers are a function of.
    pub seed: u64,
    /// Bases walked.
    pub bases: usize,
    /// Mean cosine of base × look-alike.
    pub lookalike_mean: f32,
    /// Look-alike-beats-hard-negative pairs, over pairs measured.
    pub lookalike_beats_negative: (usize, usize),
    /// Mean cosine of base × semantic sibling.
    pub sibling_mean: f32,
    /// Sibling-beats-hard-negative pairs, over pairs measured.
    pub sibling_beats_negative: (usize, usize),
    /// Mean cosine of base × its hard negative.
    pub hard_negative_mean: f32,
    /// Mean cosine of base × the next base (strangers).
    pub stranger_mean: f32,
    /// Mean and max cosine of bases × noise canvases.
    pub noise: (f32, f32),
    /// EN tag measurement.
    pub tag: TagMeasure,
    /// JA tag measurement.
    pub tag_ja: TagMeasure,
    /// Mean and max cosine of bases × unrelated EN queries.
    pub unrelated_query: (f32, f32),
    /// Mean and max cosine of bases × unrelated JA queries.
    pub unrelated_query_ja: (f32, f32),
    /// The tag-threshold sweep, EN and JA vocabulary together.
    pub sweep: Vec<SweepPoint>,
}

impl EvalOutcome {
    /// Suggested visual-edge floor: the midpoint between the related
    /// family's weakest mean and the unrelated family's strongest —
    /// computed as stated, so a model whose look-alikes score below
    /// its siblings (or whose strangers outscore its hard negatives)
    /// moves the floor instead of silently breaking the reading.
    pub fn suggested_edge_floor(&self) -> f32 {
        let weakest_related = self.lookalike_mean.min(self.sibling_mean);
        let strongest_unrelated = self.hard_negative_mean.max(self.stranger_mean);
        (weakest_related + strongest_unrelated) / 2.0
    }

    /// Suggested tag-suggestion floor: the highest swept threshold
    /// whose recall still clears 0.5.
    pub fn suggested_tag_floor(&self) -> Option<f32> {
        self.sweep
            .iter()
            .filter(|p| p.recall >= 0.5)
            .map(|p| p.threshold)
            .fold(None, |best, t| Some(best.map_or(t, |b: f32| b.max(t))))
    }

    /// The measurement as the one JSON line both callers record.
    pub fn to_json(&self, model_id: &str) -> serde_json::Value {
        serde_json::json!({
            "schema": "vision-model-eval-v1",
            "model_id": model_id,
            "seed": self.seed,
            "bases": self.bases,
            "lookalike": {
                "mean": self.lookalike_mean,
                "beats_negative": self.lookalike_beats_negative.0,
                "pairs": self.lookalike_beats_negative.1,
            },
            "sibling": {
                "mean": self.sibling_mean,
                "beats_negative": self.sibling_beats_negative.0,
                "pairs": self.sibling_beats_negative.1,
            },
            "hard_negative_mean": self.hard_negative_mean,
            "stranger_mean": self.stranger_mean,
            "noise": { "mean": self.noise.0, "max": self.noise.1 },
            "tag": {
                "own_mean": self.tag.own_mean,
                "own_beats_disjoint": self.tag.own_beats_disjoint,
                "pairs": self.tag.pairs,
            },
            "tag_ja": {
                "own_mean": self.tag_ja.own_mean,
                "own_beats_disjoint": self.tag_ja.own_beats_disjoint,
                "pairs": self.tag_ja.pairs,
            },
            "unrelated_query": { "mean": self.unrelated_query.0, "max": self.unrelated_query.1 },
            "unrelated_query_ja": { "mean": self.unrelated_query_ja.0, "max": self.unrelated_query_ja.1 },
            "tag_threshold_sweep": self.sweep.iter().map(|p| serde_json::json!({
                "threshold": p.threshold,
                "precision": p.precision,
                "recall": p.recall,
                "tp": p.tp,
                "fp": p.fp,
                "fn": p.fn_,
            })).collect::<Vec<_>>(),
            "suggested": {
                "edge_floor": self.suggested_edge_floor(),
                "tag_floor": self.suggested_tag_floor(),
            },
        })
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len().max(1) as f32
}

fn max(v: &[f32]) -> f32 {
    v.iter().copied().fold(f32::MIN, f32::max)
}

fn encode_scene(encoder: &mut Encoder, spec: &SceneSpec) -> anyhow::Result<Vec<f32>> {
    let img = scene::render(spec)?;
    encoder.encode_image(img.as_raw(), img.width(), img.height())
}

struct TagAccumulator {
    own_scores: Vec<f32>,
    own_beats_disjoint: usize,
    pairs: usize,
}

impl TagAccumulator {
    fn new() -> Self {
        Self {
            own_scores: Vec::new(),
            own_beats_disjoint: 0,
            pairs: 0,
        }
    }

    fn observe(
        &mut self,
        encoder: &mut Encoder,
        base: &[f32],
        own: &str,
        disjoint: &str,
    ) -> anyhow::Result<()> {
        let own_score = cosine(base, &encoder.encode_text(own)?);
        let disjoint_score = cosine(base, &encoder.encode_text(disjoint)?);
        self.own_scores.push(own_score);
        self.pairs += 1;
        if own_score > disjoint_score {
            self.own_beats_disjoint += 1;
        }
        Ok(())
    }

    fn finish(self) -> TagMeasure {
        TagMeasure {
            own_mean: mean(&self.own_scores),
            own_beats_disjoint: self.own_beats_disjoint,
            pairs: self.pairs,
        }
    }
}

/// One pass over the fixture set with a real encoder.
pub fn run(encoder: &mut Encoder, config: &EvalConfig) -> anyhow::Result<EvalOutcome> {
    let specs: Vec<RelatedScenes> = RelationStream::new(config.seed)
        .take(config.bases)
        .collect();

    let bases: Vec<Vec<f32>> = specs
        .iter()
        .map(|s| encode_scene(encoder, &s.scene))
        .collect::<anyhow::Result<_>>()?;
    let noise: Vec<Vec<f32>> = (0..3)
        .map(|i| {
            let img = noise_image(config.seed ^ 0xC0FF_EE00 ^ i, 640, 640);
            encoder.encode_image(img.as_raw(), img.width(), img.height())
        })
        .collect::<anyhow::Result<_>>()?;

    let (mut lookalike_beats, mut lookalike_pairs) = (0usize, 0usize);
    let (mut sibling_beats, mut sibling_pairs) = (0usize, 0usize);
    let mut lookalike_scores = Vec::new();
    let mut sibling_scores = Vec::new();
    let mut negative_scores = Vec::new();
    let mut stranger_scores = Vec::new();
    let mut noise_scores = Vec::new();

    for (i, spec) in specs.iter().enumerate() {
        let base = &bases[i];
        let negative = match &spec.hard_negative {
            Some(n) => Some(cosine(base, &encode_scene(encoder, n)?)),
            None => None,
        };
        if let Some(alike) = &spec.lookalike {
            let score = cosine(base, &encode_scene(encoder, alike)?);
            lookalike_scores.push(score);
            if let Some(neg) = negative {
                lookalike_pairs += 1;
                if score > neg {
                    lookalike_beats += 1;
                }
            }
        }
        if let Some(sibling) = &spec.semantic_sibling {
            let score = cosine(base, &encode_scene(encoder, sibling)?);
            sibling_scores.push(score);
            if let Some(neg) = negative {
                sibling_pairs += 1;
                if score > neg {
                    sibling_beats += 1;
                }
            }
        }
        if let Some(neg) = negative {
            negative_scores.push(neg);
        }
        if i + 1 != bases.len()
            && let Some(other) = bases.get((i + 1) % bases.len())
        {
            stranger_scores.push(cosine(base, other));
        }
        for n in &noise {
            noise_scores.push(cosine(base, n));
        }
    }

    let mut tag = TagAccumulator::new();
    let mut tag_ja = TagAccumulator::new();
    for (i, spec) in specs.iter().enumerate() {
        let Some(negative) = &spec.hard_negative else {
            continue;
        };
        tag.observe(
            encoder,
            &bases[i],
            &spec.scene.tags_en()[0],
            &negative.tags_en()[0],
        )?;
        tag_ja.observe(
            encoder,
            &bases[i],
            &spec.scene.tags_ja()[0],
            &negative.tags_ja()[0],
        )?;
    }

    let mut unrelated_scores = Vec::new();
    for query in unrelated_queries_en() {
        let q = encoder.encode_text(&query)?;
        for base in &bases {
            unrelated_scores.push(cosine(base, &q));
        }
    }
    let mut unrelated_ja_scores = Vec::new();
    for query in unrelated_queries_ja() {
        let q = encoder.encode_text(&query)?;
        for base in &bases {
            unrelated_ja_scores.push(cosine(base, &q));
        }
    }

    // The tag-threshold sweep: EN and JA vocabulary together, the way
    // the production job scores one flat Tag list.
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
        .map(|name| encoder.encode_text(name))
        .collect::<anyhow::Result<_>>()?;
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
        sweep.push(SweepPoint {
            threshold,
            precision: tp as f32 / (tp + fp).max(1) as f32,
            recall: tp as f32 / (tp + fn_).max(1) as f32,
            tp,
            fp,
            fn_,
        });
    }

    Ok(EvalOutcome {
        seed: config.seed,
        bases: config.bases,
        lookalike_mean: mean(&lookalike_scores),
        lookalike_beats_negative: (lookalike_beats, lookalike_pairs),
        sibling_mean: mean(&sibling_scores),
        sibling_beats_negative: (sibling_beats, sibling_pairs),
        hard_negative_mean: mean(&negative_scores),
        stranger_mean: mean(&stranger_scores),
        noise: (mean(&noise_scores), max(&noise_scores)),
        tag: tag.finish(),
        tag_ja: tag_ja.finish(),
        unrelated_query: (mean(&unrelated_scores), max(&unrelated_scores)),
        unrelated_query_ja: (mean(&unrelated_ja_scores), max(&unrelated_ja_scores)),
        sweep,
    })
}
