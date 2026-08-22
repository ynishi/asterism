//! What every forge store keeps, whatever it keeps it in.
//!
//! [`rows`] is the shape, and both adapters serve it: the in-memory one
//! holds these values under a lock, the SQLite one writes them as
//! columns. Having it here rather than inside either is what makes
//! that a shared contract rather than one of them copying the other —
//! and it is why the SQLite tables can be read as owing what this
//! module already says.
//!
//! Nothing here talks to a store. Taking a domain value apart and
//! putting one back is the same work whichever store is underneath,
//! and the half that differs is the half that is not here.

pub mod rows;
