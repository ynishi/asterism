//! `RetentionService` — the trash retention sweep.
//!
//! **Driven by the `trash_purge` job handler, and by nothing else.**
//! No Tauri command and no HTTP route fronts this: purging on a clock
//! is a scheduled destruction of the user's data, and the only way to
//! ask for it is to be the worker whose page-at-a-time, self-chaining
//! contract bounds it. A user-initiated purge is a different verb with
//! a different guard — [`AssetService::purge`](crate::application::AssetService::purge)
//! — which refuses anything not already in the trash, and that one is
//! transport-fronted precisely because it acts on one row the user
//! named.
//!
//! The retention period is injected (from the composition root, which
//! reads `ASTERISM_TRASH_RETENTION_DAYS`) rather than declared here:
//! the cutoff is policy, and this layer must not carry a policy
//! number.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};

use crate::domain::repository::{
    AssetIndexer, AssetRepository, GroupRepository, PersonaRepository,
};
use crate::error::DomainError;

/// One page of the retention sweep, as reported by
/// [`RetentionService::purge_expired`].
///
/// `scanned` vs `purged` is the load-bearing distinction: the caller
/// chain-enqueues while a page comes back full, and "full" has to mean
/// *scanned*, not *purged* — otherwise a page where every row was
/// skipped would look like the end of the backlog and stall the sweep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sweep {
    /// Trashed assets the scan returned for this page.
    pub assets_scanned: u64,
    /// Assets actually purged.
    pub assets_purged: u64,
    /// Trashed Groups the scan returned for this page.
    pub groups_scanned: u64,
    /// Groups actually purged.
    pub groups_purged: u64,
    /// Trashed personas the scan returned for this page.
    pub personas_scanned: u64,
    /// Personas actually purged (each one cascading over whatever it
    /// still held).
    pub personas_purged: u64,
    /// Rows that failed and were left for the next run (typically a
    /// restore that landed mid-sweep).
    pub skipped: u64,
}

impl Sweep {
    /// `true` when either scan filled its page, meaning there is very
    /// likely more backlog behind it.
    pub fn page_was_full(&self, limit: u32) -> bool {
        let limit = u64::from(limit);
        self.assets_scanned >= limit
            || self.groups_scanned >= limit
            || self.personas_scanned >= limit
    }

    /// Rows this page actually removed.
    pub fn purged(&self) -> u64 {
        self.assets_purged + self.groups_purged + self.personas_purged
    }

    /// `true` when the caller should run another page immediately.
    ///
    /// Requires **both** a full page and real progress. Progress is the
    /// termination guarantee: a page that removed nothing would chain
    /// into an identical scan, and a self-chaining job with no retry
    /// policy behind it turns that into an unbounded loop that only
    /// shows up as queue-table growth. The expected skip (a restore
    /// landing mid-sweep) does not need the chain — the restored row
    /// stops matching the scan predicate anyway.
    pub fn should_chain(&self, limit: u32) -> bool {
        self.page_was_full(limit) && self.purged() > 0
    }

    /// `true` when the sweep found nothing at all to act on.
    pub fn is_empty(&self) -> bool {
        self.assets_scanned == 0 && self.groups_scanned == 0 && self.personas_scanned == 0
    }
}

/// Retention sweep actuator. Held by `CoreCtx`'s support bundle and
/// handed to the job worker through `JobDeps`; it is not reachable
/// from `ServerCtx` / `AppState`.
pub struct RetentionService {
    assets: Arc<dyn AssetRepository>,
    groups: Arc<dyn GroupRepository>,
    personas: Arc<dyn PersonaRepository>,
    /// Retrieval index, write side — a purged asset's document has to
    /// go with its row, or retrieval keeps offering an id nothing can
    /// hydrate.
    indexer: Arc<dyn AssetIndexer>,
    /// How long a trashed asset / Group / persona survives before this
    /// sweep may purge it.
    trash_retention: Duration,
}

impl RetentionService {
    /// Wires the sweep around the ports it destroys through.
    pub fn new(
        assets: Arc<dyn AssetRepository>,
        groups: Arc<dyn GroupRepository>,
        personas: Arc<dyn PersonaRepository>,
        indexer: Arc<dyn AssetIndexer>,
        trash_retention: Duration,
    ) -> Self {
        Self {
            assets,
            groups,
            personas,
            indexer,
            trash_retention,
        }
    }

