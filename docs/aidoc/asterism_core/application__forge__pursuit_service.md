# asterism-core::application::forge::pursuit_service

`PursuitService` — the lifecycle verbs of the minted unit of work
(#29): open, close, reopen, and the reads that derive standing.

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

- `PursuitService` — Pursuit lifecycle use-case service.

