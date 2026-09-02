# teams-core::port::auth

`port::auth` — credential verification behind a provider swap
(#83 §1).

Credentials live behind this port so the v0 choice (instance-local
argon2id password adapter, #83 §5) stays an adapter detail. It was
cut expecting an OIDC adapter to be a second implementation; when
that sign-in arrived (#163) it sat beside the port instead, because
what a provider hands back is not a login and a secret —
`teams-infra`'s auth module says where the two paths meet. Sessions
are deliberately **not** here: a session is a short-lived infra
artifact that resolves to a `user_id` and the ledger never sees one
— `teams-infra` owns that table and its expiry.

## Traits

- `CredentialVerifier` — Verifies a presented credential and resolves it to a user.

