# teams-contract::dto

Response DTOs of the `/teams/*` surfaces a member's client does not
speak.

Mutation routes answer with the [`LedgerEventDto`] their write
appended — the same-tx rule (#83 §2) means the event *is* the
receipt, and a role change carrying old+new in its payload reads on
its own (#83 §1). That envelope now lives in `asterism-teams-wire`, along
with the session, the roster, the ledger page and the content
verbs' answers: a member's client reads all of those and may not
link this crate (#148 decision 15). It is named here rather than
re-spelled, which is the whole point of a leaf both planes depend
on.

## Types

- `BlobUploadedDto` — The result of `PUT /teams/{team_id}/blobs?digest=…`.
- `HeadPublishedDto` — The receipt of `PUT /teams/heads/registry` (#132 phase 3) — the
- `MarkedBlobLinkDto` — One marked link — everything unmark (or a decision to let reclaim
- `MarkedBlobsDto` — The team's marked-for-purge set
- `PurgeReclaimedDto` — The result of `POST /teams/{team_id}/blobs/purge/reclaim` (#95).

