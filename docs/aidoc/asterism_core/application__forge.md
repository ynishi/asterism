# asterism-core::application::forge

Forge use cases — the verbs of a line of work.

[`pursuit_service`] owns the lifecycle (open / close / reopen /
restamp) and the reads over it; [`project_service`] opens the context
those pursuits file under; [`dispatch_service`] starts a round and
files it under the pursuit it belongs to, minting one when the caller
named none.

The round itself is not a forge thing — [`dispatch`](crate::domain::dispatch)
is a core module, because an exporter invocation is a call that was
made rather than an account of why. What is a forge thing is the
*filing*: choosing which pursuit a round belongs to, and minting one
when the caller named none (doctrine 5). That rule binds here, at the
application layer, on a forge verb — `DispatchJob::new` leaves
`pursuit_id` unset, and the domain type is complete without it. So
this service sits under `forge/` for the half of its job that is
filing, and the boundary it straddles is the one #81 is about.

Nothing in the catalogue is edited by either of them. Closing a
pursuit freezes what was kept and touches no asset: no trash, no
label, no rating. Integrating the conclusion back into the library is
the catalogue's own business, and stays on the catalogue's verbs.

