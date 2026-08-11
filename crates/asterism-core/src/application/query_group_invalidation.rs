//! Query Group invalidation — the W4 hook that translates a
//! user-facing `AssetService` write into a coarse per-persona refresh.
//!
//! # Shape
//!
//! [`AssetService`] holds one [`QueryGroupInvalidator`]. Every write
//! method that changes a field a Query Group rule could read
//! (asset add / delete, tag attach / detach, manual group mutation)
//! calls [`QueryGroupInvalidator::notify_persona`] with the persona
//! id it touched. The invalidator debounces a burst — many writes to
//! the same persona in a short window collapse into a single job —
//! and then enqueues one [`JobKind::QueryGroupRefresh`] whose handler
//! reruns every Query Group in that persona.
//!
//! # Why debounce
//!
//! Bulk operations (`attach_tag_batch`, `add_memo`, big
//! `bulk_move_modality` rounds) fire dozens of writes back-to-back. A
//! naïve "enqueue per write" would flood apalis with duplicate jobs
//! and re-run the same evaluation over and over. The debounce is
//! per-persona: writes to persona P schedule a refresh for `now +
//! DEBOUNCE_MS`; a second write to P within the window rearms the
//! timer instead of enqueuing again. `now + DEBOUNCE_MS` chosen to be
//! long enough to swallow a bulk loop but short enough that a manual
//! edit's refresh feels immediate (200 ms).
//!
//! # Job-loop safety
//!
//! The refresh handler's own write
//! (`SqliteQueryGroupRepository::replace_membership`) never re-enters
//! [`AssetService`], so the "refresh must not re-fire itself"
//! invariant "job-derived writes are excluded from the hook"
//! holds structurally at this hook site — no per-call
//! opt-out is needed.
//!
//! # Job-chain rule inputs (W4-a)
//!
//! Background jobs also write fields Query Group rules read —
//! `handlers::auto_tag` links tags (the `tag_ids` filter dimension)
//! and `index_rebuild` refreshes Tantivy (the `search_text`
//! dimension). W4-a wires the same invalidator into those handlers
//! through the late-bound `JobDeps::query_group_invalidator` cell, so
//! their writes refresh memberships without waiting for the next
//! user-facing write. Deliberately out: `cover_gen` (its
//! `content_flags` / `cover` writes have no filter dimension in the
//! query contract — a refresh would be a no-op re-evaluation) and
//! `session_rebuild` (persona-less payload; the session projection
//! is not a rule input).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::domain::job::JobKind;
use crate::domain::repository::JobQueue;
use crate::domain::value::PersonaId;

/// Coalescing window: writes to the same persona within this window
/// share one refresh job. 200 ms is long enough to swallow a bulk
/// loop, short enough that a single edit's refresh feels immediate.
pub const DEBOUNCE_MS: u64 = 200;

/// Wire payload for [`JobKind::QueryGroupRefresh`].
///
/// Kept alongside the invalidator so the enqueue side and the
/// handler side share one type definition.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct QueryGroupRefreshPayload {
    /// Persona whose Query Groups the handler will re-evaluate.
    pub persona_id: String,
}

/// Per-persona debouncing enqueuer for
/// [`JobKind::QueryGroupRefresh`]. Cheap to clone (all state is
/// behind an `Arc`) so [`AssetService`] can hand a clone to any
/// method that mutates a persona-scoped field.
#[derive(Clone)]
pub struct QueryGroupInvalidator {
    inner: Arc<Inner>,
}

/// Per-slot generation carried alongside the JoinHandle. The
/// self-cleanup path at the end of a spawned task only clears its
/// slot when the counter still matches — otherwise a later rearm
/// has already replaced the slot, and blind removal would drop the
/// fresh handle (`JoinHandle` drop does **not** cancel the task, so
/// a following rearm's `abort()` lookup would return `None` and the
/// stale task would still enqueue).
struct Slot {
    id: u64,
    handle: JoinHandle<()>,
}

struct Inner {
    jobs: Arc<dyn JobQueue>,
    pending: Mutex<HashMap<PersonaId, Slot>>,
    next_id: std::sync::atomic::AtomicU64,
}

impl QueryGroupInvalidator {
    /// Wires the invalidator around a job queue.
    pub fn new(jobs: Arc<dyn JobQueue>) -> Self {
        Self {
            inner: Arc::new(Inner {
                jobs,
                pending: Mutex::new(HashMap::new()),
                next_id: std::sync::atomic::AtomicU64::new(1),
            }),
        }
    }

