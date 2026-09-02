# asterism-teams-wire::dto

Response shapes of the `/teams/*` routes a member's client reads.

Mutation routes answer with the [`LedgerEventDto`] their write
appended — the same-tx rule (#83 §2) means the event *is* the
receipt, and a role change carrying old+new in its payload reads on
its own (#83 §1).

## Types

- `ContentEnteredDto` — What the team minted for content that entered it
- `DeviceTokenDto` — One device token as its owner sees it.
- `DeviceTokenMintedDto` — A freshly minted device token (`POST /teams/auth/device`, #204).
- `DeviceTokensDto` — The caller's own device tokens (`GET /teams/auth/device`, #204).
- `HeldAssetDto` — One asset a team holds, as the bulk resolve answers about it.
- `HeldContentDto` — The have-check's answer
- `LedgerEventDto` — One entry of a team's append-only stream
- `LedgerPageDto` — One page of a team's stream (`GET /teams/{team_id}/events`).
- `MyTeamDto` — One team the caller belongs to.
- `MyTeamsDto` — The teams the caller is a member of (`GET /teams`).
- `ResolvedContentDto` — The bulk resolve's answer
- `RosterDto` — The team's current membership set, and what the caller may do in
- `RosterMemberDto` — One membership row as the roster lists it.
- `SessionDto` — A freshly minted session (`POST /teams/auth/login`, and
- `SubjectRefDto` — One typed reference an event makes.
- `TeamCreatedDto` — The result of `POST /teams/create`.
- `ViewerDto` — The caller's standing in the team whose roster this is.

