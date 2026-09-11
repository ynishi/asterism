//! The port between the exporter and the wire.
//!
//! One trait with three verbs — reach the directory, put a file in it,
//! close — and one trait that opens a connection to reach it with.
//! Everything above this line is the same whichever protocol carries the
//! bytes: which files go, under what names, in what order, what the
//! sidecar says and what the attempt record ends up holding.
//!
//! # Why the split is here and not at the exporter
//!
//! A protocol needs a server to talk to, and an SFTP server in a unit
//! test is a second implementation of the thing under test. Behind this
//! trait the exporter's own decisions are exercised against a far side
//! held in memory, which is a map of what it was asked to put; the
//! protocol implementations answer for the protocol and nothing else.
//!
//! # What a refusal is, and what a failure is
//!
//! [`TransportError::Refused`] is the answer before any byte moved: a
//! scheme the profile did not opt into, a host whose key is not the one
//! named, a credential the server would not take.
//! [`TransportError::Failed`] is what the far end said about something
//! that was attempted. The exporter records both, and the difference is
//! what a reader needs: a refusal means nothing arrived, a failure means
//! some of it may have.

use std::path::PathBuf;

use async_trait::async_trait;

/// Where a send is going, as the profile's endpoint describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Which protocol carries the bytes.
    pub scheme: Scheme,
    /// Host name, without the port.
    pub host: String,
    /// Port, when the endpoint named one. Each protocol's default is
    /// the protocol's, not this type's.
    pub port: Option<u16>,
    /// The directory on the far side, as the endpoint's path. Never
    /// empty: an endpoint with no path lands in the far side's own
    /// default directory, which is spelled `"."`.
    pub dir: String,
}

/// The protocols this adapter speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// SSH's file transfer subsystem. Verifies the host — see
    /// [`HostKey`].
    Sftp,
    /// FTP with TLS negotiated on the control connection before the
    /// credential is sent.
    Ftps,
    /// FTP in the clear. Refused unless the profile opted in.
    Ftp,
    /// A directory on this machine.
    ///
    /// Not a network protocol and not a substitute for one. It is here
    /// because the acceptance tests for everything above this trait —
    /// put order, remote names, the sidecar, what the attempt record
    /// says — need a far side that can be read back, and running them
    /// against a real server would answer for that server instead. A
    /// profile may name it, and what it gets is a copy on the same
    /// filesystem.
    File,
}

impl Scheme {
    /// The scheme as an endpoint spells it.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sftp => "sftp",
            Self::Ftps => "ftps",
            Self::Ftp => "ftp",
            Self::File => "file",
        }
    }
}

/// What the far side is asked to accept as proof of who is calling.
///
/// Values, resolved per call from the environment variables the profile
/// named. Nothing here is ever written to a row: the profile carries the
/// names and the attempt record repeats the names, and
/// [`Redaction`](asterism_exporter_common::Redaction) takes the values
/// back out of anything an implementation composed from them.
#[derive(Debug, Clone, Default)]
pub struct Credentials {
    /// The account name on the far side.
    pub user: Option<String>,
    /// The password, for a server that takes one.
    pub password: Option<String>,
    /// Where the private key is, for a server that takes one.
    pub key_path: Option<PathBuf>,
    /// The passphrase that key is encrypted with, when it is.
    pub key_passphrase: Option<String>,
}

impl Credentials {
    /// Every secret value this call was made with, for the scrub.
    pub fn secrets(&self) -> Vec<String> {
        [self.password.clone(), self.key_passphrase.clone()]
            .into_iter()
            .flatten()
            .collect()
    }
}

/// How the far side's identity is checked.
///
/// There is no third state. An SFTP endpoint with neither is refused
/// before the socket opens: an unverified host key is a credential
/// handed to whoever answered, and a prompt is not something a
/// background dispatch can put in front of anybody.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKey {
    /// The fingerprint the profile expects, as OpenSSH spells one:
    /// `SHA256:` followed by the unpadded base64 of the digest.
    Fingerprint(String),
    /// A `known_hosts` file to look the host up in.
    KnownHosts(PathBuf),
}

/// What went wrong.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Nothing was sent, and this is why.
    #[error("{0}")]
    Refused(String),
    /// The host offered a key the profile does not name.
    ///
    /// Its own variant rather than a [`Refused`](Self::Refused) with a
    /// message, so the sentence a reader of an attempt record sees is
    /// composed once and says both halves.
    #[error(
        "the host offered a key the profile does not name: expected {expected}, \
         offered {offered}"
    )]
    HostKeyMismatch {
        /// What the profile said the host's key would be.
        expected: String,
        /// What answered.
        offered: String,
    },
    /// Something was attempted and the far side said no.
    #[error("{0}")]
    Failed(String),
}

/// An open connection to one destination.
#[async_trait]
pub trait Transport: core::marker::Send {
    /// Makes sure the send's directory is there, creating it when it is
    /// not.
    ///
    /// A directory that already exists is not an error: two sends of one
    /// release land in the same place by design, and so do two releases
    /// a profile points at one directory.
    async fn ensure_dir(&mut self) -> Result<(), TransportError>;

    /// Puts one file in that directory under `name`.
    async fn put(&mut self, name: &str, bytes: &[u8]) -> Result<(), TransportError>;

    /// Says goodbye to the far side.
    ///
    /// Separate from `Drop` because a protocol's goodbye is a message on
    /// the wire and sending one takes an `await`. A connection dropped
    /// without it is closed by the far side's own timeout, so this is
    /// tidiness rather than correctness.
    async fn close(&mut self) -> Result<(), TransportError>;
}

