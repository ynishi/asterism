//! `sftp://` — SSH's file transfer subsystem.
//!
//! Two crates carry it: `russh` speaks the transport and the
//! authentication, `russh-sftp` is the subsystem on one of its channels.
//!
//! # The host is checked before anything else happens
//!
//! `russh` asks its handler about the host's key while the connection is
//! being established, before a credential is sent and long before a
//! file is. [`HostCheck`] answers that question and nothing else: it
//! compares the fingerprint the profile named, or looks the host up in
//! the `known_hosts` file the profile named, and records what was
//! offered so the refusal can say both halves.
//!
//! Why the answer is recorded rather than returned as an error: the
//! handler's error type has to be one `russh` can build from its own,
//! and a mismatch is this adapter's word rather than the library's. So
//! the handler says no, the connection fails, and the reason is read
//! back out of the slot the handler wrote it into. Without that, every
//! refusal would arrive as `russh`'s generic rejection and a reader
//! could not tell a wrong key from a closed port.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use tokio::io::AsyncWriteExt;

use crate::transport::{Credentials, HostKey, Target, Transport, TransportError};

/// The port an `sftp://` endpoint means when it names none.
const DEFAULT_PORT: u16 = 22;

/// Opens an SFTP session on the host the endpoint names.
pub async fn open(
    target: &Target,
    credentials: &Credentials,
    host_key: Option<&HostKey>,
) -> Result<Box<dyn Transport>, TransportError> {
    // Refused one layer up, where the message can say what a profile is
    // missing. Reached only if that guard is edited away.
    let expected = host_key.ok_or_else(|| {
        TransportError::Refused("an sftp:// send verifies the host it is sending to".into())
    })?;
    let port = target.port.unwrap_or(DEFAULT_PORT);
    let verdict = Arc::new(Mutex::new(None));
    let handler = HostCheck {
        expected: expected.clone(),
        host: target.host.clone(),
        port,
        verdict: Arc::clone(&verdict),
    };

    let config = Arc::new(russh::client::Config::default());
    let mut session =
        match russh::client::connect(config, (target.host.as_str(), port), handler).await {
            Ok(session) => session,
            Err(err) => {
                // A refused key is what the handler wrote down; anything
                // else is the network's answer.
                let recorded = verdict.lock().unwrap().take();
                return Err(recorded.unwrap_or_else(|| {
                    TransportError::Refused(format!("reach {}:{port}: {err}", target.host))
                }));
            }
        };

    let user = credentials.user.clone().ok_or_else(|| {
        TransportError::Refused(
            "an sftp:// profile names the account in its auth block: a server that \
             takes an anonymous SSH login is not a thing"
                .into(),
        )
    })?;
    let accepted = match (&credentials.key_path, &credentials.password) {
        (Some(path), _) => {
            let key = russh::keys::load_secret_key(path, credentials.key_passphrase.as_deref())
                .map_err(|err| {
                    TransportError::Refused(format!(
                        "read the private key auth.key_ref points at: {err}"
                    ))
                })?;
            let hash = session
                .best_supported_rsa_hash()
                .await
                .map_err(|err| TransportError::Refused(format!("negotiate a signature: {err}")))?
                .flatten();
            session
                .authenticate_publickey(
                    user,
                    russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
        }
        (None, Some(password)) => session.authenticate_password(user, password.clone()).await,
        (None, None) => session.authenticate_none(user).await,
    }
    .map_err(|err| TransportError::Refused(format!("authenticate: {err}")))?;
    if !accepted.success() {
        return Err(TransportError::Refused(
            "the host would not take the credential the profile named".into(),
        ));
    }

    let channel = session
        .channel_open_session()
        .await
        .map_err(|err| TransportError::Refused(format!("open a channel: {err}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|err| TransportError::Refused(format!("start the sftp subsystem: {err}")))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|err| TransportError::Refused(format!("start the sftp subsystem: {err}")))?;

    Ok(Box::new(SftpTransport {
        _session: session,
        sftp,
        dir: target.dir.clone(),
    }))
}

/// Answers `russh`'s one question about the host, and records what it
/// saw — see the module docs.
struct HostCheck {
    expected: HostKey,
    host: String,
    port: u16,
    verdict: Arc<Mutex<Option<TransportError>>>,
}

impl russh::client::Handler for HostCheck {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = server_public_key.public_key();
        let offered = key.fingerprint(HashAlg::Sha256).to_string();
        let (accepted, why) = match &self.expected {
            HostKey::Fingerprint(expected) => (
                &offered == expected,
                TransportError::HostKeyMismatch {
                    expected: expected.clone(),
                    offered: offered.clone(),
                },
            ),
            HostKey::KnownHosts(path) => {
                let found = russh::keys::check_known_hosts_path(&self.host, self.port, &key, path);
                (
                    matches!(found, Ok(true)),
                    match found {
                        // A host the file does not mention is not a
                        // host this profile has decided about, which is
                        // the same answer as a key that changed: the
                        // send does not happen and somebody looks.
                        Ok(_) => TransportError::HostKeyMismatch {
                            expected: format!("a matching entry in {}", path.display()),
                            offered,
                        },
                        Err(err) => TransportError::Refused(format!(
                            "read {} for {}: {err}",
                            path.display(),
                            self.host
                        )),
                    },
                )
            }
        };
        if !accepted {
            *self.verdict.lock().unwrap() = Some(why);
        }
        Ok(accepted)
    }
}

/// One open SFTP session.
struct SftpTransport {
    /// Kept because dropping it closes the channel the subsystem runs
    /// on, and nothing else in this struct owns it.
    _session: russh::client::Handle<HostCheck>,
    sftp: SftpSession,
    dir: String,
}

impl SftpTransport {
    /// Where one file goes.
    fn at(&self, name: &str) -> String {
        format!("{}/{name}", self.dir.trim_end_matches('/'))
    }
}

#[async_trait]
impl Transport for SftpTransport {
    async fn ensure_dir(&mut self) -> Result<(), TransportError> {
        // The subsystem has no `create_dir_all`, so each component is
        // made in turn. One that is already there is not an error — two
        // sends land in the same place by design — so the answer that
        // matters is whether the whole path is there at the end.
        let mut walked = String::new();
        for segment in self.dir.split('/') {
            if segment.is_empty() {
                walked.push('/');
                continue;
            }
            if !walked.is_empty() && !walked.ends_with('/') {
                walked.push('/');
            }
            walked.push_str(segment);
            if !self.sftp.try_exists(walked.clone()).await.unwrap_or(false) {
                let _ = self.sftp.create_dir(walked.clone()).await;
            }
        }
        match self.sftp.try_exists(self.dir.clone()).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(TransportError::Refused(format!(
                "the directory {} is not there and could not be made",
                self.dir
            ))),
            Err(err) => Err(TransportError::Refused(format!(
                "reach {}: {err}",
                self.dir
            ))),
        }
    }

    async fn put(&mut self, name: &str, bytes: &[u8]) -> Result<(), TransportError> {
        let at = self.at(name);
        let mut file = self
            .sftp
            .create(at.clone())
            .await
            .map_err(|err| TransportError::Failed(format!("create {at}: {err}")))?;
        file.write_all(bytes)
            .await
            .map_err(|err| TransportError::Failed(format!("write {at}: {err}")))?;
        // Writes are pipelined and acknowledged behind this call, so a
        // server that refused one answers here rather than above.
        file.close()
            .await
            .map_err(|err| TransportError::Failed(format!("finish {at}: {err}")))
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.sftp
            .close()
            .await
            .map_err(|err| TransportError::Failed(format!("close the session: {err}")))
    }
}
