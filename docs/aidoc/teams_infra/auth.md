# teams-infra::auth

Auth adapters for the teams plane (#83 §5).

v0 is the instance-local password adapter ([`password`]): argon2id
hashing behind `teams-core`'s
[`CredentialVerifier`](teams_core::port::auth::CredentialVerifier)
port, and beneath it the DB-backed opaque stores the port knows
nothing about. That split is the rule rather than a list — what a
verifier answers is the domain's question, and what resolves to a
`user_id` afterwards is a short-lived infra artifact the ledger
never sees (#83 §1). The module's own doc says which stores those
are today. OIDC is a later adapter behind the same port; 2FA is
deferred by design.

