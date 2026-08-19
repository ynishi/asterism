//! `gc` — the zero-link sweep (#83 §3 registry-GC shape, the #95
//! slice): blob bytes that no team links anymore are deleted, after a
//! reclaim and on demand (`teams-server gc`).
//!
//! ## What the sweep protects
//!
//! A blob's bytes survive while **any** link row references its digest
//! — marked-for-purge links included, because a marked link is
//! restorable during its grace window and restoring a link whose bytes
//! were swept would be a dangling reference by another name. The
//! sweep's question is therefore
//! [`SqliteTeamsRepository::digest_linked_anywhere`], deliberately not
//! the read surface's visibility predicate.
//!
//! ## The racing same-digest upload, and why the answer is a lock
//!
//! The #93 adapter's write path makes bytes durable (staging → rename)
//! **before** the link row commits (#83 §3 ordering). That order is
//! what makes a dangling link impossible for uploads — and it is
//! exactly what a concurrent sweep could break: between the upload's
//! rename and its link commit, the digest has bytes and zero links,
//! and a sweep deciding in that window would delete bytes whose link
//! is about to commit — a dangling link, manufactured by the sweeper.
//!
//! Re-checking links after removing the file cannot close this: the
//! hazardous interleaving (upload renames → sweep checks links, sees
//! zero → sweep deletes → upload's link commits) has the re-check land
//! *before* the link exists, however many times it re-checks. What
//! closes it is excluding the interleaving: [`GcGuard`] is a
//! `tokio::sync::RwLock` — every upload holds it **shared** across its
//! rename→link-commit span (uploads never block each other), and the
//! sweep holds it **exclusive** across its check-and-delete, so no
//! upload is ever mid-span while the sweep decides. This leans on the
//! single-process deployment shape #93 fixed (one server process owns
//! DB and blob dir); a second process bypasses the guard, which is why
//! the `gc` CLI documents "stopped server or same process" and why
//! cross-process coordination stays out of scope (#95 out-of-scope
//! list).

use std::sync::Arc;

use teams_core::DomainError;
use teams_core::port::blob::BlobStore as _;
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::blob::LocalFileStorageAdapter;
use crate::sqlite::SqliteTeamsRepository;

/// The lock that keeps the zero-link sweep and the upload write path
/// from interleaving (module doc). One per process, shared between the
/// upload handlers and every sweep — construct it once beside the
/// adapter and hand `Arc`s around.
#[derive(Debug, Default)]
pub struct GcGuard {
    lock: RwLock<()>,
}

impl GcGuard {
    /// A fresh guard.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enters the link phase — the span an upload must hold from just
    /// before its staging commit (the rename that makes bytes durable)
    /// until its link row + ledger event have committed. Shared:
    /// uploads only ever add references, so they exclude the sweep,
    /// never each other.
    pub async fn link_phase(&self) -> RwLockReadGuard<'_, ()> {
        self.lock.read().await
    }

    /// Enters the sweep phase — exclusive, held across the whole
    /// check-and-delete. Private to the module: the sweep is the one
    /// legitimate holder, and exposing it would invite a second
    /// deleter the invariant does not cover.
    async fn sweep_phase(&self) -> RwLockWriteGuard<'_, ()> {
        self.lock.write().await
    }
}

