//! `QueryGroupRefreshService` — bulk Query Group re-evaluation.
//!
//! **Driven by the `query_group_refresh` job handler and by process
//! startup, and by nothing else.** No Tauri command and no HTTP route
//! fronts either sweep: a user never asks for "re-evaluate everything"
//! — the writes that could invalidate a rule enqueue
//! [`JobKind::QueryGroupRefresh`](crate::domain::job::JobKind) through
//! the invalidator (W4), and the
//! startup pass exists to close the window the V19 migration could not
//! (no async isle, no Tantivy at raw-connection migrate time). The
//! transport-fronted verbs are per-group and live on
//! [`QueryGroupService`]: `create_query_group` and `update_query`, each
//! of which evaluates the one group it just wrote.
//!
//! This service is therefore a thin driver over
//! [`QueryGroupService::evaluate_and_materialize`] rather than a home
//! for it — that method has three callers on the transport side (both
//! commands above, plus `DispatchService::run`'s pre-freeze
//! refresh), so it stays in `application` where they can reach it.
//! Sweeping *every* group is the part nothing on the wire asks for.
//!
//! Both sweeps are fail-loud and keep-going: one corrupt rule must not
//! stop the rest, and every failure comes back in the outcome for the
//! caller to surface.

use std::sync::Arc;

use crate::application::QueryGroupService;
use crate::domain::repository::{QueryGroupRepository, QueryGroupRow};
use crate::domain::value::{GroupId, PersonaId};
use crate::error::DomainError;

/// Result of a [`QueryGroupRefreshService::refresh_all`] or
/// [`refresh_for_persona`](QueryGroupRefreshService::refresh_for_persona)
/// sweep.
///
/// `failures` carries `(bucket, error)` per failed group — `None` for a
/// failure of the listing itself (nothing was evaluated). The caller is
/// responsible for surfacing every entry loudly.
pub struct RefreshAllOutcome {
    /// Number of query groups successfully re-materialised.
    pub refreshed: usize,
    /// Per-group failures (bucket id when the listing succeeded).
    pub failures: Vec<(Option<GroupId>, DomainError)>,
}

/// Bulk Query Group refresh driver. Held by `CoreCtx`'s support bundle
/// and handed to the job worker through `JobDeps`; it is not reachable
/// from `ServerCtx` / `AppState`.
pub struct QueryGroupRefreshService {
    /// Listing port — the set of groups a sweep walks.
    query_groups: Arc<dyn QueryGroupRepository>,
    /// The per-group evaluator. Composed rather than reimplemented:
    /// there is exactly one evaluate-and-materialize pipeline, and a
    /// sweep is a loop over it.
    evaluator: Arc<QueryGroupService>,
}

impl QueryGroupRefreshService {
    /// Wires the sweep around the listing port and the evaluator.
    pub fn new(
        query_groups: Arc<dyn QueryGroupRepository>,
        evaluator: Arc<QueryGroupService>,
    ) -> Self {
        Self {
            query_groups,
            evaluator,
        }
    }

    /// Re-evaluates every Query Group that belongs to `persona_id`.
    ///
    /// This is the W4 refresh-job body: the invalidation hook enqueues
    /// one job per persona
    /// touched by a write, and the handler calls this. Same
    /// fail-loud, keep-going policy as [`refresh_all`](Self::refresh_all)
    /// — one corrupt rule must not stop the rest of the persona's
    /// groups from refreshing.
    pub async fn refresh_for_persona(&self, persona_id: &PersonaId) -> RefreshAllOutcome {
        self.sweep(|row| row.persona_id == *persona_id).await
    }

    /// Re-evaluates **every** Query Group across all personas,
    /// returning a per-group outcome instead of aborting on the first
    /// failure — one corrupt rule must not take the whole startup
    /// down, but the caller is expected to surface every failure
    /// loudly (no silent swallow).
    ///
    /// The *initial* evaluation would belong inside the V19
    /// migration; the evaluator needs the async isle + the Tantivy
    /// index, neither of which exists at raw-connection migrate time,
    /// so the equivalent guarantee is provided by calling this
    /// **before the UI serves** (startup-blocking, `core_init`).
    /// Evaluation order is the repository's deterministic listing —
    /// sufficient while query groups can only reference manual groups
    /// (every V19-transcribed rule); the topological
    /// ordering for query→query references lands with the W4 refresh
    /// job.
    pub async fn refresh_all(&self) -> RefreshAllOutcome {
        self.sweep(|_| true).await
    }

    /// Shared sweep body: list, filter, evaluate each, collect.
    ///
    /// Written once so the two entry points cannot drift on the
    /// keep-going policy — the difference between them is the
    /// predicate and nothing else.
    async fn sweep(&self, keep: impl Fn(&QueryGroupRow) -> bool) -> RefreshAllOutcome {
        let groups: Vec<QueryGroupRow> = match self.query_groups.list_query_groups().await {
            Ok(g) => g,
            Err(e) => {
                return RefreshAllOutcome {
                    refreshed: 0,
                    failures: vec![(None, e)],
                };
            }
        };
        let mut outcome = RefreshAllOutcome {
            refreshed: 0,
            failures: Vec::new(),
        };
        for g in groups.into_iter().filter(|g| keep(g)) {
            match self
                .evaluator
                .evaluate_and_materialize(&g.query_json, &g.persona_id, &g.bucket_id)
                .await
            {
                Ok(_) => outcome.refreshed += 1,
                Err(e) => outcome.failures.push((Some(g.bucket_id), e)),
            }
        }
        outcome
    }
}
