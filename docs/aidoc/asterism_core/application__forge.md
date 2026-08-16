# asterism-core::application::forge

Forge use cases — the verbs of a line of work.

[`pursuit_service`] owns the lifecycle (open / close / reopen /
restamp) and the reads over it; [`dispatch_service`] starts a round
and files it under the pursuit it belongs to, minting one when the
caller named none. They are together because they are the two halves
of one story — a pursuit with no round records nothing, a round with
no pursuit cannot exist (doctrine 5) — and apart from the rest
because they are the only services here whose writes carry intent
rather than content (doctrine 6; the layer itself is described in
[`domain::forge`](crate::domain::forge)).

Nothing in the catalogue is edited by either of them. Closing a
pursuit freezes what was kept and touches no asset: no trash, no
label, no rating. Integrating the conclusion back into the library is
the catalogue's own business, and stays on the catalogue's verbs.

