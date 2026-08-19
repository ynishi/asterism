//! `blob` — [`LocalFileStorageAdapter`], the v0 backing of the
//! instance's global CAS (#83 §3, the #93 slice).
//!
//! ## Layout
//!
//! ```text
//! <root>/sha256/<2ch>/<64hex>   one file per digest — the CAS proper
//! <root>/staging/               in-flight writes, unique names
//! ```
//!
//! The path under `sha256/` is the digest's canonical form with the
//! `sha256:` prefix stripped — this module is the path-mapping edge #83
//! §3 names as the only place the prefix comes off. Everywhere else
//! (the port, the link table, the wire) the digest keeps its prefix.
//!
//! ## Write path
//!
//! Stream into a uniquely named staging file while hashing → verify
//! the computed digest against the **declared** one (the domain's
//! [`verify_declared_digest`], so the mismatch arm is the same
//! rejection everywhere) → `fsync` the file → rename into the final
//! path → `fsync` the parent directories. This hardens the `.part`
//! precedent from `asterism-infra`'s preview jobs: same
//! temp-then-rename shape, plus the fsyncs and the digest gate, because
//! here the rename is what makes bytes *exist* for the link layer and
//! a half-written blob must never be reachable under its digest.
//!
//! A mismatch deletes the staging file and reports the computed digest
//! (carried by [`DomainError::DigestMismatch`]); nothing lands. An
//! abandoned write (crash, dropped connection) leaves only a staging
//! temp, which the startup sweep removes — [`open`] runs it, and it is
//! the only mechanical cleanup this layer owes (#83 §3 lifecycle).
//!
//! ## Concurrent same-digest writes
//!
//! Each writer streams into its own staging file and finishes with an
//! atomic `rename` onto the same final path. `rename` replaces: the
//! last writer's inode wins, every earlier writer's file is dropped by
//! the filesystem, and since every renamed file has already been
//! verified to hash to the digest it is named by, the replacement swaps
//! bytes for identical bytes — a value-level no-op. Both callers
//! succeed and one physical copy remains. There is deliberately **no
//! exists-check shortcut** before or during the write: skipping the
//! work when the blob is already present would make a duplicate upload
//! observably cheaper, which is the Harnik-2010 side channel the
//! upload contract closes by always accepting the full body.
//!
//! [`open`]: LocalFileStorageAdapter::open
//! [`verify_declared_digest`]: teams_core::domain::store::verify_declared_digest

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use sha2::{Digest as _, Sha256};
use teams_core::DomainError;
use teams_core::domain::store::{
    DeclaredDigest, VerifiedCopy, parse_digest, verify_declared_digest,
};
use teams_core::port::blob::BlobStore;
use tokio::io::AsyncWriteExt as _;
use uuid::Uuid;

/// The CAS directory under the root — named for the one algorithm the
/// store admits, so the layout stays self-describing if another ever
/// arrives beside it.
const CAS_DIR: &str = "sha256";
/// Where in-flight writes live until their rename.
const STAGING_DIR: &str = "staging";

/// Local-filesystem CAS adapter — one physical copy per instance,
/// filed by digest (#83 §3).
///
/// Implements the [`BlobStore`] port (whole-buffer `put`) and exposes
/// the streaming write path ([`begin_put`]) and the streaming read
/// handle ([`open_blob`]) as inherent methods: the port's owned-`Vec`
/// signature is the v0 contract, and the stream forms live here beside
/// it until a second adapter needs them to show through the port —
/// exactly the extension order the port's doc reserves.
///
/// [`begin_put`]: Self::begin_put
/// [`open_blob`]: Self::open_blob
#[derive(Clone, Debug)]
pub struct LocalFileStorageAdapter {
    cas_root: PathBuf,
    staging_root: PathBuf,
}

