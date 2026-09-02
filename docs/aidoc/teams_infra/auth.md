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
are today.

[`oidc`] (#163) is a way in that sits **beside** the port rather
than behind it: the port's one method takes a login and a secret,
and what a provider hands back is neither — a signed statement
about who somebody is, which the instance exchanges for itself as
the provider's OAuth client. What every way in shares is everything
after a `user_id` is known: the same session, the same device
token, the same gate. 2FA is deferred by design.