    /// Schedules a debounced refresh for `persona_id`. Safe to call
    /// on every write — bursts collapse to a single enqueue.
    pub fn notify_persona(&self, persona_id: PersonaId) {
        let inner = self.inner.clone();
        let my_id = inner
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut pending = inner.pending.lock().expect("pending mutex poisoned");
        // Rearm: cancel any timer already scheduled for this persona,
        // then start a fresh one. `JoinHandle::abort` is a no-op on
        // an already-finished task, and it only takes effect at an
        // await point — a task that already passed the sleep will
        // still enqueue, which is fine (the enqueue is idempotent
        // as far as correctness is concerned; F3-safe cleanup below
        // keeps the *slot* consistent).
        if let Some(prev) = pending.remove(&persona_id) {
            prev.handle.abort();
        }
        let inner_for_task = inner.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;
            let payload = QueryGroupRefreshPayload {
                persona_id: persona_id.to_string(),
            };
            let json = serde_json::to_value(&payload).unwrap_or_else(|_| serde_json::json!({}));
            let enqueue_result = inner_for_task
                .jobs
                .enqueue(JobKind::QueryGroupRefresh, json)
                .await;
            if let Err(err) = enqueue_result {
                // The write already succeeded; a queue push failure
                // means the query groups get one stale window until
                // the next write. Loud enough to notice, non-fatal.
                tracing::warn!(
                    event = "diag.query_group.enqueue_failed",
                    persona_id = %persona_id,
                    error = %err,
                    "query_group_refresh enqueue failed"
                );
            }
            // Only clear the slot if it is still ours — a later
            // rearm may already have replaced it, and blind removal
            // would drop the fresh handle (JoinHandle drop does not
            // cancel the task) so the next rearm's abort() lookup
            // returns None and the stale task still enqueues.
            let mut pending = inner_for_task
                .pending
                .lock()
                .expect("pending mutex poisoned");
            if pending.get(&persona_id).map(|s| s.id) == Some(my_id) {
                pending.remove(&persona_id);
            }
        });
        pending.insert(persona_id, Slot { id: my_id, handle });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::job::JobKind;
    use crate::error::DomainError;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingQueue {
        n: AtomicUsize,
    }

    #[async_trait]
    impl JobQueue for CountingQueue {
        async fn enqueue(
            &self,
            _kind: JobKind,
            _payload: serde_json::Value,
        ) -> Result<String, DomainError> {
            self.n.fetch_add(1, Ordering::SeqCst);
            Ok("noop".into())
        }
    }

    #[tokio::test]
    async fn burst_collapses_to_one_enqueue() {
        let q = Arc::new(CountingQueue {
            n: AtomicUsize::new(0),
        });
        let inv = QueryGroupInvalidator::new(q.clone());
        let persona = PersonaId::new();
        // Fire five writes back-to-back — every rearm cancels the
        // previous timer, so only the last one survives.
        for _ in 0..5 {
            inv.notify_persona(persona);
        }
        tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS + 100)).await;
        assert_eq!(q.n.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rearm_after_prior_timer_expires_still_enqueues_new_job() {
        // Regression for the cleanup race: a rearm arriving after the
        // first timer has already expired must still schedule its own
        // enqueue. Without the generation guard, the expired task's
        // late cleanup would drop the fresh slot and no further
        // rearm within the persona would notice (its abort() lookup
        // would miss the already-dropped handle).
        let q = Arc::new(CountingQueue {
            n: AtomicUsize::new(0),
        });
        let inv = QueryGroupInvalidator::new(q.clone());
        let persona = PersonaId::new();
        inv.notify_persona(persona);
        // Let the first timer expire + enqueue + late cleanup run.
        tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS + 100)).await;
        assert_eq!(q.n.load(Ordering::SeqCst), 1);
        // Second rearm — must enqueue again.
        inv.notify_persona(persona);
        tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS + 100)).await;
        assert_eq!(q.n.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn two_personas_get_independent_jobs() {
        let q = Arc::new(CountingQueue {
            n: AtomicUsize::new(0),
        });
        let inv = QueryGroupInvalidator::new(q.clone());
        let (a, b) = (PersonaId::new(), PersonaId::new());
        inv.notify_persona(a);
        inv.notify_persona(b);
        tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS + 100)).await;
        assert_eq!(q.n.load(Ordering::SeqCst), 2);
    }
}
