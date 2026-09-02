# teams-contract::command

Command DTOs — inputs of the state-changing `/teams/*` routes no
client sends.

What a **member's client** sends is in `asterism-teams-wire`, which
the client may link and this crate it may not (#148 decision 15):
login, team creation, the three content verbs, and — since #210
gave an owner a screen to say them from — the roster writes.

What stays is the substrate's own upload, and the line it is on is
who sends it rather than whose act it is: uploading into a team's
store is a member's act, and the route refuses an admin's implicit
one. No client sends it because content reaches a team through the
promotion path instead.

The session token is **not** a field on any of these: it travels in
the `Authorization: Bearer` header, resolved by the server's gate
middleware before a handler sees the body (#83 §5 — every route:
session token → user_id → membership gate).

## Types

- `UploadBlobCommand` — Uploads a blob into the team's store