impl LocalFileStorageAdapter {
    /// Opens (creating on demand) the blob store rooted at `root`, and
    /// runs the startup sweep of `staging/`: temp files from
    /// interrupted copies are deleted, and nothing else is touched —
    /// live blobs live under `sha256/`, which the sweep never enters.
    pub async fn open(root: impl Into<PathBuf>) -> Result<Self, DomainError> {
        let root = root.into();
        let adapter = Self {
            cas_root: root.join(CAS_DIR),
            staging_root: root.join(STAGING_DIR),
        };
        for dir in [&adapter.cas_root, &adapter.staging_root] {
            tokio::fs::create_dir_all(dir).await.map_err(|e| {
                DomainError::Infra(anyhow::anyhow!(
                    "cannot create blob directory {}: {e}",
                    dir.display()
                ))
            })?;
        }
        adapter.sweep_staging().await?;
        Ok(adapter)
    }

    /// Deletes every entry of `staging/`. Interrupted copies are the
    /// only thing that legitimately ends up there, so anything present
    /// at startup is garbage by definition; an entry that is not a
    /// plain file means something else is using the directory, which
    /// is refused loudly rather than swept around.
    async fn sweep_staging(&self) -> Result<(), DomainError> {
        let mut entries = tokio::fs::read_dir(&self.staging_root).await.map_err(|e| {
            DomainError::Infra(anyhow::anyhow!(
                "cannot sweep staging {}: {e}",
                self.staging_root.display()
            ))
        })?;
        loop {
            let entry = entries.next_entry().await.map_err(|e| {
                DomainError::Infra(anyhow::anyhow!(
                    "cannot sweep staging {}: {e}",
                    self.staging_root.display()
                ))
            })?;
            let Some(entry) = entry else { break };
            let path = entry.path();
            tokio::fs::remove_file(&path).await.map_err(|e| {
                DomainError::Infra(anyhow::anyhow!(
                    "staging sweep cannot remove {}: {e}",
                    path.display()
                ))
            })?;
        }
        Ok(())
    }

    /// Starts a streaming write: a uniquely named staging file plus a
    /// running hasher. Feed it with [`StagingWrite::write_chunk`],
    /// finish with [`StagingWrite::commit`]; dropping it un-committed
    /// removes the staging file best-effort (the startup sweep is the
    /// backstop for the crash case where no destructor runs).
    pub async fn begin_put(&self) -> Result<StagingWrite, DomainError> {
        let temp_path = self.staging_root.join(format!("put-{}", Uuid::now_v7()));
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .await
            .map_err(|e| {
                DomainError::Infra(anyhow::anyhow!(
                    "cannot open staging file {}: {e}",
                    temp_path.display()
                ))
            })?;
        Ok(StagingWrite {
            cas_root: self.cas_root.clone(),
            temp_path,
            file: Some(file),
            hasher: Sha256::new(),
            defused: false,
        })
    }

    /// Opens a blob for streaming, returning the handle and its length
    /// (taken from the open handle, so it matches the bytes a caller
    /// will read), or `None` when the CAS holds no such digest. Same
    /// caveat as the port's `get`: physical existence, not visibility.
    pub async fn open_blob(
        &self,
        digest: &str,
    ) -> Result<Option<(tokio::fs::File, u64)>, DomainError> {
        let path = self.blob_path(digest)?;
        let file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(DomainError::Infra(anyhow::anyhow!(
                    "blob {digest} is unreadable at {}: {e}",
                    path.display()
                )));
            }
        };
        let meta = file.metadata().await.map_err(|e| {
            DomainError::Infra(anyhow::anyhow!(
                "blob {digest} is unreadable at {}: {e}",
                path.display()
            ))
        })?;
        if !meta.is_file() {
            return Err(DomainError::Infra(anyhow::anyhow!(
                "blob path {} is not a regular file",
                path.display()
            )));
        }
        Ok(Some((file, meta.len())))
    }

    /// Maps a digest to its file — **the** path-mapping edge (#83 §3):
    /// the digest is validated through the domain's parser, then the
    /// algorithm tag comes off and the hex names the file under a
    /// two-character shard directory.
    fn blob_path(&self, digest: &str) -> Result<PathBuf, DomainError> {
        let canonical = parse_digest(digest)?;
        let (_tag, hex) = canonical
            .split_once(':')
            .expect("a canonical digest carries its algorithm tag");
        Ok(self.cas_root.join(&hex[..2]).join(hex))
    }
}

