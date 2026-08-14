# asterism-core::application::material_mark_service

`MaterialMarkService` — the marks placed into an Asset's material.

Verbs:
- [`post`](MaterialMarkService::post) — places a mark at a position
  in the material (User or Persona author).
- [`list_by_asset`](MaterialMarkService::list_by_asset) — reads the
  marks in the material's own order, which is not the order they
  were placed in.
- [`edit`](MaterialMarkService::edit) — rewrites the body of an
  existing mark (stamps `edited_at`).
- [`delete`](MaterialMarkService::delete) — removes one row.

Moving a mark is not among them. Rewording a note and repositioning
it are different acts, and no surface asks for the second one yet;
adding it later is a verb here, not a change to any of these.

Every write takes an [`AttributionContext`] it does not persist, for
the same reason
[`AssetCommentService`](crate::application::asset_comment_service)
does: a mark records a
[`CommentAuthor`](crate::domain::asset_comment::CommentAuthor), whose
`Persona` variant is a register (a voice) rather than a writer — so
the row states who is speaking without stating who is accountable.

## Types

- `MaterialMarkService` — Application-layer surface for [`MaterialMark`].