/// Deletes every CAS blob that no team links (marked links count as
/// links — module doc), returning the digests whose bytes went.
///
/// Runs under the guard's exclusive phase, so no upload sits between
/// its rename and its link commit while this decides; a same-digest
/// upload therefore lands wholly before the sweep (linked — spared) or
/// wholly after it (bytes rewritten by its own staging → rename, then
/// linked). Deletion itself is the adapter's idempotent `delete`, so a
/// digest that vanished by other means is "already true", not an
/// error.
pub async fn sweep_zero_link_blobs(
    guard: &Arc<GcGuard>,
    repo: &SqliteTeamsRepository,
    blobs: &LocalFileStorageAdapter,
) -> Result<Vec<String>, DomainError> {
    let _exclusive = guard.sweep_phase().await;
    let mut swept = Vec::new();
    for digest in blobs.list().await? {
        if !repo.digest_linked_anywhere(&digest).await? {
            blobs.delete(&digest).await?;
            swept.push(digest);
        }
    }
    Ok(swept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest as _, Sha256};
    use teams_core::domain::identity::{ActorStamp, LedgerActor, Membership, Role};
    use teams_core::domain::store::{DeclaredDigest, TeamBlobLink};
    use uuid::Uuid;

    const T0: i64 = 1_755_000_000_000;

    fn digest_of(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    struct Harness {
        repo: SqliteTeamsRepository,
        blobs: LocalFileStorageAdapter,
        guard: Arc<GcGuard>,
        driver: rusqlite_isle::AsyncIsleDriver,
        #[allow(dead_code)] // Held so the blob root outlives the test.
        dir: tempfile::TempDir,
    }

    async fn harness() -> Harness {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let blobs = LocalFileStorageAdapter::open(dir.path().join("blobs"))
            .await
            .unwrap();
        Harness {
            repo: SqliteTeamsRepository::new(isle),
            blobs,
            guard: Arc::new(GcGuard::new()),
            driver,
            dir,
        }
    }

    fn actor() -> LedgerActor {
        LedgerActor::member(ActorStamp {
            user_id: Uuid::now_v7(),
            display_name: "Hoshino".into(),
        })
    }

    async fn team(h: &Harness) -> (Uuid, LedgerActor) {
        let team_id = Uuid::now_v7();
        let owner_id = Uuid::now_v7();
        let actor = LedgerActor::member(ActorStamp {
            user_id: owner_id,
            display_name: "Hoshino".into(),
        });
        h.repo
            .create_team(
                team_id,
                Membership {
                    user_id: owner_id,
                    team_id,
                    role: Role::Owner,
                },
                actor.clone(),
                T0,
            )
            .await
            .unwrap();
        (team_id, actor)
    }

    /// Puts bytes into the CAS only — the orphan shape an interrupted
    /// upload (or a reclaim) leaves behind.
    async fn orphan(h: &Harness, bytes: &[u8]) -> String {
        let digest = digest_of(bytes);
        h.blobs
            .put(&DeclaredDigest::parse(&digest).unwrap(), bytes.to_vec())
            .await
            .unwrap();
        digest
    }

    /// The server's upload span, as the handler performs it: shared
    /// guard across CAS commit + link commit.
    async fn upload(h: &Harness, team_id: Uuid, bytes: &[u8]) -> String {
        let digest = digest_of(bytes);
        let _link_phase = h.guard.link_phase().await;
        h.blobs
            .put(&DeclaredDigest::parse(&digest).unwrap(), bytes.to_vec())
            .await
            .unwrap();
        h.repo
            .add_blob_link(
                TeamBlobLink::new(team_id, &digest).unwrap(),
                actor(),
                T0 + 1,
            )
            .await
            .unwrap();
        digest
    }

    #[tokio::test]
    async fn the_sweep_takes_orphans_and_spares_linked_and_grace_marked_blobs() {
        let h = harness().await;
        let (team_id, owner) = team(&h).await;

        let orphaned = orphan(&h, b"nobody links these bytes").await;
        let linked = upload(&h, team_id, b"linked bytes").await;
        let marked = upload(&h, team_id, b"marked but restorable").await;
        h.repo
            .mark_blob_link_for_purge(team_id, &marked, owner, T0 + 2)
            .await
            .unwrap();

        let swept = sweep_zero_link_blobs(&h.guard, &h.repo, &h.blobs)
            .await
            .unwrap();
        assert_eq!(swept, vec![orphaned.clone()]);

        // The orphan's bytes are gone; the linked blob and the
        // grace-marked one both survive — a marked link is restorable,
        // so its bytes must be there to come back to.
        assert!(!h.blobs.exists(&orphaned).await.unwrap());
        assert!(h.blobs.exists(&linked).await.unwrap());
        assert!(h.blobs.exists(&marked).await.unwrap());

        // Idempotent: a second sweep finds nothing to do.
        let swept = sweep_zero_link_blobs(&h.guard, &h.repo, &h.blobs)
            .await
            .unwrap();
        assert!(swept.is_empty());

        h.driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_racing_same_digest_upload_ends_with_the_blob_present_and_linked() {
        let h = harness().await;
        let (team_id, _) = team(&h).await;
        let bytes = b"raced between sweep and upload".to_vec();
        // The CAS already holds the digest with zero links — the exact
        // shape that makes it a sweep candidate while an upload of the
        // same digest is in flight.
        let digest = orphan(&h, &bytes).await;

        // The upload holds the guard shared across its rename→link
        // span (with yields inside both spans so the two tasks really
        // interleave everywhere the guard permits); the sweep holds it
        // exclusive across check-and-delete.
        let upload_side = {
            let repo = h.repo.clone();
            let blobs = h.blobs.clone();
            let guard = Arc::clone(&h.guard);
            let bytes = bytes.clone();
            let digest = digest.clone();
            async move {
                tokio::task::yield_now().await;
                let _link_phase = guard.link_phase().await;
                let mut staged = blobs.begin_put().await?;
                for chunk in bytes.chunks(4) {
                    staged.write_chunk(chunk).await?;
                    tokio::task::yield_now().await;
                }
                staged.commit(&DeclaredDigest::parse(&digest)?).await?;
                tokio::task::yield_now().await;
                repo.add_blob_link(TeamBlobLink::new(team_id, &digest)?, actor(), T0 + 1)
                    .await
            }
        };
        let sweep_side = sweep_zero_link_blobs(&h.guard, &h.repo, &h.blobs);
        let (uploaded, swept) = tokio::join!(upload_side, sweep_side);
        uploaded.unwrap();
        let swept = swept.unwrap();

        // Whichever side won the guard, the end state is the same:
        // the blob is present and linked. (If the sweep went first it
        // deleted the orphan and the upload rewrote the bytes; if the
        // upload went first the sweep found the digest linked and
        // spared it.)
        assert!(h.blobs.exists(&digest).await.unwrap(), "bytes present");
        assert!(
            h.repo.blob_link_exists(team_id, &digest).await.unwrap(),
            "link present"
        );
        assert!(
            swept.is_empty() || swept == vec![digest.clone()],
            "the sweep either spared the digest or took only the pre-upload orphan: {swept:?}"
        );

        h.driver.shutdown().await.unwrap();
    }
}
