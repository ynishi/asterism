# teams-infra::auth::oidc

OIDC sign-in (#163): the instance is the provider's OAuth client,
and the invite still says whether.

Two halves, kept apart because they answer different questions.
[`OidcClient`] talks to the provider: discovery, the authorization
URL a browser is sent to, the code exchange, and the check of the
ID token that comes back — signature against the provider's
published keys, issuer and audience against configuration, expiry
against the clock, nonce against the attempt. [`OidcIdentities`]
talks to the database: which account a verified identity resolves
to, and the pinning that makes that answer stable.

## Why the server is the client

The desktop app never speaks to the provider. It opens a browser at
this instance, and this instance runs the whole authorization-code
exchange as a confidential client — client secret here, never on a
device; one callback URL, registered once by whoever hosts the
instance. That is the backend-for-frontend shape
draft-ietf-oauth-v2-1 §2.1 recommends for a native application
that wishes to use client credentials, and what it buys is stated
where it is used: a provider outage stops new sign-ins and nothing
else, the device listing is the instance's, and a hosted deployment
with several providers changes nothing on a member's machine.

## What a verified token is not

Proof of membership. The provider answers who; the binding row
answers whether that person holds an account here, and the roster
answers whether they belong to a team. A token that verifies and
resolves to nobody is refused with the same one-armed answer a
wrong password gets, and nothing here provisions an account from a
claim.

## Pinning

An admin binds an account to an email at the provider. The first
sign-in whose token carries that email — **verified**, or it does
not count — pins the token's `sub` to the row, and from then on the
subject is what resolves and the email is inert. A provider that
later hands the address to somebody else hands them a different
subject; an unverified email claim never matches anything. The two
account-takeover shapes the issue names are closed by those two
rules, and `sqlite::migrations::V11_OIDC_IDENTITY` is where the
indexes make them structural.

## Functions

- `normalize_email` — Lower-cased and trimmed: the one form an address is stored and
- `sha256_hex` — SHA-256 of a string, hex — how an attempt's collector is compared

## Types

- `Exchange` — What a code exchange comes to.
- `IdentityBinding` — One account's binding, as an admin reads it back.
- `OidcClient` — The provider-facing half: discovery, the authorization URL, the
- `OidcConfig` — How the instance reaches its provider — what `teams-server serve`
- `OidcIdentities` — The database half: the binding rows, and the resolve that pins.
- `VerifiedIdentity` — Who a provider vouched for, after every check passed.

