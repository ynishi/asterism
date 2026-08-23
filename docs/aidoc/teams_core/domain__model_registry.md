# teams-core::domain::model_registry

`model_registry` — the instance's carriage of a qualified model's
registry entry (#126).

The entry (`asterism-model-registry-entry-v1`) is authored by the
provider's tooling (`asterism-model-lab registry`) and consumed by
the member app's fetch flow. Between the two, the instance is a
carrier, not an authority (#126 decision 2): it stores and re-serves
the provider's bytes **verbatim**, and this module deliberately
types no field of the entry's body — no digests, no URLs, no
qualification report. Parsing those here would grow the hosted
plane a reading of the model contract that #83 §4's dependency rule
keeps out (`teams-*` → `asterism-core` only; the entry's typed home
is beside `ModelPackage`, on the app side of the split).

What *is* validated is the envelope — the part the instance answers
for as a carrier: the bytes are one JSON object, the `schema` field
names the one version this instance knows how to carry, and
`model_id` is a non-empty string the storage layer can key history
by. A carrier that accepted arbitrary bytes would serve members
something their fetch flow cannot consume, and could not say which
model superseded which.

## Types

- `ModelRegistryEntry` — A validated registry entry: the provider's bytes, verbatim, plus

## Constants

- `ENTRY_SCHEMA_V1` — The one entry schema this plane carries. A future `-v2` is a new

