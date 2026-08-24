# teams-infra::auth::password

`auth::password` — the v0 instance-local credential adapter
(#83 §5).

Three decisions carry this module:

- **Hashing is argon2id through RustCrypto's `argon2`** (OWASP's
  current first choice), default parameters, PHC-string storage —
  never a hand-rolled construction. Verification runs *inside* the
  SQLite isle closure: the isle's dedicated thread keeps the key
  derivation off the async executor without a second thread pool,
  at the cost of serialising logins behind one another and behind
  other DB work — an accepted trade at this plane's team-scale,
  single-server topology (#83 §4).
- **Sessions are DB-backed opaque tokens** (#83 §1: a session is a
  short-lived infra artifact the ledger never sees). The client
  holds 32 CSPRNG bytes hex-encoded; the table holds the SHA-256 of
  that, so a leaked database never contains a usable bearer token.
  Expiry is enforced twice: [`PasswordAuth::resolve_session`]
  rejects **and deletes** an expired row on touch, and
  [`PasswordAuth::cleanup_expired`] sweeps in bulk (the server runs
  it on every login, so the table cannot accumulate dead rows
  faster than logins happen).
- **No default credentials exist** ([`reject_default_credential`]):
  the bootstrap admin (#83 §5, the §1
  [`InstanceAdmin`](teams_core::domain::identity::InstanceAdmin)) is
  created only from operator-supplied values, and a blank, too-short,
  login-equal, or well-known placeholder password is *refused* at
  creation time rather than warned about.

The port implementation ([`CredentialVerifier`]) keeps the port's
one-arm contract: a wrong password and an unknown login are the
same `Ok(None)`, and the unknown-login path verifies against a
process-local dummy hash so the two answers cost the same work
(username-enumeration resistance on the timing side too).

## Functions

- `reject_default_credential` — Refuses the credentials the "no fixed defaults" rule (#83 §5)

## Types

- `AccountRecord` — One credential-store row, as the server's gate consumes it: who the
- `PasswordAuth` — The v0 password + session adapter over the teams database.

