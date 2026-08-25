//! Domain types and invariants of the teams plane — everything here is
//! IO-free and unit-testable on a machine with no database and no disk.

pub mod head_registry;
pub mod identity;
pub mod ledger;
pub mod projection;
pub mod store;