    /// Purges every asset, Group and persona whose trash stamp is older
    /// than the injected retention period, oldest first, capped at
    /// `limit` each.
    ///
    /// The cutoff is derived from `now` and the retention period the
    /// service was constructed with, so a job that sat in the queue
    /// across a policy change purges on the current policy rather than
    /// the one in force when it was enqueued.
    ///
    /// A single row's failure does **not** abort the sweep. The realistic
    /// cause is a restore landing between the scan and the purge, which
    /// comes back as `Conflict` — and letting one recovered asset cancel
    /// the rest of the page would leave the backlog growing for a reason
    /// nobody can see. Failures are counted and reported instead.
    pub async fn purge_expired(
        &self,
        now: DateTime<Utc>,
        limit: u32,
    ) -> Result<Sweep, DomainError> {
        let cutoff = now - self.trash_retention;
        let mut sweep = Sweep::default();

        let asset_ids = self.assets.scan_purgeable(cutoff, limit).await?;
        sweep.assets_scanned = asset_ids.len() as u64;
        // One commit for the whole page rather than one per asset: the
        // index port asks callers to batch (a commit is ~10-100 ms), and
        // a 5 000-row sweep would otherwise take 5 000 of them on the
        // shared writer.
        let mut purged_ids = Vec::new();
        for id in asset_ids {
            match self.assets.purge(&id).await {
                Ok(()) => {
                    purged_ids.push(id);
                    sweep.assets_purged += 1;
                }
                Err(err) => {
                    tracing::warn!(
                        event = "diag.retention.purge_failed",
                        entity = "asset",
                        id = %id,
                        error = %err,
                        "retention sweep skipped asset"
                    );
                    sweep.skipped += 1;
                }
            }
        }
        self.unindex_swept_assets(&purged_ids).await;

        let group_ids = self.groups.scan_purgeable(cutoff, limit).await?;
        sweep.groups_scanned = group_ids.len() as u64;
        for id in group_ids {
            match self.groups.purge(&id).await {
                Ok(()) => sweep.groups_purged += 1,
                Err(err) => {
                    tracing::warn!(
                        event = "diag.retention.purge_failed",
                        entity = "group",
                        id = %id,
                        error = %err,
                        "retention sweep skipped group"
                    );
                    sweep.skipped += 1;
                }
            }
        }

        // Personas last, and deliberately so: purging one cascades over
        // whatever assets and Groups it still holds, so running it after
        // the per-entity passes keeps the common case (individually
        // trashed rows) accounted for in the asset / group counters
        // rather than vanishing inside a persona cascade.
        let persona_ids = self.personas.scan_purgeable(cutoff, limit).await?;
        sweep.personas_scanned = persona_ids.len() as u64;
        for id in persona_ids {
            // Collect the doomed documents before the cascade, for the
            // same reason `PersonaService::purge` does.
            let doomed = self.assets.ids_by_persona(&id).await.unwrap_or_default();
            match self.personas.purge(&id).await {
                Ok(()) => {
                    sweep.personas_purged += 1;
                    self.unindex_swept_assets(&doomed).await;
                }
                Err(err) => {
                    tracing::warn!(
                        event = "diag.retention.purge_failed",
                        entity = "persona",
                        id = %id,
                        error = %err,
                        "retention sweep skipped persona"
                    );
                    sweep.skipped += 1;
                }
            }
        }
        Ok(sweep)
    }

    /// Takes the assets this sweep purged out of search, behind one
    /// commit per batch.
    ///
    /// The twin of `AssetService`'s own unindex step, deliberately not
    /// shared with it. The two answer different questions — "the user
    /// removed this" versus "this outlived its retention window" — and
    /// the sweep's batches are pages of rows the user never named. A
    /// shared helper would have to live on one side or the other, which
    /// would either put a clock-driven concern on a service transports
    /// can reach, or make the user-facing path depend on the worker's.
    ///
    /// Failures are logged, not propagated: the purge already happened,
    /// so failing here would abort a sweep over a document that is
    /// merely stale. It disappears at the next reindex.
    async fn unindex_swept_assets(&self, ids: &[crate::domain::value::AssetId]) {
        if ids.is_empty() {
            return;
        }
        let mut dropped = false;
        for id in ids {
            match self.indexer.remove(id).await {
                Ok(()) => dropped = true,
                Err(err) => tracing::warn!(
                    event = "diag.retrieval.remove_failed",
                    asset_id = %id,
                    error = %err,
                    "retrieval index remove failed"
                ),
            }
        }
        if dropped && let Err(err) = self.indexer.flush().await {
            tracing::warn!(
                event = "diag.retrieval.flush_failed",
                dropped = ids.len(),
                error = %err,
                "retrieval index flush failed after dropping documents"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chaining needs a full page **and** progress. Fullness is measured
    /// on scanned rows so a partly-skipped page is not mistaken for the
    /// end of the backlog; progress is what makes the loop terminate.
    #[test]
    fn sweep_chains_only_on_a_full_page_that_made_progress() {
        // A page where nothing could be purged would chain into an
        // identical scan. This job has no retry policy behind it, so
        // that is an unbounded loop, not a retry.
        let all_skipped = Sweep {
            assets_scanned: 200,
            assets_purged: 0,
            skipped: 200,
            ..Sweep::default()
        };
        assert!(all_skipped.page_was_full(200), "the page was full");
        assert!(
            !all_skipped.should_chain(200),
            "…but a zero-progress page must not chain"
        );
        assert!(!all_skipped.is_empty(), "and it is not a no-op either");

        // Partial progress on a full page still means backlog behind it.
        let partial_progress = Sweep {
            assets_scanned: 200,
            assets_purged: 199,
            skipped: 1,
            ..Sweep::default()
        };
        assert!(partial_progress.should_chain(200));

        let short_page = Sweep {
            assets_scanned: 3,
            assets_purged: 3,
            ..Sweep::default()
        };
        assert!(
            !short_page.should_chain(200),
            "a short page is the end of the backlog"
        );

        // Groups alone can fill a page — the asset side being empty must
        // not end the sweep early.
        let groups_only = Sweep {
            groups_scanned: 200,
            groups_purged: 200,
            ..Sweep::default()
        };
        assert!(groups_only.should_chain(200));
        assert!(!groups_only.is_empty());

        assert!(Sweep::default().is_empty());
        assert!(!Sweep::default().should_chain(200));
    }
}
