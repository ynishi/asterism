//! `backup` — the all-in-one instance backup (#83 §4, the #95 slice):
//! quiesce → `VACUUM INTO` snapshot → DB-first archive.
//!
//! ## The three steps, and why in this order
//!
//! 1. **Quiesce + snapshot.** The whole `VACUUM INTO` runs inside one
//!    `AsyncIsle` call — the isle *is* the single writer of the
//!    deployment shape (#83 §4), so holding its one connection for the
//!    duration is the app-level consistency point: no repository write
//!    can interleave with the snapshot. The snapshot is never a copy
//!    of the live file — copying a live SQLite file is a documented
//!    corruption path (WAL content is not in the main file), while
//!    `VACUUM INTO` writes a complete, transactionally consistent
//!    database.
//! 2. **DB first, blobs after.** The archive receives the snapshot
//!    before a single blob is read. The asymmetry is #83 §3's, applied
//!    to backup (§4): every link in the snapshot points at bytes that
//!    were durable *before* that link committed and are read *after*
//!    the snapshot — so the worst inconsistency a backup can hold is
//!    an orphan blob (uploaded after the snapshot; harmless, the
//!    restored instance's sweep collects it), never a dangling DB
//!    reference. The one caveat is a **reclaim + sweep landing between
//!    the two steps** — it can remove bytes the snapshot still links —
//!    which is why backup wants a quiet instance (the CLI holds the
//!    writer for step 1, and the single-process shape means a stopped
//!    or idle server for the rest).
//! 3. **One archive.** A plain uncompressed tar (the workspace
//!    manifest's `tar` line says why no compression): `db/teams.db`
//!    first, then `blobs/sha256/<shard>/<hex>` for every blob. Tar
//!    preserves entry order, so "DB first" is not only what the code
//!    does — it is readable in the artefact, and the tests assert it
//!    there. `staging/` is never archived: its contents are garbage by
//!    definition (#93's startup sweep deletes them).
//!
//! ## Where the snapshot lands
//!
//! In a fresh local temp directory, never at the destination: the
//! destination may be a mounted/rclone network target, and a live
//! SQLite file must never sit on network storage (#83 §4 hard rule —
//! network storage is a backup *destination* only). Only the finished
//! archive is written to the destination path.
//!
//! ## Restore
//!
//! Documentation, not a command (#95): unpack the archive, place
//! `db/teams.db` where the server's `--db` points and `blobs/` where
//! `--blobs` points, start the server. The restore e2e in
//! `teams-server` proves the unpacked pair serves an existing link
//! end-to-end. The full text ships on `teams-server backup --help`.

use std::path::{Path, PathBuf};

use rusqlite_isle::AsyncIsle;
use teams_core::DomainError;

/// The snapshot's entry name inside the archive — first entry, always.
pub const ARCHIVE_DB_ENTRY: &str = "db/teams.db";
/// The blob tree's prefix inside the archive; entries continue with
/// the CAS's own layout (`sha256/<shard>/<hex>`).
pub const ARCHIVE_BLOBS_PREFIX: &str = "blobs";

/// What a completed backup wrote — enough for the CLI to report and
/// for a caller to sanity-check.
#[derive(Debug)]
pub struct BackupReport {
    /// The archive that was written.
    pub archive: PathBuf,
    /// Size of the `VACUUM INTO` snapshot, bytes.
    pub db_snapshot_bytes: u64,
    /// How many blob files the archive holds.
    pub blob_files: usize,
}

/// Runs the whole backup: quiesce + `VACUUM INTO` through `isle`, then
/// the DB-first tar of snapshot + `<blob_root>/sha256` to
/// `destination`. See the module doc for the ordering contract.
///
/// `destination` must not already exist — a backup never overwrites a
/// previous one (an interrupted overwrite would destroy the good copy
/// it was replacing); pick a fresh name per run.
pub async fn create_backup(
    isle: &AsyncIsle,
    blob_root: &Path,
    destination: &Path,
) -> Result<BackupReport, DomainError> {
    if destination.exists() {
        return Err(DomainError::Validation(format!(
            "backup destination {} already exists; a backup never overwrites a previous \
             one — pick a fresh name",
            destination.display()
        )));
    }

    // Step 1 — quiesce + snapshot, one isle call: the single writer is
    // held for exactly the snapshot step. The snapshot goes to a local
    // temp dir, never to the (possibly network) destination.
    let snapshot_dir = tempfile::tempdir()
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot create snapshot tempdir: {e}")))?;
    let snapshot_path = snapshot_dir.path().join("teams.db");
    let snapshot_str = snapshot_path.to_str().ok_or_else(|| {
        DomainError::Infra(anyhow::anyhow!(
            "snapshot path {} is not valid UTF-8, which VACUUM INTO cannot take",
            snapshot_path.display()
        ))
    })?;
    let sql = format!("VACUUM INTO {}", quote_sql_string(snapshot_str));
    isle.call(move |conn| conn.execute_batch(&sql))
        .await
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("VACUUM INTO failed: {e}")))?;
    let db_snapshot_bytes = std::fs::metadata(&snapshot_path)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("snapshot vanished after VACUUM: {e}")))?
        .len();

    // Steps 2 + 3 — the tar is sequential sync IO; off the async
    // runtime it goes. The snapshot tempdir moves into the task so it
    // outlives every read.
    let blob_root = blob_root.to_path_buf();
    let destination_owned = destination.to_path_buf();
    let blob_files = tokio::task::spawn_blocking(move || {
        let result = write_archive(&snapshot_path, &blob_root, &destination_owned);
        // The snapshot tempdir lives exactly as long as the archive
        // build that reads from it.
        drop(snapshot_dir);
        result
    })
    .await
    .map_err(|e| DomainError::Infra(anyhow::anyhow!("archive task panicked: {e}")))??;

    Ok(BackupReport {
        archive: destination.to_path_buf(),
        db_snapshot_bytes,
        blob_files,
    })
}

