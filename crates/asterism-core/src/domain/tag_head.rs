//! The trained tag head (#132 phase 2): per-tag logistic rows over
//! frozen embeddings — pure math, no IO, no new dependencies.
//!
//! A head row is a binary logistic classifier for one tag, trained on
//! the person's own rulings: an `accepted` suggestion is a positive
//! example, a `rejected` one a negative, and the input is the asset's
//! **cached** vector — training re-scores what the encoder already
//! produced and never re-encodes anything.
//!
//! ## Why no ML crate
//!
//! The issue left "linfa or candle" to be decided in-phase; the
//! decision is neither. One row is a convex problem in at most a few
//! thousand parameters over at most a few hundred examples — plain
//! full-batch gradient descent converges in milliseconds, and the
//! whole trainer is a page of arithmetic that a test can pin
//! end-to-end. A tensor framework would buy nothing but a dependency
//! graph.
//!
//! ## The gate is the safety
//!
//! With rulings this sparse, a trained row can easily be worse than
//! zero-shot. Nothing here is promoted on faith: every tag holds out
//! part of its rulings ([`split_for_holdout`], deterministic — no RNG,
//! so a rerun over the same rulings reproduces the same split), and
//! [`evaluate_row`] scores the candidate **and** the caller-supplied
//! zero-shot baseline on the same held-out examples. The caller
//! promotes only on a strict win ([`HeadEval::is_win`]); a head that
//! cannot beat cosine stays unpromoted and the zero-shot pass simply
//! continues.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::value::TagId;
use crate::domain::visual::TagHeadRef;
use crate::error::DomainError;

/// The promoted head as the scoring pass holds it: a label and the
/// trained rows, keyed by tag. A tag without a row keeps zero-shot —
/// the artifact's contract, now in memory.
///
/// Bound once at startup beside the encoder and never swapped in a
/// running process; a newer promotion applies on the next launch. The
/// probability a trained row produces sits on a different scale from
/// cosine (0..1 versus roughly −1..1), and both land in the same
/// suggestion queue — a known wart, sorted per asset, revisited when
/// a screen renders the scores side by side.
#[derive(Debug, Clone)]
pub struct BoundTagHead {
    /// The label suggestions and stamps carry while this head scores.
    pub head: TagHeadRef,
    /// The trained rows; absent tags score zero-shot.
    pub rows: BTreeMap<TagId, TrainedRow>,
}

/// Minimum rulings **per class** (accepted, rejected) before a tag is
/// trainable: three to learn from and one to hold out, under
/// [`HOLD_OUT_EVERY`]. Below this the tag keeps its zero-shot row —
/// a v0 constant, expected to be re-measured once real ruling volumes
/// exist.
pub const MIN_RULINGS_PER_CLASS: usize = 4;

/// Every N-th example of each class is held out for the eval (the
/// N-th, 2N-th, …). Deterministic by construction: the split is a
/// function of ruling order, which the storage returns stably, so
/// reruns reproduce it.
pub const HOLD_OUT_EVERY: usize = 4;

/// Gradient-descent schedule for one row. Full batch; the learning
/// rate and iteration count are sized for L2-normalized inputs and
/// example counts in the tens-to-hundreds, and the ridge term keeps a
/// barely-separable tag from running its weights out.
const GD_ITERATIONS: usize = 300;
const GD_LEARNING_RATE: f32 = 0.5;
const GD_L2: f32 = 0.01;

/// One ruling as the trainer consumes it: the asset's cached vector
/// and which way the person ruled.
#[derive(Debug, Clone)]
pub struct RulingExample {
    /// The asset's stored vector under the current encoder identity.
    pub vector: Vec<f32>,
    /// `true` for an accepted suggestion, `false` for a rejected one.
    pub accepted: bool,
}

/// One trained row: `sigmoid(w · v + b)` is the probability the
/// person would accept this tag for a vector `v`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainedRow {
    /// The weight vector, same dimensionality as the encoder's output.
    pub weights: Vec<f32>,
    /// The bias term.
    pub bias: f32,
}

