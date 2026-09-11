//! `file://` — a directory on this machine as a destination.
//!
//! Why it exists is on [`Scheme::File`](crate::transport::Scheme::File):
//! everything above the transport trait needs a far side a test can read
//! back, and an SFTP server inside a test answers for the server rather
//! than for this adapter.
//!
//! It is a destination a profile may name, not a stub. What it does is
//! what the others do — reach the directory, put the files, put the
//! sidecar — with the filesystem as the wire, so a send to it lands the
//! same bytes under the same names in the same order.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::transport::{Target, Transport, TransportError};

/// Opens the directory the endpoint names.
pub async fn open(target: &Target) -> Result<Box<dyn Transport>, TransportError> {
    Ok(Box::new(LocalTransport {
        dir: PathBuf::from(&target.dir),
    }))
}

/// A directory being written into.
struct LocalTransport {
    dir: PathBuf,
}

#[async_trait]
impl Transport for LocalTransport {
    async fn ensure_dir(&mut self) -> Result<(), TransportError> {
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|e| TransportError::Refused(format!("reach {}: {e}", self.dir.display())))
    }

    async fn put(&mut self, name: &str, bytes: &[u8]) -> Result<(), TransportError> {
        // A name is one path segment. A profile whose
        // `remote_name_template` renders a separator would otherwise
        // write outside the directory the endpoint named, which is a
        // different destination from the one the send recorded.
        if name.contains('/') || name.contains('\\') || name == ".." {
            return Err(TransportError::Failed(format!(
                "a remote name is one path segment, and this is not: {name:?}"
            )));
        }
        tokio::fs::write(self.dir.join(name), bytes)
            .await
            .map_err(|e| TransportError::Failed(format!("put {name}: {e}")))
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{Scheme, read_endpoint};

    #[tokio::test]
    async fn it_creates_the_directory_and_writes_what_it_is_given() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("incoming").join("2026-09");
        let target = read_endpoint(&format!("file://{}", dir.display())).expect("an endpoint");
        assert_eq!(target.scheme, Scheme::File);

        let mut wire = open(&target).await.expect("open");
        wire.ensure_dir().await.expect("the directory");
        wire.put("one.png", b"bytes").await.expect("a file");
        wire.close().await.expect("close");

        assert_eq!(std::fs::read(dir.join("one.png")).unwrap(), b"bytes");
    }

    /// A name that walked out of the directory would land the bytes
    /// somewhere the send did not record.
    #[tokio::test]
    async fn a_name_that_is_not_one_segment_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = read_endpoint(&format!("file://{}", tmp.path().display())).expect("endpoint");
        let mut wire = open(&target).await.expect("open");
        wire.ensure_dir().await.expect("the directory");

        assert!(wire.put("../escaped.png", b"bytes").await.is_err());
        assert!(wire.put("nested/one.png", b"bytes").await.is_err());
    }
}
