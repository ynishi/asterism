# asterism-core::domain::material

`Material` — the physical-original layer of an asset (asset-model v4).

A material is the bytes-side fact record of one original artefact:
where it lives (`locator`), how big it is, and what shape its data
has (`mime`). It answers "what is this made of" — the question the
old `ContentKind` reached through the modality master, conflating
data format with user classification and container structure.

Materials are aggregate-internal to [`Asset`](crate::domain::asset::Asset):
identified by `(owning asset, ord)` — the PhotoKit `PHAssetResource`
shape — and never referenced from outside the aggregate. An asset
with [`AssetRole::Collection`](crate::domain::value::AssetRole)
carries no materials: a container has no bytes of its own, its
content is its members.

## Functions

- `guess_mime` — Best-effort mime guess from a locator's file extension.

## Types

- `Material` — One physical original belonging to an asset.

## Constants

- `KNOWN_IMAGE_MIMES` — Every `image/*` value [`guess_mime`] can produce.
- `KNOWN_VIDEO_MIMES` — Every `video/*` value [`guess_mime`] can produce.