#[async_trait]
impl BlobStore for LocalFileStorageAdapter {
    async fn put(
        &self,
        declared: &DeclaredDigest,
        bytes: Vec<u8>,
    ) -> Result<VerifiedCopy, DomainError> {
        let mut staged = self.begin_put().await?;
        staged.write_chunk(&bytes).await?;
        staged.commit(declared).await
    }

    async fn get(&self, digest: &str) -> Result<Option<Vec<u8>>, DomainError> {
        let path = self.blob_path(digest)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(DomainError::Infra(anyhow::anyhow!(
                "cannot read blob {digest} at {}: {e}",
                path.display()
            ))),
        }
    }

    async fn exists(&self, digest: &str) -> Result<bool, DomainError> {
        let path = self.blob_path(digest)?;
        match tokio::fs::metadata(&path).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(DomainError::Infra(anyhow::anyhow!(
                "cannot stat blob {digest} at {}: {e}",
                path.display()
            ))),
        }
    }

    async fn delete(&self, digest: &str) -> Result<(), DomainError> {
        let path = self.blob_path(digest)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            // Idempotent: the caller is the async zero-link sweep, and
            // "these bytes are gone" is already true — the same
            // reasoning as logout's.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(DomainError::Infra(anyhow::anyhow!(
                "cannot delete blob {digest} at {}: {e}",
                path.display()
            ))),
        }
    }

    async fn list(&self) -> Result<Vec<String>, DomainError> {
        let mut digests = Vec::new();
        let mut shards = read_dir(&self.cas_root).await?;
        while let Some(shard) = next_entry(&mut shards, &self.cas_root).await? {
            let shard_path = shard.path();
            let shard_name = shard.file_name();
            let mut files = read_dir(&shard_path).await?;
            while let Some(file) = next_entry(&mut files, &shard_path).await? {
                let name = file.file_name();
                let name = name.to_string_lossy();
                // Reconstruct the canonical form and pass it back
                // through the domain's parser — a stray file is
                // refused loudly, never guessed into a digest (the
                // same discipline as stored role text on read).
                let canonical = parse_digest(&format!("{CAS_DIR}:{name}")).map_err(|_| {
                    DomainError::Infra(anyhow::anyhow!(
                        "foreign file in the CAS: {}",
                        file.path().display()
                    ))
                })?;
                let shard = shard_name.to_string_lossy();
                if !name.starts_with(shard.as_ref()) {
                    return Err(DomainError::Infra(anyhow::anyhow!(
                        "misfiled blob: {} sits in shard {shard}",
                        file.path().display()
                    )));
                }
                digests.push(canonical);
            }
        }
        digests.sort();
        Ok(digests)
    }
}

/// One in-flight streaming write: the staging file, the running hash,
/// and the promise that nothing becomes visible until [`commit`]'s
/// rename.
///
/// Dropping this without committing removes the staging file
/// (best-effort, synchronously — the startup sweep catches whatever a
/// crash leaves behind).
///
/// [`commit`]: Self::commit
pub struct StagingWrite {
    cas_root: PathBuf,
    temp_path: PathBuf,
    file: Option<tokio::fs::File>,
    hasher: Sha256,
    defused: bool,
}

impl StagingWrite {
    /// Appends a chunk: hashed and written in one motion, so the
    /// digest verified at commit time is over exactly the bytes on
    /// disk.
    pub async fn write_chunk(&mut self, chunk: &[u8]) -> Result<(), DomainError> {
        self.hasher.update(chunk);
        self.file
            .as_mut()
            .expect("write_chunk after commit is unreachable: commit consumes self")
            .write_all(chunk)
            .await
            .map_err(|e| {
                DomainError::Infra(anyhow::anyhow!(
                    "cannot write staging file {}: {e}",
                    self.temp_path.display()
                ))
            })
    }

