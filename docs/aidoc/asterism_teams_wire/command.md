# asterism-teams-wire::command

Command shapes — inputs of the `/teams/*` routes a member's client
calls.

An owner's roster writes are here too, and that is not an exception
to the line above. They moved from `teams-contract` when an owner
gained a screen to say them from (#210), and an owner saying them
is a member's client saying them. What stayed behind stayed for the
reason the crate doc gives — no client sends it — and not for
anything about whose act it is: the substrate's own upload is a
member's act that no client happens to send.

The session token is **not** a field on any of these: it travels in
the `Authorization: Bearer` header, resolved by the server's gate
middleware before a handler sees the body (#83 §5 — every route:
session token → user_id → membership gate).

## Types

- `CollectOidcAttemptCommand` — Collects a sign-in attempt
- `CreateTeamCommand` — Creates a team (`POST /teams/create`).
- `DeviceLoginCommand` — Presents a device token to `POST /teams/auth/device/login` (#204).
- `EnterContentCommand` — Brings content into a team against open work
- `GrantOwnerCommand` — Grants the owner role (`POST /teams/{team_id}/owners/grant`, owner
- `HaveContentCommand` — Asks which digests a team already has
- `InviteMemberCommand` — Invites a user into the team (`POST /teams/{team_id}/members/invite`,
- `LoginCommand` — Presents a credential to `POST /teams/auth/login`.
- `MintDeviceTokenCommand` — Asks for a device token (`POST /teams/auth/device`, #204).
- `OidcAttemptCommand` — Starts a sign-in through the provider
- `RemoveMemberCommand` — Removes a member (`POST /teams/{team_id}/members/remove`, owner
- `RenameTeamCommand` — Renames a team (`POST /teams/{team_id}/rename`, #218) — an
- `ResolveContentCommand` — Asks what a team holds for a list of its own asset ids
- `RevokeOwnerCommand` — Revokes the owner role (`POST /teams/{team_id}/owners/revoke`,