/// What opens one.
///
/// A trait rather than a function so that the exporter can be built
/// against a far side a test owns — see the module docs.
#[async_trait]
pub trait Connector: core::marker::Send + Sync {
    /// Opens a connection to `target`, refusing before any byte moves
    /// when the host is not the one `host_key` names.
    async fn open(
        &self,
        target: &Target,
        credentials: &Credentials,
        host_key: Option<&HostKey>,
    ) -> Result<Box<dyn Transport>, TransportError>;
}

/// Reads a profile's endpoint into a [`Target`].
///
/// The grammar is a URL's, narrowed to what a destination needs:
/// `<scheme>://[host][:port][/dir]`. Credentials in the authority are
/// refused rather than parsed — a password in an endpoint would be
/// written into the params blob, which is persisted unedited and handed
/// back on every read of the dispatch, and the profile's `auth` block is
/// the way that does not do that.
pub fn read_endpoint(endpoint: &str) -> Result<Target, TransportError> {
    let (scheme, rest) = endpoint.split_once("://").ok_or_else(|| {
        TransportError::Refused(format!(
            "an endpoint is <scheme>://<host>[:<port>][/<dir>], and this names no \
             scheme: {endpoint:?}"
        ))
    })?;
    let scheme = match scheme {
        "sftp" => Scheme::Sftp,
        "ftps" => Scheme::Ftps,
        "ftp" => Scheme::Ftp,
        "file" => Scheme::File,
        other => {
            return Err(TransportError::Refused(format!(
                "this adapter carries sftp, ftps, ftp and file, and not {other:?}"
            )));
        }
    };
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    if authority.contains('@') {
        return Err(TransportError::Refused(
            "an endpoint carries no credential: the params blob it sits in is \
             persisted unedited and handed back on every read of the dispatch, so \
             the account goes in the profile's auth block, which names environment \
             variables"
                .into(),
        ));
    }
    // A `file://` endpoint's path is the whole of it, and an absolute
    // path starts at the third slash — so its authority is empty and
    // anything else there is a host this scheme cannot reach.
    if scheme == Scheme::File {
        if !authority.is_empty() {
            return Err(TransportError::Refused(format!(
                "a file:// endpoint names no host, and this names {authority:?}"
            )));
        }
        if path.is_empty() {
            return Err(TransportError::Refused(
                "a file:// endpoint is file:// followed by an absolute directory".into(),
            ));
        }
        return Ok(Target {
            scheme,
            host: String::new(),
            port: None,
            dir: path.to_string(),
        });
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => {
            let parsed = port
                .parse::<u16>()
                .map_err(|_| TransportError::Refused(format!("{port:?} is not a port number")))?;
            (host, Some(parsed))
        }
        None => (authority, None),
    };
    if host.is_empty() {
        return Err(TransportError::Refused(format!(
            "an endpoint names a host: {endpoint:?}"
        )));
    }
    Ok(Target {
        scheme,
        host: host.to_string(),
        port,
        // An endpoint with no path lands wherever the account lands,
        // which every one of these protocols spells as the working
        // directory it opens in.
        dir: if path.is_empty() {
            ".".to_string()
        } else {
            path.to_string()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_splits_into_scheme_host_port_and_directory() {
        let target = read_endpoint("sftp://stock.example.com:2222/incoming/2026-09").unwrap();

        assert_eq!(target.scheme, Scheme::Sftp);
        assert_eq!(target.host, "stock.example.com");
        assert_eq!(target.port, Some(2222));
        assert_eq!(target.dir, "/incoming/2026-09");
    }

    /// The port and the path are each the protocol's business when the
    /// endpoint leaves them out, so what lands here is the absence
    /// rather than a guess.
    #[test]
    fn an_endpoint_may_name_neither_port_nor_directory() {
        let target = read_endpoint("ftps://files.example.com").unwrap();

        assert_eq!(target.port, None);
        assert_eq!(target.dir, ".");
    }

    #[test]
    fn a_file_endpoint_is_a_directory_and_no_host() {
        let target = read_endpoint("file:///tmp/outbound").unwrap();

        assert_eq!(target.scheme, Scheme::File);
        assert_eq!(target.dir, "/tmp/outbound");
        assert!(target.host.is_empty());
        assert!(read_endpoint("file://elsewhere/tmp").is_err());
    }

    /// A credential in the endpoint would be written into the params
    /// blob — see [`read_endpoint`].
    #[test]
    fn an_endpoint_carrying_an_account_is_refused() {
        let refused = read_endpoint("ftp://user:secret@files.example.com/in");

        assert!(matches!(refused, Err(TransportError::Refused(_))));
    }

    #[test]
    fn a_scheme_this_adapter_does_not_carry_is_refused() {
        assert!(read_endpoint("https://files.example.com/in").is_err());
        assert!(read_endpoint("files.example.com/in").is_err());
        assert!(read_endpoint("sftp://:22/in").is_err());
        assert!(read_endpoint("sftp://host:nope/in").is_err());
    }

    #[test]
    fn every_scheme_spells_itself_the_way_an_endpoint_does() {
        for scheme in [Scheme::Sftp, Scheme::Ftps, Scheme::Ftp, Scheme::File] {
            let endpoint = match scheme {
                Scheme::File => "file:///tmp/out".to_string(),
                other => format!("{}://host/dir", other.as_str()),
            };
            assert_eq!(read_endpoint(&endpoint).unwrap().scheme, scheme);
        }
    }
}
