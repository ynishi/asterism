# teams-contract::dto

Response DTOs of the `/teams/*` surface.

Mutation routes answer with the [`LedgerEventDto`] their write
appended — the same-tx rule (#83 §2) means the event *is* the
receipt, and a role change carrying old+new in its payload reads on
its own (#83 §1).

## Types

- `BlobUploadedDto` — The result of `PUT /teams/{team_id}/blobs?digest=…`.
- `ContentEnteredDto` — What the team minted for content that entered it
- `HeadPublishedDto` — The receipt of `PUT /teams/heads/registry` (#132 phase 3) — the
- `HeldAssetDto` — One asset a team holds, as the bulk resolve answers about it.
- `HeldContentDto` — The have-check's answer
- `LedgerEventDto` — One entry of a team's append-only stream
- `LedgerPageDto` — One page of a team's stream (`GET /teams/{team_id}/events`).
- `MarkedBlobLinkDto` — One marked link — everything unmark (or a decision to let reclaim
- `MarkedBlobsDto` — The team's marked-for-purge set
- `PurgeReclaimedDto` — The result of `POST /teams/{team_id}/blobs/purge/reclaim` (#95).
- `ResolvedContentDto` — The bulk resolve's answer
- `RosterDto` — The team's current membership set
- `RosterMemberDto` — One membership row as the roster lists it.
- `SessionDto` — A freshly minted session (`POST /teams/auth/login`).
- `SubjectRefDto` — One typed reference an event makes.
- `TeamCreatedDto` — The result of `POST /teams/create`.

