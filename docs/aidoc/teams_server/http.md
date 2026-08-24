# teams-server::http

HTTP transport — the axum `/teams/*` router (#83 §5, the #91
slice).

## Route table

| Method | Path | Authority |
|---|---|---|
| POST | `/teams/auth/login` | none (rate-limited) |
| POST | `/teams/auth/logout` | bearer token (rate-limited) |
| POST | `/teams/create` | any authenticated user; operator-only under closed registration |
| POST | `/teams/{team_id}/delete` | owner, or the operator (operator-stamped) |
| GET | `/teams/{team_id}/roster` | member, or the operator |
| GET | `/teams/{team_id}/events` | member, or the operator |
| POST | `/teams/{team_id}/members/invite` | owner |
| POST | `/teams/{team_id}/members/remove` | owner |
| POST | `/teams/{team_id}/owners/grant` | owner |
| POST | `/teams/{team_id}/owners/revoke` | owner |
| PUT | `/teams/{team_id}/blobs?digest=…` | member (a roster row; the operator has no implicit upload) |
| GET | `/teams/{team_id}/blobs/{digest}` | member, or the operator — every failure is the same `404`, see below |
| POST | `/teams/{team_id}/blobs/{digest}/purge/mark` | owner, or the operator (operator-stamped — #95, the §1 delete row's reclaim sibling) |
| POST | `/teams/{team_id}/blobs/{digest}/purge/unmark` | owner, or the operator (operator-stamped) |
| POST | `/teams/{team_id}/blobs/purge/reclaim` | owner, or the operator (operator-stamped); refused while every mark is inside its grace window |
| GET | `/teams/{team_id}/blobs/purge/marked` | owner, or the operator — the marked set, same authority as the mark |
| PUT | `/teams/heads/registry` | the operator only — instance scope (#132), no team gate |
| GET | `/teams/heads/registry` | any authenticated account — the live head artifact's bytes, verbatim |

## The gate (#83 §5: every route, no exceptions)

Two middleware layers, in request order:

1. [`auth_gate`] — `Authorization: Bearer` token →
   [`PasswordAuth::resolve_session`] → [`AccountRecord`], inserted
   as an extension. Missing, malformed, unknown and **expired**
   tokens are all the same `401` (an expired row is deleted on
   touch).
2. [`team_gate`] (team-scoped routes only) — the `{team_id}` path
   segment → team existence (`404`) → the caller's current role in
   *this* team, read from state, never from the ledger (#83 §1).
   A caller with neither a membership row nor the operator flag is
   `403` before any handler runs.

The handler then asks `teams-core`'s decision functions
([`verb_allowed`] / [`may_create_team`]) with the capacity the gate
established. When both capacities could act, the membership row
wins and the ledger stamp is the member's — the operator variant
is reserved for the operator acting *from outside* the membership
set, which is exactly when §1 demands the stamp say so.

## The blob read is the one deliberate exception to [`team_gate`]

`GET /teams/{team_id}/blobs/{digest}` sits behind [`auth_gate`]
only, and answers **one indistinguishable `404`** for every miss:
unknown team, caller neither a member nor the operator, digest
never uploaded, digest linked only in a team the caller cannot
read — and, since #95, a link **marked for purge**, whose grace
window hides it behind the very same answer. The gate's
usual 403/404 split would confirm which part of the probe was
right; on the byte-serving surface that is exactly the existence
oracle the link boundary exists to close (#83 §3 — a digest
"exists" for a caller iff a link row sits in a team they belong
to), the same conflation `asterism-server`'s asset-file route
documents. Uploads stay behind the full gate: mutations answer
403 to outsiders on every other route, and a 403 on `PUT` reveals
nothing about any digest.

## Error mapping

Same body shape as `asterism-server` (`{"kind", "message"}`).
Domain refusals surface as client errors, never `500`:
`Validation` → 400, [`DomainError::LastOwner`] and
`DigestMismatch` → 409 (the mismatch body carries declared and
computed, both), `Infra` → 500; the gate adds 401/403/404 and the
limiter 429.

## Functions

- `router` — Builds the router; the caller binds a listener and calls

