# asterism-core::domain::disclosure

What an artefact discloses about how it was made, and the rule that
decides it.

Three things live here, and they are one concept: the vocabulary
([`DigitalSourceType`], [`DisclosureRecord`], [`Stamped`]), the rule
that turns stored metadata into a statement ([`record_for`]), and the
policy the rule takes ([`PromptDisclosure`]). Rendering a record into
an XMP packet or a C2PA manifest definition is not here — that is
`asterism-disclosure-format`, and putting a packet's bytes beside the
term it carries would be the container formats leaking into the
vocabulary.

# Why the vocabulary is in the core and not in a leaf crate

It was in one, beside the renderers, and that was wrong in a way that
only showed as the feature grew. The renderers need `pngmeta` and a
CRC — a chunk walker — so the core's dependency graph acquired the
container parser this crate's own manifest records evicting. And the
read-back side ("what does this file currently say, and does it still
match its bytes") has to be modelled in the core, because a port
cannot return a type the core cannot name; with the write vocabulary
in a leaf crate, one concept would have been split across two places
the moment the second half arrived.

The split that holds is by *reason to change*: what may be asserted
changes with IPTC, and how a packet is written into a PNG changes
with the container. The first is domain and is here; the second is an
adapter concern and is not.

# The database is the source of truth, not the file

Everything here is derived from stored values —
[`Material::meta_kv`](crate::domain::material::Material::meta_kv) and
the [`DerivedFrom`](crate::domain::edge::EdgeKind::DerivedFrom)
edges — and never from the exported file's own metadata. That is what
makes a manifest re-appliable after a downstream conversion strips
it: the answer was never in the file to begin with.

# Evidence, not inference

Each term is asserted only on evidence that something wrote:

| evidence in the container | term |
|---|---|
| a generator's own keys, and no recorded parent declares a non-model origin | `trainedAlgorithmicMedia` |
| a generator's own keys, and some recorded parent **declares** a non-model origin | `compositeWithTrainedAlgorithmicMedia` |
| no generator keys, but EXIF names a camera | `digitalCapture` |
| none of the above | nothing is written |

Declares, in the second row: a parent whose container says nothing
is unknown ([`ParentOrigin::Unknown`]), and unknown moves nothing —
it is a statement about what the caller knows, not about the file.

The last row is the important one. An artefact nothing established
gets no `DigitalSourceType` property, which is a different statement
from every term in the vocabulary — the same reading
[`attribution`](crate::domain::attribution) gives an absent
author. A missing mark on a synthetic file is a gap; a wrong mark on
one is a false statement, and only the second is unrecoverable.

Two terms have no automatic producer and are never inferred.
[`HumanEdits`](DigitalSourceType::HumanEdits)
would have to be inferred from the *absence* of a machine, which
manufactures exactly the evidence a copyright claim needs it to
record.
[`AlgorithmicMedia`](DigitalSourceType::AlgorithmicMedia)
needs a producer that says so about itself, and none in this corpus
does. Both are assertable by hand, and only by hand.

# Asserted, and then signed verbatim

The hand-assertion route ([`SOURCE_TYPE_KEY`],
[`asserted_source_type`]) is the person's own voice in the table
above, and it outranks every row of it: the certificate the
manifest is signed under is theirs, so their explicit statement is
the claim. The ordinary use is the artefact nothing established —
the scanned film, the file whose metadata a pipeline stripped —
where the assertion is the only voice there is. A parent carrying
one reads as *declared* ([`ParentOrigin::declared`]), never as
unknown.

## Functions

- `asserted_source_type` — The source type a person asserted on this asset, if any.
- `asserted_source_type_entry` — The full statement [`SOURCE_TYPE_KEY`] files, if any.
- `declared_origin` — What a container's metadata declares about the file's origin.
- `evidence_source_type` — The term the container's own evidence establishes, if any.
- `read_evidence` — Reads the canonical metadata object a probe stored.
- `record_for` — Builds the record for one artefact.

## Types

- `AssertedSourceType` — A person's source-type assertion with the statement's own context —
- `ContainerEvidence` — What a container's metadata establishes about how a file was made.
- `ParentEvidence` — What a parent contributes to its child's disclosure.
- `ParentOrigin` — What a parent's own container declares about how it was made.
- `PromptDisclosure` — Whether an artefact's prompt is disclosed in the exported file.

## Constants

- `DISCLOSURE_NOTE_KEY` — Key under which what became of an artefact's disclosure is recorded,
- `SOURCE_TYPE_KEY` — Key under which a person's source-type assertion is recorded, inside

