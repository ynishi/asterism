//! Forge use cases — the verbs of a line of work.
//!
//! [`LineService`] owns the repository a history sits on: opening one,
//! reading what is alive on it, and moving the two things about a line
//! that are not written into its history — its name and the rule it
//! settles collisions by. [`PursuitService`] owns work against a line:
//! opening it, adding a round, running the line's rule over what the
//! work collides with, and ending it. They are apart from the rest
//! because they are the only services here whose writes carry intent
//! rather than content (the layer itself is described in
//! [`domain::forge`](crate::domain::forge)).
//!
//! Neither has a transport. The first model's did, and it went with it
//! on #102; what these two are reachable through today is a caller
//! inside the process, and the adapter under them is where that
//! changes.
//!
//! Nothing in the raw layer is edited by either of them. Ending work
//! records that it ended and touches no asset: no trash, no label, no
//! rating. Integrating the conclusion back into the library is the raw
//! layer's own business, and stays on the raw layer's verbs.
//!
//! # Which way the services may point
//!
//! A rule with two halves, and the second is the one that gets broken:
//!
//! - **A service here does not name a service or a port of the raw
//!   layer.** What it needs from below, it asks through the face that
//!   answers — [`boundary`](crate::domain::forge::boundary) — so the
//!   thing it depends on is a contract rather than an implementation.
//! - **Nothing in the raw layer names a service here.** Not a type,
//!   not a port, not a function. When the raw layer needs something of
//!   the forge, the forge answers a face; the call still happens, and
//!   the dependency does not.
//!
//! Both halves say the same thing: the two sides are wired through
//! contracts in both directions, so neither can be compiled with the
//! other in front of it. Wire them directly and each needs the other
//! to build, which is the state that makes either impossible to move.
//!
//! **Shared identity is not a crossing.** `PersonaId`, `AssetId`, the
//! attribution triple and `DomainError` are named directly here, as
//! they are everywhere else, because they belong to neither side. The
//! rule is about capability, never about what something is.

pub mod line_service;
pub mod pursuit_service;
pub mod thread_service;

pub use line_service::LineService;
pub use pursuit_service::PursuitService;
pub use thread_service::{Anchored, ThreadService};
