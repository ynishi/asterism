//! Forge use cases — the verbs of a line of work.
//!
//! [`pursuit_service`] owns the lifecycle (open / close / reopen /
//! restamp) and the reads over it; [`dispatch_service`] starts a round
//! and files it under the pursuit the caller named, or under none when
//! the caller named none ([`DispatchJob::pursuit_id`] is an `Option`,
//! and an export that files nowhere is an ordinary export). They are
//! together because they are the two halves of one story — a pursuit
//! with no round records nothing — and apart from the rest because
//! they are the only services here whose writes carry intent rather
//! than content (the layer itself is described in
//! [`domain::forge`](crate::domain::forge)).
//!
//! [`DispatchJob::pursuit_id`]: crate::domain::dispatch::DispatchJob::pursuit_id
//!
//! Nothing in the catalogue is edited by either of them. Closing a
//! pursuit freezes what was kept and touches no asset: no trash, no
//! label, no rating. Integrating the conclusion back into the library is
//! the catalogue's own business, and stays on the catalogue's verbs.

pub mod dispatch_service;
pub mod mapping;
pub mod project_service;
pub mod pursuit_service;

pub use dispatch_service::DispatchService;
pub use project_service::ProjectService;
pub use pursuit_service::PursuitService;
