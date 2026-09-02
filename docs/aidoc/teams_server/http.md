# teams-server::http

HTTP transport — the axum `/teams/*` router (#83 §5, the #91
slice).

## Route table

| Method | Path | Authority |
|---|---|---|
| POST | `/teams/auth/login` | none (rate-limited) |
| POST | `/teams/auth/logout` | bearer token (rate-limited) |
| POST | `/teams/auth/device/login` | a device token (rate-limited) — answers with an ordinary session |
| GET | `/teams/auth/providers` | none — what this instance offers besides a password (#163) |
| POST | `/teams/auth/oidc/attempts` | none (rate-limited) — starts a sign-in through the provider (#163) |
| GET | `/teams/auth/oidc/attempts/{id}` | none — the page a browser lands on, HTML |
| POST | `/teams/auth/oidc/attempts/{id}/authorize` | none — the button, `303` to the provider |
| GET | `/teams/auth/oidc/callback` | a code from the provider (rate-limited) — `303` to the app's loopback listener |
| GET | `/teams/auth/oidc/attempts/{id}/done` | none — the page the listener sends the browser on to, HTML |
| POST | `/teams/auth/oidc/attempts/{id}/collect` | the attempt's secret and the grant the browser delivered (rate-limited) — answers with an ordinary session, once |
| POST | `/teams/auth/device` | any live session — mints a device token (#204) |
| GET | `/teams/auth/device` | any live session — the caller's own tokens, never their values |
| DELETE | `/teams/auth/device/{id}` | any live session — owner-scoped, `204` |
| POST | `/teams/create` | any authenticated user; admin-only under closed registration |
| POST | `/teams/{team_id}/delete` | owner, or an admin (admin-stamped) |
| GET | `/teams/{team_id}/roster` | member, or an admin |
| GET | `/teams/{team_id}/events` | member, or an admin — paged, see [`events`] |
| GET | `/teams/{team_id}/events/subject` | member, or an admin — one subject's events, same page contract |
| — | `/teams/{team_id}/forge/*` | member for every write but one; see the `forge` module |
| POST | `/teams/{team_id}/members/invite` | owner |
| POST | `/teams/{team_id}/members/remove` | owner |
| POST | `/teams/{team_id}/members/leave` | any caller holding a row, of themself |
| POST | `/teams/{team_id}/owners/grant` | owner |
| POST | `/teams/{team_id}/owners/revoke` | owner |
| PUT | `/teams/{team_id}/blobs?digest=…` | member (a roster row; an admin has no implicit upload) |
| GET | `/teams/{team_id}/blobs/{digest}` | member, or an admin — every failure is the same `404`, see below |
| POST | `/teams/{team_id}/blobs/{digest}/purge/mark` | owner, or an admin (admin-stamped — #95, the §1 delete row's reclaim sibling) |
| POST | `/teams/{team_id}/blobs/{digest}/purge/unmark` | owner, or an admin (admin-stamped) |
| POST | `/teams/{team_id}/blobs/purge/reclaim` | owner, or an admin (admin-stamped); refused while every mark is inside its grace window |
| GET | `/teams/{team_id}/blobs/purge/marked` | owner, or an admin — the marked set, same authority as the mark |
| PUT | `/teams/heads/registry` | admins only — instance scope (#132), no team gate |
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
   A caller with neither a membership row nor the admin flag is
   `403` before any handler runs.

The handler then asks `teams-core`'s decision functions
([`verb_allowed`] / [`may_create_team`]) with the capacity the gate
established. When both capacities could act, the membership row
wins and the ledger stamp is the member's — the admin variant is
reserved for an admin acting *from outside* the membership set,
which is exactly when §1 demands the stamp say so.

## Which of the device-token routes the limiter covers (#204)

One of the four, and the split is the decision. #83 §5 puts new
auth routes in the limited router, and what that limiter is for is
an unauthenticated caller presenting a *credential*: its budget is
what bounds guessing. `POST /teams/auth/device/login` is exactly
that — a token arrives from nobody in particular and either
resolves or does not — so it sits beside the password arm and
shares its bucket.

The mint, the listing and the revoke present no credential; they
present a session [`auth_gate`] has already resolved. Putting them
under the same bucket would spend a login's budget on a caller who
is already inside, so a person who minted a token would find
themselves unable to log in again — while protecting a guessing
surface that does not exist, because there is nothing to guess past
a session that already resolved. They sit behind the gate instead.

## Minting asks for a live session and nothing more (#204)

Not the password arm specifically, and this is the other question
#204 leaves open. Any-session is what makes the provider path
(#163) free: a sign-in through the provider ends in a session the
same way a password does, and the minting path never learns which
way in was taken — which is the property the whole issue turns on.
Requiring a re-auth would put a password back in front of a flow
whose point is that a password is not always what happened.

What that costs is written down rather than waved at: a stolen live
session can mint a device token, which outlives the session by
design. The bound on it is that the owner can see every token
(`GET`) and revoke any of it (`DELETE`), and that the tokens the
disk holds end on a day fixed at the mint and earlier when unused
([`TeamsCtx::device_token_ttl_ms`] and
[`TeamsCtx::device_token_idle_ms`], #163). A re-auth requirement
can be added later without moving the table or changing a single
row shape.

## The blob read is the one deliberate exception to [`team_gate`]

`GET /teams/{team_id}/blobs/{digest}` sits behind [`auth_gate`]
only, and answers **one indistinguishable `404`** for every miss:
unknown team, caller neither a member nor an admin, digest
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

The forge's routes answer on a second table, because their
refusals come from the other plane's `DomainError` and carry a
field this one has no column for: `reason`, on a conflict, which
is what tells a caller whether retrying is worth anything. Those
bodies are `asterism-server`'s to the letter — see `ApiError::Forge`
and `forge_response` below.

## Functions

- `router` — Builds the router; the caller binds a listener and calls

