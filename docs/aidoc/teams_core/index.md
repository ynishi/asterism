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
  [`InstanceAdmin`](domain::identity::InstanceAdmin) /
  [`ActorStamp`](domain::identity::ActorStamp), the last-owner rule,
  and the #83 §1 authority table as decision functions.
- `domain::ledger` — the append-only
  [`LedgerEvent`](domain::ledger::LedgerEvent) envelope and the v0
  kind registry. The payload is opaque to the substrate.
- `domain::store` — [`TeamBlobLink`](domain::store::TeamBlobLink) /
  [`Locator`](domain::store::Locator) and the declared-digest
  verification rule (accept or reject the whole op; no third
  outcome).
- `port` — the traits `teams-infra` implements: blob storage and
  credential verification.
- `error` — [`DomainError`], the crate's `thiserror` enum.

## Dependency rule

What this crate takes from the local app is vocabulary — the
`sha256:`-prefixed digest notation and its parser from
`asterism-core`, reused as-is so the teams plane and the local app
spell a byte fingerprint one way.

Which `asterism-*` edges may be declared at all is stated once,
beside the dependency itself in `Cargo.toml` (#83 §4): the
never-list, what is deliberately not on it, and which direction the
licence boundary guards. What the rule comes to *here* is that the
types below are spelled in the teams plane's own words — no
invariant in this crate is stated in a shape the desktop app owns.

## Modules

- [`domain`](domain.md): Domain types and invariants of the teams plane — everything here is
- [`domain::head_registry`](domain__head_registry.md): `head_registry` — the instance's carriage of a trained tag head
- [`domain::identity`](domain__identity.md): `identity` — who exists, who belongs to a team, and who may do what.
- [`domain::ledger`](domain__ledger.md): `ledger` — the actor-stamped, append-only event envelope (#83 §2).
- [`domain::projection`](domain__projection.md): The captured projection — descriptive metadata, keyed by entry and
- [`domain::store`](domain__store.md): `store` — the team-side view of instance-owned blobs, and the
- [`error`](error.md): `DomainError` — the innermost error type of the teams plane.
- [`port`](port.md): Ports — the traits `teams-infra` implements (dependency inversion,
- [`port::auth`](port__auth.md): `port::auth` — credential verification behind a provider swap
- [`port::blob`](port__blob.md): `port::blob` — backing storage for the instance's global CAS.

