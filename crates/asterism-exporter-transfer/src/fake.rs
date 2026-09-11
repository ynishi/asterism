//! A far side held in memory, for testing what this adapter decides.
//!
//! Everything above [`Transport`] is protocol-independent — which files
//! go, under what names, in what order, what the sidecar says, what the
//! attempt record ends up holding — and all of it is exercised against
//! this. A real server in those tests would answer for the server.
//!
//! Compiled under `#[cfg(test)]`, so it is this crate's own and not
//! something the workspace ships. What a caller in another crate gets
//! instead is [`Connector`], which is the seam this implements and
//! which [`TransferExporter::with_connector`](crate::TransferExporter::with_connector)
//! takes — a far side somebody else owns goes in there rather than
//! being borrowed from here.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::transport::{Connector, Credentials, HostKey, Target, Transport, TransportError};

/// What was put, in the order it was put.
#[derive(Debug, Default)]
pub struct Fake {
    /// Every file by the name it was put under.
    pub files: Mutex<BTreeMap<String, Vec<u8>>>,
    /// The names in the order they arrived, so put order is readable.
    pub order: Mutex<Vec<String>>,
    /// Whether the directory was reached before anything was put.
    pub directory_made: Mutex<bool>,
    /// Names this far side refuses, and what it says about each.
    pub refuse: Mutex<BTreeMap<String, String>>,
}

impl Fake {
    /// A far side that takes everything.
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Makes this far side answer no to one name.
    pub fn refusing(self: &Arc<Self>, name: &str, answer: &str) {
        self.refuse
            .lock()
            .unwrap()
            .insert(name.to_string(), answer.to_string());
    }

    /// What was put under `name`.
    pub fn file(&self, name: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap().get(name).cloned()
    }

    /// The names in the order they were put.
    pub fn order(&self) -> Vec<String> {
        self.order.lock().unwrap().clone()
    }
}

/// One open connection to a [`Fake`].
pub struct FakeTransport {
    far: Arc<Fake>,
}

#[async_trait]
impl Transport for FakeTransport {
    async fn ensure_dir(&mut self) -> Result<(), TransportError> {
        *self.far.directory_made.lock().unwrap() = true;
        Ok(())
    }

    async fn put(&mut self, name: &str, bytes: &[u8]) -> Result<(), TransportError> {
        if let Some(answer) = self.far.refuse.lock().unwrap().get(name) {
            return Err(TransportError::Failed(answer.clone()));
        }
        self.far
            .files
            .lock()
            .unwrap()
            .insert(name.to_string(), bytes.to_vec());
        self.far.order.lock().unwrap().push(name.to_string());
        Ok(())
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}

/// What opens a connection to a [`Fake`], or refuses to.
pub struct FakeConnector {
    far: Arc<Fake>,
    /// The fingerprint this far side offers, so a profile naming
    /// another one is refused the way a real host's would be.
    offers: Option<String>,
}

impl FakeConnector {
    /// A far side that accepts whatever host key it is asked for.
    pub fn accepting(far: Arc<Fake>) -> Arc<Self> {
        Arc::new(Self { far, offers: None })
    }

    /// A far side that offers this fingerprint and nothing else.
    pub fn offering(far: Arc<Fake>, fingerprint: &str) -> Arc<Self> {
        Arc::new(Self {
            far,
            offers: Some(fingerprint.to_string()),
        })
    }
}

#[async_trait]
impl Connector for FakeConnector {
    async fn open(
        &self,
        _target: &Target,
        _credentials: &Credentials,
        host_key: Option<&HostKey>,
    ) -> Result<Box<dyn Transport>, TransportError> {
        if let (Some(offered), Some(HostKey::Fingerprint(expected))) = (&self.offers, host_key)
            && offered != expected
        {
            return Err(TransportError::HostKeyMismatch {
                expected: expected.clone(),
                offered: offered.clone(),
            });
        }
        Ok(Box::new(FakeTransport {
            far: Arc::clone(&self.far),
        }))
    }
}
