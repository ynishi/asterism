# teams-core 0.0.0

# teams-core — domain layer of the Asterism teams plane

First slice of #83: the types and invariants of the hosted Team
plane, with no IO anywhere in the crate. What a Team *is* — who may
act on it, what the ledger records, what the store promises — is
decided here; SQLite, the filesystem, HTTP and auth adapters arrive
in the follow-up slices (`teams-infra` / `teams-server`).

## Layout

- `domain::identity` — [`User`](domain::identity::User) /
  [`Membership`](domain::identity::Membership) /
  [`InstanceOperator`](domain::identity::InstanceOperator) /
  [`ActorStamp`](domain::identity::ActorStamp), the last-owner rule,
  and the #83 §1 authority table as decision functions.
- `domain::ledger` — the append-only
  [`LedgerEvent`](domain::ledger::LedgerEvent) envelope and the v0
  kind registry. The payload is opaque to the substrate.
- `domain::store` — [`TeamBlobLink`](domain::store::TeamBlobLink) /
  [`Locator`](domain::store::Locator) and the declared-digest
  verification rule (accept or reject the whole op; no third
  outcome).
- `port` — the traits `teams-infra` implements: blob storage,
  credential verification, and the share port reserved for #63.
- `error` — [`DomainError`], the crate's `thiserror` enum.

## Dependency rule

This crate depends on `asterism-core` and on no other asterism-*
crate (#83 §4). What it takes from there is vocabulary — the
`sha256:`-prefixed digest notation and its parser — reused as-is so
the teams plane and the local app spell a byte fingerprint one way.

## Modules

- [`domain`](domain.md): Domain types and invariants of the teams plane — everything here is
- [`domain::head_registry`](domain__head_registry.md): `head_registry` — the instance's carriage of a trained tag head
- [`domain::identity`](domain__identity.md): `identity` — who exists, who belongs to a team, and who may do what.
- [`domain::ledger`](domain__ledger.md): `ledger` — the actor-stamped, append-only event envelope (#83 §2).
- [`domain::store`](domain__store.md): `store` — the team-side view of instance-owned blobs, and the
- [`error`](error.md): `DomainError` — the innermost error type of the teams plane.
- [`port`](port.md): Ports — the traits `teams-infra` implements (dependency inversion,
- [`port::auth`](port__auth.md): `port::auth` — credential verification behind a provider swap
- [`port::blob`](port__blob.md): `port::blob` — backing storage for the instance's global CAS.
- [`port::share`](port__share.md): `port::share` — reserved for the share domain.

