# asterism-teams-wire::command

Command shapes — inputs of the `/teams/*` routes a member's client
calls.

An owner's roster writes are here too, and that is not an exception
to the line above. They moved from `teams-contract` when an owner
gained a screen to say them from (#210), and an owner saying them
is a member's client saying them. The dividing line stays *who says
it*: what an operator or an admin says from outside a team — the
substrate's own upload, the purge, the head registry — stays where
it was, because no client speaks it.

The session token is **not** a field on any of these: it travels in
the `Authorization: Bearer` header, resolved by the server's gate
middleware before a handler sees the body (#83 §5 — every route:
session token → user_id → membership gate).

## Types

- `CreateTeamCommand` — Creates a team (`POST /teams/create`).
- `DeviceLoginCommand` — Presents a device token to `POST /teams/auth/device/login` (#204).
- `EnterContentCommand` — Brings content into a team against open work
- `GrantOwnerCommand` — Grants the owner role (`POST /teams/{team_id}/owners/grant`, owner
- `HaveContentCommand` — Asks which digests a team already has
- `InviteMemberCommand` — Invites a user into the team (`POST /teams/{team_id}/members/invite`,
- `LoginCommand` — Presents a credential to `POST /teams/auth/login`.
- `MintDeviceTokenCommand` — Asks for a device token (`POST /teams/auth/device`, #204).
- `RemoveMemberCommand` — Removes a member (`POST /teams/{team_id}/members/remove`, owner
- `ResolveContentCommand` — Asks what a team holds for a list of its own asset ids
- `RevokeOwnerCommand` — Revokes the owner role (`POST /teams/{team_id}/owners/revoke`,

