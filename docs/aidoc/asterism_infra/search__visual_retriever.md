# asterism-infra::search::visual_retriever

The retriever that answers `Similar` from stored vectors (#112).

A composite over the text retriever: `Text` delegates unchanged,
`Similar` becomes a brute-force cosine scan over the persona's
stored feature vectors under the bound model. Brute force on
purpose — at personal-library scale the whole scan is a few
megabytes of f32, and an ANN structure earns its complexity only
when the P2-5 measurements say the scan misses a latency target.

Degradation is layered the way the rest of the feature degrades:
no bound encoder means `Similar` declines exactly as the text-only
build declines it; a bound encoder with no stored vector for the
query asset returns the empty set — "not encoded yet" is an honest
nothing, not an error.

## Types

- `VisualAwareRetriever` — Composite retriever: text unchanged, `Similar` from stored vectors.

