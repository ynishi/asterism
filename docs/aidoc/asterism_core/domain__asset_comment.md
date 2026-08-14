# asterism-core::domain::asset_comment

`AssetComment` — a thread of short notes attached to an Asset.

Comments are Asset-first: an Asset is the aggregate root, comments
are child entries the User or a Persona can post. The intent is
deliberately smaller than a full chat surface — one Asset, a
chronological list of short bodies, each carrying an author kind
and (for Persona authors) a persona reference.

Design notes:

- **Two author kinds**: `User` (the human running Asterism) and
  `Persona` (an AI persona registered in the vault). We do not
  model a full identity table for the User side — the vault is
  single-user by design; every `User` post is "me". A Persona
  comment carries the persona id so downstream UI can render the
  author avatar and colour without a follow-up lookup.
- **Body is free-form text** (no markdown-only guard). The UI
  renders it as plain text with newlines preserved; a future
  markdown pass can layer on top without a schema change.
- **`edited_at`** is `None` for the original post and stamped on
  every `edit` — the UI reveals a "(edited)" marker when it's set
  without exposing the raw diff.

## Types

- `AssetComment` — A single comment on an Asset.
- `CommentAuthor` — Who wrote a comment.

