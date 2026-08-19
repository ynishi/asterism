//! `port::blob` — backing storage for the instance's global CAS.
//!
//! Hand-rolled trait, house style (#83 §3): `object_store` / OpenDAL
//! stay adapter details if S3 arrives — they are not the port. The
//! five verbs are the day-1 set because GC, orphan audit and backup
//! need `delete` + `list` from the start, not only `put` / `get`.
//!
//! The v0 adapter (`teams-infra`) is a local-filesystem layout with a
//! staging dir and a stream-hash → verify → fsync → rename write path;
//! none of that shows here, deliberately.

use async_trait::async_trait;

use crate::domain::store::{DeclaredDigest, VerifiedCopy};
use crate::error::DomainError;

/// Content-addressed blob storage — one physical copy per instance,
/// addressed by whole-file digest only.
///
/// `put` takes the **declared** digest and returns a [`VerifiedCopy`]:
/// the adapter hashes while writing and the domain's verification rule
/// ([`verify_declared_digest`](crate::domain::store::verify_declared_digest))
/// decides accept-or-reject, so a mismatch surfaces as
/// [`DomainError::DigestMismatch`] and no bytes land. The owned
/// `Vec<u8>` is the v0 signature; when the adapter's streaming write
/// path needs to show through the port (multi-GB promotions), a stream
/// form is added *beside* this method rather than by changing what
/// this one promises.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Writes bytes under their declared digest — verified, staged,
    /// then made visible atomically (adapter's job). Rejects the whole
    /// op on digest mismatch.
    async fn put(
        &self,
        declared: &DeclaredDigest,
        bytes: Vec<u8>,
    ) -> Result<VerifiedCopy, DomainError>;

    /// Reads a blob's bytes, or `None` when the CAS holds no such
    /// digest. Visibility (may *this caller* see it) is the link
    /// layer's question, answered before this port is reached.
    async fn get(&self, digest: &str) -> Result<Option<Vec<u8>>, DomainError>;

    /// Whether the CAS physically holds the digest. Same caveat as
    /// [`Self::get`]: physical existence, not visibility.
    async fn exists(&self, digest: &str) -> Result<bool, DomainError>;

    /// Removes a blob's bytes. Callers reach this only from the async
    /// zero-link sweep (#83 §3 lifecycle) — never directly from a user
    /// verb, which operates on links.
    async fn delete(&self, digest: &str) -> Result<(), DomainError>;

    /// Every digest the CAS physically holds — what the orphan audit
    /// and backup walk.
    async fn list(&self) -> Result<Vec<String>, DomainError>;
}
