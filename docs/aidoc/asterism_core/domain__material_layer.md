# asterism-core::domain::material_layer

`MaterialLayer` — one band of marks over an Asset's material, and
the thing that says **where a mark came from**.

A material can be marked more than once by more than one hand. The
container itself declares chapters; a person disagrees with them and
writes their own; a job derives a third set from the audio. Before
this type those three were the same rows, distinguishable only by
whoever happened to have written them last — so "re-read the file"
either destroyed a person's work or duplicated the file's, and there
was no answer to "which of these did I write?".

The layer carries that answer as data. [`LayerOrigin`] says who
produced the band, [`LayerRole`] says what kind of thing it holds,
and marks belong to a layer rather than directly to the asset:
[`ChapterMark`](crate::domain::chapter_mark) hangs off a `Structure`
layer, [`MaterialMark`](crate::domain::material_mark) off an
`Annotation` one.

Design notes:

- **The layer is addressed through the asset, not the material.**
  `(asset_id, material_ord)` names the original the band is over,
  the same pair
  [`Material`](crate::domain::material) is identified by, and the
  same one `MaterialMark` resolves to `ord == 0` by convention.
  Materials are aggregate-internal, so there is no material id to
  reference and this is the shape the aggregate offers.
- **`Imported` is immutable, and that rule is not here.** Whether a
  caller may write into a band depends on which caller it is — a
  person editing, or the re-probe job replacing the file's own
  declaration wholesale — and the entity cannot see which. The
  application layer holds it
  ([`material_layer_service`](crate::application::material_layer_service)),
  in the one place both routes pass through.
- **"The default band" is a cross-row fact**, so it is not enforced
  here. At most one layer per `(asset, material_ord, role)` carries
  `is_default`, and that is a partial unique index in the schema (see
  the V78 doc comment in `migrations.rs`): a rule about *other rows*
  cannot be checked by a value holding one of them, and a check that
  read them would be a race between its read and its write.
  [`Self::validate`] carries the half that is self-contained.
- **No display name.** A band is described by what it *is* —
  `(origin, role)` — and a surface renders that pair. Storing a
  caption as well would make "the imported layer" and whatever the
  caption says two answers to one question, and the caption would be
  the one that drifts.

## Types

- `LayerOrigin` — Who produced a layer's contents.
- `LayerRole` — What kind of marks a layer holds.
- `MaterialLayer` — One band of marks over an Asset's material.

## Constants

- `PRIMARY_MATERIAL_ORD` — The `material_ord` of an asset's primary original — the axis

