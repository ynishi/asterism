# teams-contract::dto

Response DTOs of the `/teams/*` surface.

Mutation routes answer with the [`LedgerEventDto`] their write
appended — the same-tx rule (#83 §2) means the event *is* the
receipt, and a role change carrying old+new in its payload reads on
its own (#83 §1).

## Types

- `BlobUploadedDto` — The result of `PUT /teams/{team_id}/blobs?digest=…`.
- `LedgerEventDto` — One entry of a team's append-only stream
- `MarkedBlobLinkDto` — One marked link — everything unmark (or a decision to let reclaim
- `MarkedBlobsDto` — The team's marked-for-purge set
- `ModelRegistryPublishedDto` — The receipt of `PUT /teams/models/registry` (#126) — the envelope
- `PurgeReclaimedDto` — The result of `POST /teams/{team_id}/blobs/purge/reclaim` (#95).
- `RosterDto` — The team's current membership set
- `RosterMemberDto` — One membership row as the roster lists it.
- `SessionDto` — A freshly minted session (`POST /teams/auth/login`).
- `SubjectRefDto` — One typed reference an event makes.
- `TeamCreatedDto` — The result of `POST /teams/create`.

