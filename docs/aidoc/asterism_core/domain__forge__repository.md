# asterism-core::domain::forge::repository

The forge's persistence ports.

Split from [`domain::repository`](crate::domain::repository), which
held these two beside the raw layer's twenty-eight and so made the
forge's storage contract part of the file a new raw-layer port is
added to. Nothing about the traits changed in the move.

The raw layer does not name these, and needs nothing of a pursuit.

## Traits

- `ProjectRepository` — Persistence port for the forge's project and its lines (#63
- `PursuitRepository` — Persistence port for the pursuit family (#29): the minted unit of

