# asterism-core::application::forge::legacy_pursuit_service

`LegacyPursuitService` — the lifecycle verbs of the minted unit of work
(#29): open, close, reopen, and the reads that derive standing.

**This is on its way out, and the name says so.** The model it
serves — `PursuitEvent`, `PursuitTx`, the standing derived from
them — is superseded by [`model`](crate::domain::forge::model),
where a line's history is a chain of change points and work is a
log of passes. Nothing here is being extended.

It is still the one wired to transport, so it stays until the
service that replaces it can answer the same surface. That service
is `PursuitService`, which took this one's name: the code that is
leaving carries the awkward one, so that a reader never has to work
out which of two same-named services is current.

When the replacement is wired, this file and the model under it go
in one deletion. Until then, a change here is a change to something
scheduled for removal, and is worth questioning on those grounds
alone.

The open creates one (naming intent up front), the one-way
lifecycle facts (close / reopen) are recorded rather than written
as a status, and the ledger takes the membership gestures.
Transport routes land in the next slice of #29; until then the
service fronts the e2e surface through `CoreCtx`, the same way
every service is reachable there.

The close writes one row and materialises nothing: it records that
a line of work ended, and what the line of work was on stays
derivable from the ledger it leaves alone.

## Types

- `LegacyPursuitService` — Pursuit lifecycle use-case service.

