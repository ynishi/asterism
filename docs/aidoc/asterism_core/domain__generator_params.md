# asterism-core::domain::generator_params

`generator_params` — what an extraction concluded about the
parameters a generator recorded, and the port the conclusion comes
through.

The values in question — the checkpoint a run loaded, the seed it
sampled with — are not stored as fields. They sit inside the
free-text metadata values the import path copied verbatim
([`material_meta`](crate::domain::material_meta)'s rule: strings,
unparsed), and reading them out is a parser, not a mapping. What is
extractable is decided **per file, not per generator family**: the
same input on the same node class holds a literal in one graph and a
link to another node's output in the next. That is why this is a
port with an outcome vocabulary rather than a lookup table.

# The three layers, and which one this is

The split is the one [`probe`](crate::domain::probe) argues for —
"the parser is the part that grows per format, and it is the part
that belongs furthest from here":

| layer | holds |
|---|---|
| this module, in the core | the outcome vocabulary and the trait |
| `asterism-media-probe` | the pure grammar — the A1111 line tokeniser, a function of a string with no opinion about which keys matter |
| `asterism-infra` | the judgement and the registry — which input key names a seed, what a two-element array means, which families are recognised at all |

# Not on the artefact probe

That port is keyed by container mime and selected before any byte is
read, and a generator family is not a mime — one `image/png` may be
ComfyUI, A1111, InvokeAI or NovelAI, knowable only after the
metadata is read. It already refused a third axis once, for raw
metadata, on the same reasoning
([`ArtefactProbe::meta_raw_of`](crate::domain::probe::ArtefactProbe::meta_raw_of)).
The input here is the **stored metadata rows** — the canonical
object [`read_evidence`](crate::domain::disclosure::read_evidence)
already parses — so an extractor re-runs across the whole library
without opening a single file.

# Workflow identity is not here

It is not extractable from either family — the ComfyUI graph mints
no run id, and A1111's grammar has no such field — so there is no
`workflow` member of [`GeneratorParams`] to be perpetually absent.
If a workflow identity ever reaches the manifest it is a value the
user supplies, on the hand-assertion footing
[`asserted_source_type`](crate::domain::disclosure::asserted_source_type)
established, not an extraction.

# Extraction does not touch the meta axis

The digest and its canonical form say one thing — *the container
carried this text* — and extraction changes neither that input nor
that definition. An extractor reads the stored values; it never
rewrites them, so every digest stands exactly as it did before the
extractor existed.

## Types

- `GeneratorParams` — One extraction's conclusion about every parameter it answers for.
- `ParamExtraction` — What an extraction concluded about one parameter.

## Traits

- `ParamExtractor` — Reads generator parameters out of stored metadata.

