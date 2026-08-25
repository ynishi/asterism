# asterism-teams-wire::command

Command shapes — inputs of the `/teams/*` routes a member's client
calls.

The session token is **not** a field on any of these: it travels in
the `Authorization: Bearer` header, resolved by the server's gate
middleware before a handler sees the body (#83 §5 — every route:
session token → user_id → membership gate).

## Types

- `CreateTeamCommand` — Creates a team (`POST /teams/create`).
- `EnterContentCommand` — Brings content into a team against open work
- `HaveContentCommand` — Asks which digests a team already has
- `LoginCommand` — Presents a credential to `POST /teams/auth/login`.
- `ResolveContentCommand` — Asks what a team holds for a list of its own asset ids