/// Single-quotes a string for embedding in SQL — `VACUUM INTO` takes
/// an expression, and binding a parameter to it is not supported on
/// every SQLite build, so the filename is quoted the one way SQL
/// defines (double every `'`).
fn quote_sql_string(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "''"))
}

/// Builds the archive at `destination`: snapshot first, then every
/// file under `<blob_root>/sha256`, shards and files in sorted order
/// so two backups of the same state are byte-identical. Returns the
/// blob file count.
fn write_archive(
    snapshot: &Path,
    blob_root: &Path,
    destination: &Path,
) -> Result<usize, DomainError> {
    if let Some(parent) = destination.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| {
            DomainError::Infra(anyhow::anyhow!(
                "cannot create destination directory {}: {e}",
                parent.display()
            ))
        })?;
    }
    let file = std::fs::File::create_new(destination).map_err(|e| {
        DomainError::Infra(anyhow::anyhow!(
            "cannot create archive {}: {e}",
            destination.display()
        ))
    })?;
    let mut builder = tar::Builder::new(file);

    // DB first — the ordering rule, in the artefact itself.
    builder
        .append_path_with_name(snapshot, ARCHIVE_DB_ENTRY)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot archive the snapshot: {e}")))?;

    // Blobs after: the CAS tree, `staging/` excluded by walking only
    // `sha256/`. A missing CAS dir is an instance that never stored a
    // blob — an empty blob half, not an error.
    let mut blob_files = 0;
    let cas_root = blob_root.join("sha256");
    if cas_root.is_dir() {
        for shard in sorted_entries(&cas_root)? {
            for blob in sorted_entries(&shard)? {
                let name = format!(
                    "{ARCHIVE_BLOBS_PREFIX}/sha256/{}/{}",
                    file_name_str(&shard)?,
                    file_name_str(&blob)?
                );
                builder.append_path_with_name(&blob, &name).map_err(|e| {
                    DomainError::Infra(anyhow::anyhow!(
                        "cannot archive blob {}: {e}",
                        blob.display()
                    ))
                })?;
                blob_files += 1;
            }
        }
    }

    let file = builder
        .into_inner()
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot finish the archive: {e}")))?;
    // The archive is the artefact the operator relies on; it is not
    // "written" until it is durable.
    file.sync_all().map_err(|e| {
        DomainError::Infra(anyhow::anyhow!(
            "cannot fsync archive {}: {e}",
            destination.display()
        ))
    })?;
    Ok(blob_files)
}

fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>, DomainError> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot list {}: {e}", dir.display())))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot list {}: {e}", dir.display())))?;
    entries.sort();
    Ok(entries)
}

