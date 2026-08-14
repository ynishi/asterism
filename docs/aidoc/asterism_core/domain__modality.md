# asterism-core::domain::modality

`ModalityDef` — the open Modality master entry (the `modality` table).

Two-layer model:
identity + presentation metadata live here (an **open**, dynamically
addable/removable master row); behaviour lives in the **closed**
[`ContentKind`] the row points at through `kind`. Consumers (jobs /
UI / import) branch on `kind` / capabilities, never on the raw slug.

The row carries only identity + presentation:

- `slug` — the primary key ([`Modality`], an open slug).
- `label` — display name.
- `kind` — the single reference into behaviour ([`ContentKind`]).
- `sort_order` — sidebar rank (the SoT for modality ordering).
- `hidden` — soft-retirement flag (the operational alternative to a
  delete when assets still reference the slug).
- `cover_template` — optional override of the kind's default
  [`CoverTemplate`].

## Types

- `ModalityDef` — One row of the `modality` master — an open, user-editable entry.
- `ModalityView` — A [`ModalityDef`] paired with the number of assets currently

