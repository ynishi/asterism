//! Forge use cases — the verbs of a line of work.
//!
//! [`legacy_pursuit_service`] owns the lifecycle (open / close / reopen) and
//! the reads over it; [`project_service`] owns the filing it hangs
//! under. They are apart from the rest because they are the only
//! services here whose writes carry intent rather than content (the
//! layer itself is described in
//! [`domain::forge`](crate::domain::forge)).
//!
//! # Which model a service here serves
//!
//! Two live side by side while one replaces the other, so this is
//! worth being able to tell at a glance rather than by reading:
//!
//! ```text
//!   legacy_pursuit_service   PursuitEvent / PursuitTx / standing
//!                            wired to transport, extended by nobody,
//!                            deleted when its replacement is wired
//!
//!   (arriving)               domain::forge::model — a line's history
//!                            as a chain, work as a log of passes
//! ```
//!
//! The naming is deliberate: what is leaving carries the qualified
//! name, and the plain one belongs to what stays. A file named for the
//! current model, serving the old one, is the thing this avoids.
//!
//! Nothing in the raw layer is edited by either of them. Closing a
//! pursuit records that a line of work ended and touches no asset: no
//! trash, no label, no rating. Integrating the conclusion back into the
//! library is the raw layer's own business, and stays on the raw
//! layer's verbs.
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

pub use line_service::LineService;
pub use pursuit_service::PursuitService;
