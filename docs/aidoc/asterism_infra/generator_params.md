# asterism-infra::generator_params

Reading generator parameters out of stored metadata rows — the
judgement and the registry behind the
[`ParamExtractor`](asterism_core::domain::generator_params::ParamExtractor)
port.

This is the layer that holds opinions. The core owns the outcome
vocabulary and the trait; `asterism-media-probe::a1111` owns the
grammar of one family's text; what lives here is every judgement
neither of them may make — which input key names a seed, which node
input is a checkpoint, what a two-element array means, and which
families are recognised at all.

# The registry, and how a third family arrives

Two families are recognised: ComfyUI, routed by the keys
[`generator_keys::COMFY`] writes, and AUTOMATIC1111, routed by
[`generator_keys::A1111`] — the same one-fact key list the
disclosure evidence rule reads, which is why it is imported rather
than restated. A family this registry does not recognise (InvokeAI,
NovelAI) reads as
[`NotApplicable`](ParamExtraction::NotApplicable) today; adding one
is a routing arm in [`params_of`](StoredParamExtractor::params_of),
a judgement function beside the two here, and — where the family's
text needs a grammar — a tokeniser in the media-probe crate on
`a1111`'s terms.

# Extractability is decided per file

The same input on the same node class holds a literal in one graph
and a link to another node's output in the next — any custom seed
node, any converted widget. A link is read as
[`Indirect`](ParamExtraction::Indirect), never resolved: the walk
that would resolve it is the future improvement the state exists to
leave findable. And where a graph carries more than one candidate
and they disagree, the answer is
[`Ambiguous`](ParamExtraction::Ambiguous) rather than a winner —
there is no evidence about which sampler produced the file, and a
wrong value in a signed claim is the unrecoverable direction. That
refusal is also why no "which sampler won" is recorded beside the
value: agreement extracts and disagreement refuses, so no choice is
ever made.

## Types

- `StoredParamExtractor` — The extractor over stored rows.

