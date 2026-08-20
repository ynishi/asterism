//! Forge use cases — the verbs of a line of work.
//!
//! [`pursuit_service`] owns the lifecycle (open / close / reopen) and
//! the reads over it; [`project_service`] owns the filing it hangs
//! under. They are apart from the rest because they are the only
//! services here whose writes carry intent rather than content (the
//! layer itself is described in
//! [`domain::forge`](crate::domain::forge)).
//!
//! Nothing in the raw layer is edited by either of them. Closing a
//! pursuit records that a line of work ended and touches no asset: no
//! trash, no label, no rating. Integrating the conclusion back into the
//! library is the raw layer's own business, and stays on the raw
//! layer's verbs.

pub mod mapping;
pub mod project_service;
pub mod pursuit_service;

pub use project_service::ProjectService;
pub use pursuit_service::PursuitService;