    /// Finishes the write: verify → fsync → rename → fsync parents.
    ///
    /// The computed digest is spelled `sha256:` + hex here — the
    /// hasher side of the shared notation; running it through
    /// [`verify_declared_digest`] re-parses it, which is what keeps
    /// this spelling honest against the domain's grammar. On mismatch
    /// the staging file is deleted and the error carries both sides;
    /// nothing lands (#83 §3: reject the whole op, never
    /// accept-new-digest).
    ///
    /// Both the shard directory and the CAS root are fsynced after the
    /// rename: the first makes the file's directory entry durable, the
    /// second the shard directory's own (it may have been created by
    /// this very write). An orphan blob after a crash is harmless; a
    /// blob that vanishes after its link row committed would be a
    /// dangling link, which #83 §3's ordering promises never happens.
    pub async fn commit(mut self, declared: &DeclaredDigest) -> Result<VerifiedCopy, DomainError> {
        let file = self
            .file
            .take()
            .expect("commit consumes self, so the handle is present");
        let computed = format!(
            "{CAS_DIR}:{:x}",
            std::mem::take(&mut self.hasher).finalize()
        );
        let verified = match verify_declared_digest(declared, &computed) {
            Ok(verified) => verified,
            Err(refused) => {
                drop(file);
                tokio::fs::remove_file(&self.temp_path).await.map_err(|e| {
                    DomainError::Infra(anyhow::anyhow!(
                        "digest mismatch, and the staging file {} also failed to delete: {e}",
                        self.temp_path.display()
                    ))
                })?;
                self.defused = true;
                return Err(refused);
            }
        };
        file.sync_all().await.map_err(|e| {
            DomainError::Infra(anyhow::anyhow!(
                "cannot fsync staging file {}: {e}",
                self.temp_path.display()
            ))
        })?;
        drop(file);
        let (_tag, hex) = verified
            .digest()
            .split_once(':')
            .expect("a verified digest carries its algorithm tag");
        let shard_dir = self.cas_root.join(&hex[..2]);
        tokio::fs::create_dir_all(&shard_dir).await.map_err(|e| {
            DomainError::Infra(anyhow::anyhow!(
                "cannot create shard directory {}: {e}",
                shard_dir.display()
            ))
        })?;
        let final_path = shard_dir.join(hex);
        tokio::fs::rename(&self.temp_path, &final_path)
            .await
            .map_err(|e| {
                DomainError::Infra(anyhow::anyhow!(
                    "cannot rename {} into place: {e}",
                    self.temp_path.display()
                ))
            })?;
        self.defused = true;
        fsync_dir(&shard_dir).await?;
        fsync_dir(&self.cas_root).await?;
        Ok(verified)
    }
}

impl Drop for StagingWrite {
    fn drop(&mut self) {
        if !self.defused {
            // Best-effort, synchronous: an abandoned write cleans up
            // after itself when it can, and the startup sweep answers
            // for the cases where no destructor ran at all.
            let _ = std::fs::remove_file(&self.temp_path);
        }
    }
}

/// Fsyncs a directory — what makes a completed rename's directory
/// entry durable, per the write path in the module doc.
async fn fsync_dir(path: &Path) -> Result<(), DomainError> {
    let dir = tokio::fs::File::open(path).await.map_err(|e| {
        DomainError::Infra(anyhow::anyhow!(
            "cannot open directory {} to fsync it: {e}",
            path.display()
        ))
    })?;
    dir.sync_all().await.map_err(|e| {
        DomainError::Infra(anyhow::anyhow!(
            "cannot fsync directory {}: {e}",
            path.display()
        ))
    })
}

async fn read_dir(path: &Path) -> Result<tokio::fs::ReadDir, DomainError> {
    tokio::fs::read_dir(path).await.map_err(|e| {
        DomainError::Infra(anyhow::anyhow!(
            "cannot list directory {}: {e}",
            path.display()
        ))
    })
}

