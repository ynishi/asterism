# asterism-infra::search::visual_retriever

The retriever that answers from stored vectors: `Similar` from an
asset's pixels (#112), and the tail of `Text` from what assets say
about themselves (#32).

A composite over the text retriever. `Similar` becomes a
brute-force cosine scan over stored feature vectors under the bound
model. `Text` runs full text first and appends what the meaning
layer proposes for assets full text did not name — the two are not
competing rankings, they are an instrument and a suggestion in that
order.

Brute force on purpose — an ANN structure earns its complexity only
when a measurement says the scan misses a latency target. What is
scanned is no longer always one persona's vectors, so the sizing to
keep in view is the whole library's: #32's own note puts 50k assets
at 512 dimensions of `f32` near 100 MB, and this reads them per
query.

# Both routes scan what the query asked about

The query's scope is the population, and the two routes reach it
differently only because they start differently. `Similar` starts
from an asset, so an unscoped query falls back to that asset's own
persona — the library being asked about is the one the subject
lives in. `Text` starts from words and has no such anchor, so an
unscoped query scans every persona, which is the population full
text answered for the same query.

Requiring a scope on the text route instead is what kept the
meaning layer switched off in the app's default state, where no
persona is selected until somebody selects one.

Degradation is layered the way the rest of the feature degrades. No
bound encoder means `Similar` declines exactly as the text-only
build declines it, and `Text` answers with full text alone — which
is what it answered with before this layer existed, so a build or a
profile without a model is not a build with a broken search. A bound
encoder with no stored vector for the query asset returns the empty
set: "not encoded yet" is an honest nothing, not an error.

## Types

- `VisualAwareRetriever` — Composite retriever: text with the meaning layer appended to it,