impl TrainedRow {
    /// The acceptance probability for one vector.
    pub fn score(&self, vector: &[f32]) -> f32 {
        let z: f32 = self
            .weights
            .iter()
            .zip(vector)
            .map(|(w, v)| w * v)
            .sum::<f32>()
            + self.bias;
        sigmoid(z)
    }

    /// The decision the eval (and a scoring pass) reads off the
    /// probability: accept at even odds or better.
    pub fn predicts_accept(&self, vector: &[f32]) -> bool {
        self.score(vector) >= 0.5
    }
}

/// Held-out outcome of one tag's candidate row against the baseline,
/// on identical examples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowEval {
    /// Held-out examples scored.
    pub held_out: usize,
    /// How many the trained row called the way the person ruled.
    pub candidate_correct: usize,
    /// How many the zero-shot baseline called that way.
    pub baseline_correct: usize,
}

/// The whole head's eval: the per-row outcomes summed. Promotion reads
/// [`Self::is_win`] and nothing else.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadEval {
    /// Total held-out examples across trained tags.
    pub held_out: usize,
    /// Candidate calls matching the rulings.
    pub candidate_correct: usize,
    /// Baseline calls matching the rulings.
    pub baseline_correct: usize,
}

impl HeadEval {
    /// Folds one row's outcome in.
    pub fn absorb(&mut self, row: RowEval) {
        self.held_out += row.held_out;
        self.candidate_correct += row.candidate_correct;
        self.baseline_correct += row.baseline_correct;
    }

    /// A strict win on non-empty evidence: the candidate called more
    /// held-out rulings correctly than the baseline did. Ties lose —
    /// a head that merely matches cosine is churn (a new head ref
    /// re-walks the library) with nothing bought.
    pub fn is_win(&self) -> bool {
        self.held_out > 0 && self.candidate_correct > self.baseline_correct
    }
}

/// Splits one tag's rulings into train and held-out, per class, every
/// [`HOLD_OUT_EVERY`]-th example held out. Refuses a tag below
/// [`MIN_RULINGS_PER_CLASS`] on either side: with less there is
/// nothing honest to hold out, and an uneval'd row must not exist.
pub fn split_for_holdout(
    examples: &[RulingExample],
) -> Result<(Vec<RulingExample>, Vec<RulingExample>), DomainError> {
    let (accepted, rejected): (Vec<_>, Vec<_>) = examples.iter().cloned().partition(|e| e.accepted);
    if accepted.len() < MIN_RULINGS_PER_CLASS || rejected.len() < MIN_RULINGS_PER_CLASS {
        return Err(DomainError::Validation(format!(
            "a trainable tag needs {MIN_RULINGS_PER_CLASS} rulings per class; \
             got {} accepted, {} rejected",
            accepted.len(),
            rejected.len()
        )));
    }
    let mut train = Vec::new();
    let mut held = Vec::new();
    for class in [accepted, rejected] {
        for (index, example) in class.into_iter().enumerate() {
            if (index + 1) % HOLD_OUT_EVERY == 0 {
                held.push(example);
            } else {
                train.push(example);
            }
        }
    }
    Ok((train, held))
}

/// Trains one row by full-batch gradient descent on the training
/// split. Deterministic: zero init, fixed schedule, no shuffling.
pub fn train_row(dim: usize, train: &[RulingExample]) -> Result<TrainedRow, DomainError> {
    if train.is_empty() {
        return Err(DomainError::Validation(
            "cannot train a row on zero examples".into(),
        ));
    }
    for example in train {
        if example.vector.len() != dim {
            return Err(DomainError::Validation(format!(
                "a training vector has dimension {}, the encoder produces {dim}",
                example.vector.len()
            )));
        }
    }
    let mut row = TrainedRow {
        weights: vec![0.0; dim],
        bias: 0.0,
    };
    let n = train.len() as f32;
    for _ in 0..GD_ITERATIONS {
        let mut grad_w = vec![0.0f32; dim];
        let mut grad_b = 0.0f32;
        for example in train {
            let error = row.score(&example.vector) - if example.accepted { 1.0 } else { 0.0 };
            for (g, v) in grad_w.iter_mut().zip(&example.vector) {
                *g += error * v;
            }
            grad_b += error;
        }
        for (w, g) in row.weights.iter_mut().zip(&grad_w) {
            // Ridge on the weights, never the bias — the base rate is
            // the bias's to carry.
            *w -= GD_LEARNING_RATE * (g / n + GD_L2 * *w);
        }
        row.bias -= GD_LEARNING_RATE * grad_b / n;
    }
    Ok(row)
}

