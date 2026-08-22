//! Forge use cases — the verbs of a line of work.
//!
//! [`LineService`] owns the repository a history sits on: opening one,
//! reading what is alive on it, moving the things about a line that
//! are not written into its history — its name, the rule it settles
//! collisions by, and whether it is finished with — and dropping it. [`PursuitService`] owns work against a line:
//! opening it, adding a round, running the line's rule over what the
//! work collides with, and ending it. [`ThreadService`] owns what is
//! said about either: a conversation hangs off a pursuit, a round, an
//! entry as one round had it, or a change point, and touches neither
//! log. The three are apart from the rest because they are the only
//! services here whose writes carry intent rather than content (the
//! layer itself is described in
//! [`domain::forge`](crate::domain::forge)).
//!
//! Nothing in the raw layer is edited by any of them. Ending work
//! records that it ended and touches no asset: no trash, no label, no
//! rating. Integrating the conclusion back into the library is the raw
//! layer's own business, and stays on the raw layer's verbs.
//!
//! # Which way the services may point
//!
//! **A service here does not name a service or a port of the raw
//! layer.** What it needs from below, it asks through the face that
//! answers — [`boundary`](crate::domain::forge::boundary) — so the
//! thing it depends on is a contract rather than an implementation.
//! Wire the two directly and each needs the other to build, which is
//! the state that makes either impossible to move. What the raw layer
//! may name is its own side of that boundary, and
//! [`domain::forge`](crate::domain::forge) states both directions.
//!
//! **Shared identity is not a crossing.** `AssetId`, `define_uuid_id`,
//! the attribution triple and `DomainError` belong to neither side, so
//! naming one is not a crossing wherever it happens. The rule is about
//! capability, never about what something is. `PersonaId` was in that
//! list until the forge stopped asking whose an asset is.

pub mod line_service;
pub mod pursuit_service;
pub mod thread_service;

pub use line_service::LineService;
pub use pursuit_service::PursuitService;
pub use thread_service::{Anchored, ThreadService};
