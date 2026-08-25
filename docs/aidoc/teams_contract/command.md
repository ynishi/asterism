# teams-contract::command

Command DTOs — inputs of the state-changing `/teams/*` routes.

The session token is **not** a field on any of these: it travels in
the `Authorization: Bearer` header, resolved by the server's gate
middleware before a handler sees the body (#83 §5 — every route:
session token → user_id → membership gate).

## Types

- `CreateTeamCommand` — Creates a team (`POST /teams/create`).
- `EnterContentCommand` — Brings content into a team against open work
- `GrantOwnerCommand` — Grants the owner role (`POST /teams/{team_id}/owners/grant`, owner
- `HaveContentCommand` — Asks which digests a team already has
- `InviteMemberCommand` — Invites a user into the team (`POST /teams/{team_id}/members/invite`,
- `LoginCommand` — Presents a credential to `POST /teams/auth/login`.
- `RemoveMemberCommand` — Removes a member (`POST /teams/{team_id}/members/remove`, owner
- `ResolveContentCommand` — Asks what a team holds for a list of its own asset ids
- `RevokeOwnerCommand` — Revokes the owner role (`POST /teams/{team_id}/owners/revoke`,
- `UploadBlobCommand` — Uploads a blob into the team's store