/// Scores the candidate and the caller's zero-shot baseline on the
/// same held-out examples. The baseline closure answers "would the
/// zero-shot pass have suggested this tag for this vector" — the
/// cosine-and-floor rule, supplied by the layer that owns the floor.
pub fn evaluate_row(
    row: &TrainedRow,
    held: &[RulingExample],
    baseline_suggests: impl Fn(&[f32]) -> bool,
) -> RowEval {
    let mut eval = RowEval {
        held_out: held.len(),
        ..RowEval::default()
    };
    for example in held {
        if row.predicts_accept(&example.vector) == example.accepted {
            eval.candidate_correct += 1;
        }
        if baseline_suggests(&example.vector) == example.accepted {
            eval.baseline_correct += 1;
        }
    }
    eval
}

fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rulings that are linearly separable along the first axis, with
    /// enough per class to train and hold out.
    fn separable(n_per_class: usize) -> Vec<RulingExample> {
        let mut examples = Vec::new();
        for i in 0..n_per_class {
            let wiggle = (i as f32) * 0.01;
            examples.push(RulingExample {
                vector: vec![0.9 - wiggle, 0.1 + wiggle, 0.0],
                accepted: true,
            });
            examples.push(RulingExample {
                vector: vec![-0.8 + wiggle, 0.2, 0.1],
                accepted: false,
            });
        }
        examples
    }

    #[test]
    fn a_separable_tag_trains_to_a_row_that_beats_a_coin() {
        let (train, held) = split_for_holdout(&separable(8)).unwrap();
        assert!(!held.is_empty());
        let row = train_row(3, &train).unwrap();
        // Every held-out example is on the right side.
        let eval = evaluate_row(&row, &held, |_| false);
        assert_eq!(eval.candidate_correct, eval.held_out, "{eval:?}");
    }

    #[test]
    fn the_split_is_deterministic_and_keeps_both_classes_in_both_halves() {
        let examples = separable(8);
        let (train_a, held_a) = split_for_holdout(&examples).unwrap();
        let (train_b, held_b) = split_for_holdout(&examples).unwrap();
        assert_eq!(train_a.len(), train_b.len());
        assert_eq!(held_a.len(), held_b.len());
        assert!(held_a.iter().any(|e| e.accepted) && held_a.iter().any(|e| !e.accepted));
        assert!(train_a.iter().any(|e| e.accepted) && train_a.iter().any(|e| !e.accepted));
    }

    #[test]
    fn too_few_rulings_refuse_to_train_rather_than_overfit() {
        let mut examples = separable(MIN_RULINGS_PER_CLASS);
        examples.pop(); // one class drops below the floor
        assert!(split_for_holdout(&examples).is_err());
        assert!(train_row(3, &[]).is_err());
    }

    #[test]
    fn the_eval_scores_candidate_and_baseline_on_identical_examples() {
        let held = separable(2);
        let oracle = TrainedRow {
            weights: vec![10.0, 0.0, 0.0],
            bias: 0.0,
        };
        // The candidate is right about everything; the baseline always
        // says "suggest", which is right only for the accepted half.
        let eval = evaluate_row(&oracle, &held, |_| true);
        assert_eq!(eval.held_out, 4);
        assert_eq!(eval.candidate_correct, 4);
        assert_eq!(eval.baseline_correct, 2);

        let mut head = HeadEval::default();
        head.absorb(eval);
        assert!(head.is_win());
        // A tie is not a win: churn with nothing bought.
        let tie = HeadEval {
            held_out: 4,
            candidate_correct: 2,
            baseline_correct: 2,
        };
        assert!(!tie.is_win());
        assert!(!HeadEval::default().is_win(), "empty evidence never wins");
    }

    #[test]
    fn a_dimension_mismatch_is_refused_before_it_poisons_the_row() {
        let bad = vec![RulingExample {
            vector: vec![1.0, 0.0],
            accepted: true,
        }];
        assert!(train_row(3, &bad).is_err());
    }
}
