# asterism-infra::search::visual_retriever

The retriever that answers from stored vectors: `Similar` from an
asset's pixels (#112), and the tail of `Text` from what assets say
about themselves (#32).

A composite over the text retriever. `Similar` becomes a
brute-force cosine scan over the persona's stored feature vectors
under the bound model. `Text` runs full text first and appends what
the meaning layer proposes for assets full text did not name — the
two are not competing rankings, they are an instrument and a
suggestion in that order.

Brute force on purpose — at personal-library scale the whole scan is
a few megabytes of f32, and an ANN structure earns its complexity
only when the P2-5 measurements say the scan misses a latency
target.

Degradation is layered the way the rest of the feature degrades. No
bound encoder means `Similar` declines exactly as the text-only
build declines it, and `Text` answers with full text alone — which
is what it answered with before this layer existed, so a build or a
profile without a model is not a build with a broken search. A bound
encoder with no stored vector for the query asset returns the empty
set: "not encoded yet" is an honest nothing, not an error.

## Types

- `VisualAwareRetriever` — Composite retriever: text unchanged, `Similar` from stored vectors.

