# teams-contract::command

Command DTOs — inputs of the state-changing `/teams/*` routes an
owner, an admin or an operator's tooling calls.

What a **member's client** sends moved to `asterism-teams-wire` when the
leaf landed (#148 decision 15): login, team creation and the three
content verbs live there now, because the client may not link this
crate. What stayed is what is not a member's vocabulary — the
roster verbs, which are an owner's, and the substrate's own upload.

The session token is **not** a field on any of these: it travels in
the `Authorization: Bearer` header, resolved by the server's gate
middleware before a handler sees the body (#83 §5 — every route:
session token → user_id → membership gate).

## Types

- `GrantOwnerCommand` — Grants the owner role (`POST /teams/{team_id}/owners/grant`, owner
- `InviteMemberCommand` — Invites a user into the team (`POST /teams/{team_id}/members/invite`,
- `RemoveMemberCommand` — Removes a member (`POST /teams/{team_id}/members/remove`, owner
- `RevokeOwnerCommand` — Revokes the owner role (`POST /teams/{team_id}/owners/revoke`,
- `UploadBlobCommand` — Uploads a blob into the team's store

