# asterism-core::application::forge::pursuit_service

`PursuitService` — the lifecycle verbs of the minted unit of work
(#29): open, close, reopen, restamp, and the reads that derive
standing.

Always-mint lives in
[`DispatchService`](super::dispatch_service::DispatchService) —
a dispatch arriving unstamped mints its own pursuit there. This
service is everything else: the explicit pre-create (naming intent
up front), the one-way lifecycle facts (close / reopen — recorded,
never a status write), the close's single deliberate
materialisation (the kept set frozen into a snapshot), and the
restamp repair verb. Transport routes land in the next slice of
#29; until then the service fronts the e2e surface through
`CoreCtx`, the same way every service is reachable there.

# The close freeze is a forge-side calling convention

The core treats snapshot member order as part of snapshot identity
(caller order, nothing sorts). The close path sorts the kept set
ascending *itself* before freezing, so identical kept sets dedupe
across closes; a close snapshot consequently does not dedupe with a
pick-ordered input snapshot over the same members — correct, they
are different statements.

## Types

- `PursuitService` — Pursuit lifecycle use-case service.

