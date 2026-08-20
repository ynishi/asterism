# asterism-core::domain::material_mark

`MaterialMark` — a mark placed into an Asset's **material**: the
coordinate space the asset's content carries, rather than the asset
as a row.

An asset names one work. Its material is what that
work is made of, and a material has somewhere to point *inside*:
a time axis `[0, duration_ms)` for video and audio, a plane for
images and frames. [`MaterialAnchor`] is that "where", and the mark
is one note fastened to it.

"Material" is [`Material`](crate::domain::material)'s word
(asset-model v4): the physical-original layer of an asset. The mark
nevertheless stores `asset_id`, not a material reference, and that
is not an inconsistency: materials are aggregate-internal —
identified by `(owning asset, ord)` and never referenced from
outside the aggregate — and the axis the anchor measures is the
asset's playback presentation of its primary original (`ord == 0`),
the same axis `asset.duration_ms` describes.

Not a comment. [`AssetComment`](crate::domain::asset_comment) is a
thread hanging off an Asset as a whole and reads in posting order;
a mark points into the material and reads in the material's own
order. The two answer different questions ("what was said about
this" versus "what is here"), which is why this is a separate
aggregate rather than a nullable position column on the comment row.

Design notes:

- **The anchor is the axis, not the type.** A second coordinate
  space (a rectangle on an image, say) arrives as another
  [`MaterialAnchor`] variant, not another aggregate — the shape
  W3C Annotation gives its target + selector. Before this split the
  coordinate space was baked into the type name, and every new one
  cost a type, a table and an adapter.
- **Position is mandatory** (`anchor`, not `Option<MaterialAnchor>`)
  — a mark with no position is a comment, and that already exists.
- **The body carries the whole content.** No tag / kind axis: a tag
  axis would make "a mark with a tag and no body" expressible, and
  the non-empty `body` rule is the one invariant worth keeping while
  the requirement is still moving. Adding tags later is a join
  table, with this table unchanged.
- **Author is [`CommentAuthor`]**, reused rather than restated. The
  `Comment` in that name reads oddly here, but two spellings of the
  same author vocabulary would be the worse of the two problems.
  `body` / `author` / `created_at` / `edited_at` deliberately carry
  the same names and types as on `AssetComment`, so that the shared
  note vocabulary can be lifted out mechanically when there is a
  reason to.
- **`body` is a public field** (as on `AssetComment`), so a record
  update can empty it. The rule is therefore enforced at every door:
  at construction, on the way into storage
  ([`MaterialMark::validate`], which an adapter's `save` calls),
  and on the way back out ([`MaterialMark::rehydrate`]). The
  schema deliberately holds no `body` CHECK, because SQL's `trim` is
  a weaker predicate than Rust's and a weaker mirror is worse than
  none — so if the write door let a value past, the read door would
  be the first thing to see it, and by then it is a stored row that
  the only listing verb refuses.

## Types

- `MaterialAnchor` — Where in a material a mark points.
- `MaterialMark` — One mark in an Asset's material.
- `TimelineSpan` — Where on a timeline a mark sits. `end_ms == None` means "an

