# asterism-core::domain::visual

Visual-feature vocabulary and the encoder port (#112).

Everything a model produces is **derived state with an identity**:
a vector is meaningless without knowing which model, at which
preprocessing revision, produced it. [`ModelIdentity`] is that
identity, and it travels with every stored vector, every suggestion,
and every synthetic edge a model produces — so replacing a model
invalidates exactly its own output and nothing a person asserted.

The core knows the *shape* of the work — identities, vectors, the
encoder port — and none of the machinery. ONNX Runtime, weights,
preprocessing kernels live behind [`VisualEncoder`] in the model-use
crate; this module must never grow a model dependency (that is
acceptance item 4 of #112, and the reason the port takes a raw RGB
buffer rather than an image type).

## Functions

- `cosine_normalized` — Cosine similarity of two L2-normalized vectors: the dot product.

## Types

- `ModelIdentity` — The derivation identity of everything one model configuration
- `TagEvidence` — One scored tag suggestion with its full derivation identity.
- `TagHeadRef` — Which scoring head proposed a suggestion (#132, the identity
- `TagSuggestionDisposition` — Where one tag suggestion stands between the model and the person.
- `VisualFeature` — One stored feature vector and its full derivation identity.
- `VisualFeatureKind` — What kind of feature a stored vector is.

## Traits

- `VisualEncoder` — Encoder port: pixels (or text) in, one normalized vector out.

