# asterism-core::domain::tag_head

The trained tag head (#132 phase 2): per-tag logistic rows over
frozen embeddings — pure math, no IO, no new dependencies.

A head row is a binary logistic classifier for one tag, trained on
the person's own rulings: an `accepted` suggestion is a positive
example, a `rejected` one a negative, and the input is the asset's
**cached** vector — training re-scores what the encoder already
produced and never re-encodes anything.

## Why no ML crate

The issue left "linfa or candle" to be decided in-phase; the
decision is neither. One row is a convex problem in at most a few
thousand parameters over at most a few hundred examples — plain
full-batch gradient descent converges in milliseconds, and the
whole trainer is a page of arithmetic that a test can pin
end-to-end. A tensor framework would buy nothing but a dependency
graph.

## The gate is the safety

With rulings this sparse, a trained row can easily be worse than
zero-shot. Nothing here is promoted on faith: every tag holds out
part of its rulings ([`split_for_holdout`], deterministic — no RNG,
so a rerun over the same rulings reproduces the same split), and
[`evaluate_row`] scores the candidate **and** the caller-supplied
zero-shot baseline on the same held-out examples. The caller
promotes only on a strict win ([`HeadEval::is_win`]); a head that
cannot beat cosine stays unpromoted and the zero-shot pass simply
continues.

## Functions

- `evaluate_row` — Scores the candidate and the caller's zero-shot baseline on the
- `split_for_holdout` — Splits one tag's rulings into train and held-out, per class, every
- `train_row` — Trains one row by full-batch gradient descent on the training

## Types

- `HeadEval` — The whole head's eval: the per-row outcomes summed. Promotion reads
- `RowEval` — Held-out outcome of one tag's candidate row against the baseline,
- `RulingExample` — One ruling as the trainer consumes it: the asset's cached vector
- `TrainedRow` — One trained row: `sigmoid(w · v + b)` is the probability the

## Constants

- `HOLD_OUT_EVERY` — Every N-th example of each class is held out for the eval (the
- `MIN_RULINGS_PER_CLASS` — Minimum rulings **per class** (accepted, rejected) before a tag is

