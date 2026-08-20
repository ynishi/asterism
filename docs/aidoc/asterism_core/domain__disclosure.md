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
| a generator's own keys, and every recorded parent is itself synthetic (or there are none) | `trainedAlgorithmicMedia` |
| a generator's own keys, and some recorded parent is **not** synthetic | `compositeWithTrainedAlgorithmicMedia` |
| no generator keys, but EXIF names a camera | `digitalCapture` |
| none of the above | nothing is written |

The last row is the important one. An artefact nothing established
gets no `DigitalSourceType` property, which is a different statement
from every term in the vocabulary — the same reading
[`attribution`](crate::domain::attribution) gives an absent
author. A missing mark on a synthetic file is a gap; a wrong mark on
one is a false statement, and only the second is unrecoverable.

Two terms have no automatic producer and are not reachable from here
at all.
[`HumanEdits`](DigitalSourceType::HumanEdits)
would have to be inferred from the *absence* of a machine, which
manufactures exactly the evidence a copyright claim needs it to
record.
[`AlgorithmicMedia`](DigitalSourceType::AlgorithmicMedia)
needs a producer that says so about itself, and none in this corpus
does. Both remain assertable by hand.

## Functions

- `is_synthetic` — Whether a container's metadata says a generator made the file.
- `read_evidence` — Reads the canonical metadata object a probe stored.
- `record_for` — Builds the record for one artefact.

## Types

- `ContainerEvidence` — What a container's metadata establishes about how a file was made.
- `ParentEvidence` — What a parent contributes to its child's disclosure.
- `PromptDisclosure` — Whether an artefact's prompt is disclosed in the exported file.

## Constants

- `DISCLOSURE_NOTE_KEY` — Key under which what became of an artefact's disclosure is recorded,

