# asterism-provenance::source_type

`DigitalSourceType` — the one field a synthetic file is obliged to
carry, and the closed set of values this corpus can honestly assert.

The vocabulary is IPTC's, published as a controlled vocabulary at
`cv.iptc.org/newscodes/digitalsourcetype`. The *value* is the term's
URI, not its short name: that is what
`Iptc4xmpExt:DigitalSourceType` is typed as (URI, per the IPTC Photo
Metadata Standard 2025.1 specification), and it is what a C2PA
`c2pa.actions` assertion carries in its `digitalSourceType` field.
One string serves both, which is the reason this type exists at all
rather than each emitter spelling its own.

# Why a closed set, when `Modality` next door is an open slug

An open slug is right when a new value is a data change with no code
behind it. This is the opposite case twice over. Each value here is a
*claim about how a file came to exist*, and the claims are not
interchangeable — the difference between `trainedAlgorithmicMedia`
and `compositeWithTrainedAlgorithmicMedia` is the difference between
"a model made this" and "a model altered a photograph", and a caller
that could pass an arbitrary string could assert either by typo.
Second, the receiving side is closed too: a validator reads the URI
against the published vocabulary, and a term IPTC does not define is
not a weaker claim, it is an unreadable one.

# Why these five and not the whole vocabulary

IPTC defines more terms than this — film and print digitisation,
several composite forms, a deprecated pair. A term is here when a
file in this corpus can arrive with the fact it names, because a
value nothing can produce is a value nothing tests. The three
digitisation terms (`negativeFilm` / `positiveFilm` / `print`)
describe a scanner Asterism has no knowledge of; they can be added
when something can establish them, and adding one is this enum plus
its URI.

# Not asserting is a state

There is no `Unknown` variant. An artefact whose origin nothing
established gets no `DigitalSourceType` property at all — the same
doctrine
[`attribution`](../../asterism_core/domain/attribution/index.html)
states for an absent author: absence is a question nobody has
answered, and a vocabulary term meaning "we do not know" would be an
answer. It also matters legally in the one direction that is not
symmetric: a missing mark on a synthetic file is a gap, while a wrong
mark on one is a false statement.

## Types

- `DigitalSourceType` — How a file came to exist, in IPTC's vocabulary.
- `UnknownSourceType` — A value that is not a term of the IPTC digital source type vocabulary.

