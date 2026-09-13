//! Where a resumption point is kept, as the importer sees it.
//!
//! #293 gave a scan a position and a run the right to hand one back, and
//! left it homeless: the point was printed and an operator carried it to
//! the next run by hand. This is the port that ends that, and the only
//! thing it says about *where* a point is kept is that somewhere is not
//! here.
//!
//! ## Why a port rather than a call
//!
//! The obvious shape is for the runner to POST to the server directly,
//! and it is the wrong one for the same reason the rest of this crate is
//! written against traits. An importer under test would then need a
//! server; and the transport is the question that is still open — an
//! adapter that runs itself and pushes is the shape taken today, and one
//! Asterism starts and reads is the one that was not. A port is what
//! lets the second arrive without the runner learning about it.
//!
//! [`HttpSyncStore`](crate::client::HttpSyncStore) implements it over
//! the same `ApiClient` every record already travels through, which is
//! why an adapter that keeps its position needs nothing it did not
//! already have.

use async_trait::async_trait;

use crate::port::{SourceError, SyncState};

/// What a stored position is filed under.
///
/// The persona because the same source imported into two personas has
/// two independent positions, and one being ahead says nothing about the
/// other. The partition because it is the scan's own statement of what
/// it is scanning — [`SourceScanner::partition`](crate::SourceScanner::partition)
/// is where it comes from, and a caller must not compose one itself: a
/// key spelled by hand is a key that stops matching the day the scanner
/// changes what it scans, silently, by finding nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StateKey {
    /// Persona the import lands in.
    pub persona_id: String,
    /// The scan's own name for what it is scanning.
    pub partition: String,
}

impl StateKey {
    /// Builds a key from a persona and a scanner's partition.
    pub fn new(persona_id: impl Into<String>, partition: impl Into<String>) -> Self {
        Self {
            persona_id: persona_id.into(),
            partition: partition.into(),
        }
    }
}

/// Somewhere to keep a resumption point between runs.
///
/// Nothing lists, because a run knows its own key and asks for that
/// one. Nothing deletes, because forgetting a position is a request
/// nobody has made — and a delete nobody calls is a delete nobody has
/// tested.
///
/// Failures come back as [`SourceError`] rather than a type of this
/// module's own, so that a caller has one classification to act on. A
/// store that cannot be reached is [`Transient`](SourceError::Transient)
/// and a store that refuses what it is given is
/// [`Config`](SourceError::Config) — the same cuts the rest of an import
/// already reads.
#[async_trait]
pub trait SyncStore: Send + Sync {
    /// The point stored under `key`, or `None` for a source this
    /// persona has not imported yet.
    ///
    /// Absence is an answer. A first run is the ordinary case, and a
    /// caller that had to tell "nobody has been here" from "the store
    /// is broken" by reading a message would get it wrong on the day it
    /// mattered — and getting it wrong means importing the source
    /// again.
    async fn read(&self, key: &StateKey) -> Result<Option<SyncState>, SourceError>;

    /// Stores a point, replacing whatever `key` held.
    ///
    /// Whether this run had earned the right to move the position is
    /// decided before the call, in [`run_import`](crate::run_import),
    /// which is the only place that knows what became of the records in
    /// front of the checkpoint. A store that second-guessed it would be
    /// a second opinion on one question.
    async fn write(&self, key: &StateKey, state: &SyncState) -> Result<(), SourceError>;
}

#[cfg(test)]
pub(crate) mod memory {
    //! An in-memory store, for tests that need a position to survive one
    //! run and reach the next without a server in between.

    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub(crate) struct MemorySyncStore {
        rows: Mutex<HashMap<StateKey, SyncState>>,
    }

    impl MemorySyncStore {
        /// What is stored under `key`, read without going through the
        /// port — so a test can assert what a run left behind rather
        /// than only what the next one sees.
        pub(crate) fn stored(&self, key: &StateKey) -> Option<SyncState> {
            self.rows
                .lock()
                .expect("the fixture's rows")
                .get(key)
                .cloned()
        }
    }

    #[async_trait]
    impl SyncStore for MemorySyncStore {
        async fn read(&self, key: &StateKey) -> Result<Option<SyncState>, SourceError> {
            Ok(self
                .rows
                .lock()
                .expect("the fixture's rows")
                .get(key)
                .cloned())
        }

        async fn write(&self, key: &StateKey, state: &SyncState) -> Result<(), SourceError> {
            self.rows
                .lock()
                .expect("the fixture's rows")
                .insert(key.clone(), state.clone());
            Ok(())
        }
    }
}
