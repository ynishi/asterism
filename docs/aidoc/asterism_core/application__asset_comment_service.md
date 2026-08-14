# asterism-core::application::asset_comment_service

`AssetCommentService` — thread lifecycle on an Asset.

Verbs:
- [`post`](AssetCommentService::post) — appends a new comment (User
  or Persona author).
- [`list`](AssetCommentService::list) — reads the thread in
  chronological order.
- [`edit`](AssetCommentService::edit) — rewrites the body of an
  existing comment (stamps `edited_at`).
- [`delete`](AssetCommentService::delete) — removes one row.

MVP scope: no reactions, no threading (flat list), no @mention
parsing. The thread is a single flat stream — sufficient for
Asset-focused annotation.

Every write here takes an [`AttributionContext`] it does not persist.
A comment records a
[`CommentAuthor`](crate::domain::asset_comment::CommentAuthor)
instead, whose `User` variant is the same "me" the attribution
`Owner` names and whose `Persona` variant is a register (a voice),
not a writer — so the row states who is speaking without stating who
is accountable. Closing that gap is a later wave;
this argument does not close it.

## Types

- `AssetCommentService` — Application-layer surface for `AssetComment`.

