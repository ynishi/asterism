# asterism-core::domain::disclosure

Turning what the library stored into what a file will disclose.

[`asterism_provenance`] owns the vocabulary and both renderings of
it, and is deliberately unable to decide anything. This module is
where the deciding happens: given the metadata a container carried
and the edges the library recorded, which IPTC term is true of this
artefact, and which of the 2025.1 AI properties can be filled in.

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
from every term in the vocabulary — the same doctrine
[`attribution`](crate::domain::attribution) states for an absent
author. A missing mark on a synthetic file is a gap; a wrong mark on
one is a false statement, and only the second is unrecoverable.

Two terms have no automatic producer and are not reachable from here
at all.
[`HumanEdits`](asterism_provenance::DigitalSourceType::HumanEdits)
would have to be inferred from the *absence* of a machine, which
manufactures exactly the evidence a copyright claim needs it to
record.
[`AlgorithmicMedia`](asterism_provenance::DigitalSourceType::AlgorithmicMedia)
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