fn file_name_str(path: &Path) -> Result<&str, DomainError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            DomainError::Infra(anyhow::anyhow!(
                "foreign file in the CAS: {} has no UTF-8 name",
                path.display()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest as _, Sha256};
    use teams_core::domain::identity::{ActorStamp, LedgerActor, Membership, Role, Team};
    use teams_core::domain::store::{DeclaredDigest, TeamBlobLink};
    use teams_core::port::blob::BlobStore as _;
    use uuid::Uuid;

    use crate::blob::LocalFileStorageAdapter;
    use crate::sqlite::SqliteTeamsRepository;

    const T0: i64 = 1_755_000_000_000;

    fn digest_of(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    /// A file-backed instance with one team and `blobs` uploaded
    /// through the real write path (bytes durable, then link + event).
    async fn instance(
        dir: &Path,
        blobs: &[&[u8]],
    ) -> (
        rusqlite_isle::AsyncIsle,
        rusqlite_isle::AsyncIsleDriver,
        PathBuf,
        Uuid,
    ) {
        let db_path = dir.join("teams.db");
        let blob_root = dir.join("blobs");
        let (isle, driver) = crate::sqlite::open_and_migrate(&db_path).await.unwrap();
        let repo = SqliteTeamsRepository::new(isle.clone());
        let adapter = LocalFileStorageAdapter::open(&blob_root).await.unwrap();

        let team_id = Uuid::now_v7();
        let owner_id = Uuid::now_v7();
        let actor = LedgerActor::member(ActorStamp {
            user_id: owner_id,
            display_name: "Hoshino".into(),
        });
        repo.create_team(
            Team::new(team_id, "a team").unwrap(),
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
        for bytes in blobs {
            let digest = digest_of(bytes);
            adapter
                .put(&DeclaredDigest::parse(&digest).unwrap(), bytes.to_vec())
                .await
                .unwrap();
            repo.add_blob_link(
                TeamBlobLink::new(team_id, &digest).unwrap(),
                actor.clone(),
                T0 + 1,
            )
            .await
            .unwrap();
        }
        (isle, driver, blob_root, team_id)
    }

    fn entry_names(archive: &Path) -> Vec<String> {
        let file = std::fs::File::open(archive).unwrap();
        let mut tar = tar::Archive::new(file);
        tar.entries()
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[tokio::test]
    async fn the_archive_is_db_first_holds_every_linked_blob_and_no_staging() {
        let dir = tempfile::tempdir().unwrap();
        let (first, second) = (b"first blob".as_slice(), b"second blob".as_slice());
        let (isle, driver, blob_root, _) = instance(dir.path(), &[first, second]).await;
        // A staging leftover, as an interrupted copy leaves — the
        // archive must not carry garbage.
        std::fs::write(blob_root.join("staging").join("put-stale"), b"half").unwrap();

        let dest = dir.path().join("out").join("backup.tar");
        let report = create_backup(&isle, &blob_root, &dest).await.unwrap();
        assert_eq!(report.blob_files, 2);
        assert!(report.db_snapshot_bytes > 0);

        let names = entry_names(&dest);
        // DB first — the ordering rule is readable in the artefact.
        assert_eq!(names[0], ARCHIVE_DB_ENTRY);
        // Every linked blob follows, at its CAS path; no staging, no
        // WAL sidecar (the snapshot is one complete file).
        for bytes in [first, second] {
            let hex = digest_of(bytes);
            let hex = hex.strip_prefix("sha256:").unwrap();
            let expected = format!("blobs/sha256/{}/{hex}", &hex[..2]);
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
        assert_eq!(names.len(), 3);
        assert!(
            names
                .iter()
                .all(|n| !n.contains("staging") && !n.contains("-wal")),
            "no staging garbage, no WAL sidecar: {names:?}"
        );

        // A backup never overwrites a previous one.
        let refused = create_backup(&isle, &blob_root, &dest).await;
        assert!(matches!(refused, Err(DomainError::Validation(_))));

        driver.shutdown().await.unwrap();
    }

    /// The snapshot is a `VACUUM INTO` result, not a live-file copy —
    /// proven by content: under WAL, recent commits live in the `-wal`
    /// sidecar and a naive copy of the main file would miss them,
    /// while the snapshot must hold everything committed at the
    /// consistency point.
    #[tokio::test]
    async fn the_snapshot_is_transactionally_complete_not_a_live_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (isle, driver, blob_root, team_id) =
            instance(dir.path(), &[b"committed just before the backup"]).await;
        // The WAL sidecar exists and is non-trivial — the live main
        // file alone is *not* the database right now, which is exactly
        // why copying it would be corruption.
        let wal = dir.path().join("teams.db-wal");
        assert!(wal.exists(), "WAL mode leaves a -wal beside the live db");

        let dest = dir.path().join("backup.tar");
        create_backup(&isle, &blob_root, &dest).await.unwrap();

        // Unpack and open the snapshot cold: the team and its link —
        // committed only through the WAL — are all there.
        let unpacked = dir.path().join("restore");
        tar::Archive::new(std::fs::File::open(&dest).unwrap())
            .unpack(&unpacked)
            .unwrap();
        let conn = rusqlite::Connection::open(unpacked.join(ARCHIVE_DB_ENTRY)).unwrap();
        let teams: i64 = conn
            .query_row(
                "SELECT count(*) FROM team WHERE id = ?1",
                rusqlite::params![team_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(teams, 1);
        let links: i64 = conn
            .query_row("SELECT count(*) FROM team_blob_link", [], |row| row.get(0))
            .unwrap();
        assert_eq!(links, 1);

        driver.shutdown().await.unwrap();
    }
}