async fn next_entry(
    entries: &mut tokio::fs::ReadDir,
    path: &Path,
) -> Result<Option<tokio::fs::DirEntry>, DomainError> {
    entries.next_entry().await.map_err(|e| {
        DomainError::Infra(anyhow::anyhow!(
            "cannot list directory {}: {e}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared notation, spelled by the test's own hasher — the
    /// same justification as the repository tests' literal digests:
    /// this crate deliberately has no asterism-* edge (#83 §4), and
    /// every one of these strings still passes through the domain's
    /// parser inside the adapter.
    fn digest_of(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    fn declared_for(bytes: &[u8]) -> DeclaredDigest {
        DeclaredDigest::parse(&digest_of(bytes)).unwrap()
    }

    async fn adapter() -> (LocalFileStorageAdapter, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let adapter = LocalFileStorageAdapter::open(dir.path().join("blobs"))
            .await
            .unwrap();
        (adapter, dir)
    }

    async fn staging_entries(adapter: &LocalFileStorageAdapter) -> usize {
        let mut n = 0;
        let mut entries = tokio::fs::read_dir(&adapter.staging_root).await.unwrap();
        while entries.next_entry().await.unwrap().is_some() {
            n += 1;
        }
        n
    }

    #[tokio::test]
    async fn a_write_round_trips_through_the_port() {
        let (adapter, _dir) = adapter().await;
        let bytes = b"the artefact's bytes".to_vec();
        let digest = digest_of(&bytes);

        let verified = adapter
            .put(&declared_for(&bytes), bytes.clone())
            .await
            .unwrap();
        assert_eq!(verified.digest(), digest);

        assert!(adapter.exists(&digest).await.unwrap());
        assert_eq!(adapter.get(&digest).await.unwrap(), Some(bytes));
        assert_eq!(adapter.list().await.unwrap(), vec![digest.clone()]);
        assert_eq!(staging_entries(&adapter).await, 0);

        // The layout is the documented one: sha256/<2ch>/<64hex>.
        let hex = digest.strip_prefix("sha256:").unwrap();
        assert!(
            adapter.cas_root.join(&hex[..2]).join(hex).is_file(),
            "the blob must sit at sha256/{}/{hex}",
            &hex[..2]
        );
    }

    #[tokio::test]
    async fn nothing_is_visible_before_commit_and_an_abandoned_write_leaves_only_staging() {
        let (adapter, _dir) = adapter().await;
        let bytes = b"interrupted";
        let digest = digest_of(bytes);

        let mut staged = adapter.begin_put().await.unwrap();
        staged.write_chunk(bytes).await.unwrap();

        // Interruption point: bytes written, commit never reached. The
        // CAS shows nothing — not a partial file, not an entry.
        assert!(!adapter.exists(&digest).await.unwrap());
        assert!(adapter.list().await.unwrap().is_empty());
        assert_eq!(staging_entries(&adapter).await, 1);

        // The abandoned write cleans its temp on drop…
        drop(staged);
        assert_eq!(staging_entries(&adapter).await, 0);
        assert!(!adapter.exists(&digest).await.unwrap());
    }

    #[tokio::test]
    async fn a_mismatch_leaves_no_blob_and_no_staging_residue() {
        let (adapter, _dir) = adapter().await;
        let declared = declared_for(b"what the user chose");
        let arriving = b"what the path held at upload";

        let mut staged = adapter.begin_put().await.unwrap();
        staged.write_chunk(arriving).await.unwrap();
        let refused = staged.commit(&declared).await;

        match refused {
            Err(DomainError::DigestMismatch {
                declared: d,
                computed,
            }) => {
                assert_eq!(d, declared.as_str());
                assert_eq!(
                    computed,
                    digest_of(arriving),
                    "the computed side is reported"
                );
            }
            other => panic!("expected DigestMismatch, got {other:?}"),
        }
        assert_eq!(staging_entries(&adapter).await, 0, "no staging residue");
        assert!(adapter.list().await.unwrap().is_empty(), "no blob");
        assert!(!adapter.exists(&digest_of(arriving)).await.unwrap());
    }

    #[tokio::test]
    async fn the_startup_sweep_clears_stale_temps_and_spares_live_blobs() {
        let (adapter, dir) = adapter().await;
        let bytes = b"live blob".to_vec();
        let digest = digest_of(&bytes);
        adapter
            .put(&declared_for(&bytes), bytes.clone())
            .await
            .unwrap();

        // A stale temp, as an interrupted copy (no destructor) leaves.
        let stale = adapter.staging_root.join("put-stale");
        tokio::fs::write(&stale, b"half a blob").await.unwrap();

        // Re-open = restart. The temp goes, the live blob stays.
        let reopened = LocalFileStorageAdapter::open(dir.path().join("blobs"))
            .await
            .unwrap();
        assert_eq!(staging_entries(&reopened).await, 0);
        assert!(!stale.exists());
        assert_eq!(reopened.get(&digest).await.unwrap(), Some(bytes));
    }

    #[tokio::test]
    async fn concurrent_same_digest_writes_converge_on_one_copy_and_both_succeed() {
        let (adapter, _dir) = adapter().await;
        let bytes = b"promoted twice at once".to_vec();
        let digest = digest_of(&bytes);
        let declared = declared_for(&bytes);

        // Two full streaming writes, interleaved chunk by chunk so
        // both staging files are open at once and the renames race.
        let write = |adapter: LocalFileStorageAdapter, declared: DeclaredDigest| {
            let bytes = bytes.clone();
            async move {
                let mut staged = adapter.begin_put().await?;
                for chunk in bytes.chunks(4) {
                    staged.write_chunk(chunk).await?;
                    tokio::task::yield_now().await;
                }
                staged.commit(&declared).await
            }
        };
        let (a, b) = tokio::join!(
            write(adapter.clone(), declared.clone()),
            write(adapter.clone(), declared.clone())
        );
        assert_eq!(a.unwrap().digest(), digest, "first caller succeeds");
        assert_eq!(b.unwrap().digest(), digest, "second caller succeeds");

        assert_eq!(
            adapter.list().await.unwrap(),
            vec![digest.clone()],
            "one physical copy"
        );
        assert_eq!(adapter.get(&digest).await.unwrap(), Some(bytes));
        assert_eq!(staging_entries(&adapter).await, 0);
    }

    #[tokio::test]
    async fn delete_is_idempotent_and_list_refuses_foreign_files() {
        let (adapter, _dir) = adapter().await;
        let bytes = b"short-lived".to_vec();
        let digest = digest_of(&bytes);
        adapter
            .put(&declared_for(&bytes), bytes.clone())
            .await
            .unwrap();

        adapter.delete(&digest).await.unwrap();
        assert!(!adapter.exists(&digest).await.unwrap());
        adapter.delete(&digest).await.unwrap(); // second delete: same truth

        // A file the layout does not admit is refused, not guessed at.
        let shard = adapter.cas_root.join("ab");
        tokio::fs::create_dir_all(&shard).await.unwrap();
        tokio::fs::write(shard.join("notes.txt"), b"?")
            .await
            .unwrap();
        let listed = adapter.list().await;
        assert!(
            matches!(&listed, Err(DomainError::Infra(e)) if e.to_string().contains("foreign file")),
            "a stray file must fail the walk loudly: {listed:?}"
        );
    }

    #[tokio::test]
    async fn the_adapter_speaks_only_the_canonical_notation() {
        let (adapter, _dir) = adapter().await;
        for wrong in [
            "a".repeat(64),                           // bare hex
            format!("cr1-sha256:{}", "a".repeat(64)), // content-region axis
            "sha256:short".to_string(),
        ] {
            assert!(
                matches!(
                    adapter.exists(&wrong).await,
                    Err(DomainError::Validation(_))
                ),
                "{wrong:?} must be refused at the path-mapping edge"
            );
        }
    }
}
