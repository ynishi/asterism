# asterism-core::application::disclosure_service

Building an artefact's disclosure out of the library, and putting it
into a file.

Two verbs, and the second is the one the acceptance criteria are
really about:

- [`record_for`](DisclosureService::record_for) — assemble what an
  asset discloses from what is *stored*: the container metadata a
  probe read, the `derived_from` edges the library recorded, the
  asset's own title.
- [`apply_to`](DisclosureService::apply_to) — write that record into
  a file through the [`DisclosureWriter`] port.

# Why this makes the database the source of truth

Nothing here reads the target file's metadata. The record is derived
entirely from rows, so a file that came back from a downstream
conversion with its manifest stripped can be handed to
[`apply_to`](DisclosureService::apply_to) and get the same disclosure
again — the answer never lived in the file. That is the property a
manifest cannot have on its own, since any re-encode removes it.

# Why the port is here and not in `repository`

[`DisclosureWriter`] is an outbound port like the repositories, and
it lives in the core for the same reason they do — adapters
implement traits, they do not define them (`asterism-infra`'s crate
doc). It is declared beside its only caller rather than in
[`repository`](crate::domain::repository) because it is not one: a
repository owns the storage of an entity, and this one owns no
entity at all — it takes a value and a path and modifies a file
neither it nor this service owns.

## Types

- `DisclosureService` — Assembles and applies AI-disclosure provenance.

## Traits

- `DisclosureWriter` — Writes a disclosure into a file that already exists.

