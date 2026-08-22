//! Domain types and invariants of the teams plane — everything here is
//! IO-free and unit-testable on a machine with no database and no disk.

pub mod identity;
pub mod ledger;
pub mod model_registry;
pub mod store;
