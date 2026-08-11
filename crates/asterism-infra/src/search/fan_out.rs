//! One [`AssetIndexer`] over several.
//!
//! An asset's body feeds two indexes with different jobs: the SQL
//! `asset_fts` trigram index answers the Query-side `text_match`
//! predicate (an exact set), and the Tantivy index answers Retrieval
//! (a ranked shortlist). They are separate because the questions are
//! separate — but they go stale together, so keeping them in step is
//! one concern, not two.
//!
//! Fanning out here rather than at each call site means every path
//! that already maintains the index (rebuild / trash / fold / purge)
//! maintains both, with no chance of a new path remembering one and
//! forgetting the other.
//!
//! # Failure is not partial-silent
//!
//! Every member runs even when an earlier one fails, and the **first**
//! error is returned. Stopping at the first failure would leave the
//! remaining indexes untouched with no signal about which ones, which
//! is a worse state to recover from than "all attempted, one reported".
//! The recovery path is the same either way: re-run `IndexRebuild`.

use asterism_core::domain::repository::{AssetIndexer, IndexDoc};
use asterism_core::domain::value::AssetId;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use std::sync::Arc;

/// Fans one indexer call out to every member, in order.
pub struct FanOutIndexer {
    members: Vec<Arc<dyn AssetIndexer>>,
}

impl FanOutIndexer {
    /// Wraps the members. Order is the call order; it matters only for
    /// which error surfaces when more than one fails.
    pub fn new(members: Vec<Arc<dyn AssetIndexer>>) -> Self {
        Self { members }
    }
}

/// Awaits `$call` for every member, returning the first error while
/// still having called the rest. A macro rather than a helper taking a
/// closure: a closure returning a borrowing future needs a lifetime
/// this trait's `&self` methods cannot express.
macro_rules! fan_out {
    ($self:ident, $member:ident => $call:expr) => {{
        let mut first_err = None;
        for $member in &$self.members {
            if let Err(e) = $call.await
                && first_err.is_none()
            {
                first_err = Some(e);
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }};
}

#[async_trait]
impl AssetIndexer for FanOutIndexer {
    async fn upsert(&self, doc: &IndexDoc) -> Result<(), DomainError> {
        fan_out!(self, m => m.upsert(doc))
    }

    async fn remove(&self, asset_id: &AssetId) -> Result<(), DomainError> {
        fan_out!(self, m => m.remove(asset_id))
    }

    async fn flush(&self) -> Result<(), DomainError> {
        fan_out!(self, m => m.flush())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records what it was asked to do, and can be told to fail.
    struct Spy {
        name: &'static str,
        fail: bool,
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl AssetIndexer for Spy {
        async fn upsert(&self, doc: &IndexDoc) -> Result<(), DomainError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:upsert:{}", self.name, doc.asset_id));
            if self.fail {
                return Err(DomainError::Validation(format!("{} refused", self.name)));
            }
            Ok(())
        }
        async fn remove(&self, asset_id: &AssetId) -> Result<(), DomainError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:remove:{asset_id}", self.name));
            if self.fail {
                return Err(DomainError::Validation(format!("{} refused", self.name)));
            }
            Ok(())
        }
        async fn flush(&self) -> Result<(), DomainError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:flush", self.name));
            Ok(())
        }
    }

    fn doc(asset_id: AssetId) -> IndexDoc {
        IndexDoc {
            asset_id,
            persona_id: asterism_core::domain::value::PersonaId::new(),
            text: Some("body".into()),
        }
    }

    #[tokio::test]
    async fn every_member_is_called() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let id = AssetId::new();
        let fan = FanOutIndexer::new(vec![
            Arc::new(Spy {
                name: "sql",
                fail: false,
                calls: calls.clone(),
            }),
            Arc::new(Spy {
                name: "tantivy",
                fail: false,
                calls: calls.clone(),
            }),
        ]);
        fan.upsert(&doc(id)).await.unwrap();
        fan.remove(&id).await.unwrap();
        fan.flush().await.unwrap();
        let seen = calls.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![
                format!("sql:upsert:{id}"),
                format!("tantivy:upsert:{id}"),
                format!("sql:remove:{id}"),
                format!("tantivy:remove:{id}"),
                "sql:flush".to_string(),
                "tantivy:flush".to_string(),
            ]
        );
    }

    /// A failing member must not cancel the ones behind it: leaving an
    /// index untouched *and* unreported is the state that cannot be
    /// diagnosed later.
    #[tokio::test]
    async fn a_failing_member_does_not_stop_the_others() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let id = AssetId::new();
        let fan = FanOutIndexer::new(vec![
            Arc::new(Spy {
                name: "sql",
                fail: true,
                calls: calls.clone(),
            }),
            Arc::new(Spy {
                name: "tantivy",
                fail: false,
                calls: calls.clone(),
            }),
        ]);
        let err = fan.upsert(&doc(id)).await.unwrap_err();
        assert!(err.to_string().contains("sql refused"));
        assert_eq!(
            calls.lock().unwrap().clone(),
            vec![format!("sql:upsert:{id}"), format!("tantivy:upsert:{id}")],
            "the member behind the failure still ran"
        );
    }
}
