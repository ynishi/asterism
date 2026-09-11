//! `ftps://` and `ftp://` — one client, two answers about TLS.
//!
//! `suppaftp` fixes the TLS stream as a type parameter at construction,
//! so a plain connection cannot be upgraded later and the two are
//! different types all the way down. [`FtpWire`] is that pair, and every
//! verb below matches on it.
//!
//! # What each scheme costs
//!
//! `ftps://` negotiates TLS on the control connection before the
//! credential is sent and asks for the data connections to be protected
//! too, so the files are encrypted as well as the login. `ftp://`
//! protects neither, which is why nothing reaches this module under that
//! scheme unless the profile said so — the refusal is in
//! `crate::check_scheme`, where the message can name the field.
//!
//! # Roots
//!
//! The Mozilla set, compiled in. The alternative is the platform's own
//! store, which would make what a send trusts a property of the machine
//! it ran on; this workspace already made that choice for its HTTP
//! client, and this is the same choice rather than a second one.

use std::sync::Arc;

use async_trait::async_trait;
use suppaftp::tokio::{
    AsyncFtpStream, AsyncNoTlsStream, AsyncRustlsConnector, AsyncRustlsFtpStream,
    AsyncRustlsStream, ImplAsyncFtpStream,
};
use suppaftp::tokio_rustls::TlsConnector;
use suppaftp::tokio_rustls::rustls::{ClientConfig, RootCertStore};
use suppaftp::types::FileType;
use tokio::io::AsyncWriteExt;

use crate::transport::{Credentials, Scheme, Target, Transport, TransportError};

/// The port an FTP endpoint means when it names none. The same number
/// for both schemes: `ftps://` here is explicit FTPS, which starts on
/// the control port and negotiates upward.
const DEFAULT_PORT: u16 = 21;

/// What an anonymous login sends, by the convention every FTP server
/// offering one expects.
const ANONYMOUS: (&str, &str) = ("anonymous", "anonymous@");

/// Runs the same call on whichever half of [`FtpWire`] is open.
///
/// A macro rather than a trait object: the two are one generic type at
/// two parameters and the methods are inherent, so a trait would have to
/// restate every signature this module uses in order to name four of
/// them.
macro_rules! with_wire {
    ($wire:expr, $session:ident => $call:expr) => {
        match &mut $wire {
            FtpWire::Plain($session) => $call,
            FtpWire::Secure($session) => $call,
        }
    };
}

/// Opens an FTP or FTPS session on the host the endpoint names.
pub async fn open(
    target: &Target,
    credentials: &Credentials,
) -> Result<Box<dyn Transport>, TransportError> {
    let port = target.port.unwrap_or(DEFAULT_PORT);
    let address = format!("{}:{port}", target.host);
    let mut wire = match target.scheme {
        Scheme::Ftps => FtpWire::Secure(
            AsyncRustlsFtpStream::connect(&address)
                .await
                .map_err(|err| TransportError::Refused(format!("reach {address}: {err}")))?
                .into_secure(AsyncRustlsConnector::from(tls()), &target.host)
                .await
                .map_err(|err| {
                    TransportError::Refused(format!("negotiate TLS with {address}: {err}"))
                })?,
        ),
        _ => FtpWire::Plain(
            AsyncFtpStream::connect(&address)
                .await
                .map_err(|err| TransportError::Refused(format!("reach {address}: {err}")))?,
        ),
    };

    let (user, password) = match (&credentials.user, &credentials.password) {
        (Some(user), Some(password)) => (user.clone(), password.clone()),
        (Some(user), None) => (user.clone(), String::new()),
        (None, _) => (ANONYMOUS.0.to_string(), ANONYMOUS.1.to_string()),
    };
    with_wire!(wire, session => session.login(&user, &password).await)
        .map_err(|err| TransportError::Refused(format!("log in to {address}: {err}")))?;
    // Every file this adapter sends is a stamped copy of somebody's
    // artefact, and a server left in its default text mode would rewrite
    // the line endings inside one.
    with_wire!(wire, session => session.transfer_type(FileType::Binary).await)
        .map_err(|err| TransportError::Refused(format!("ask for binary transfers: {err}")))?;

    Ok(Box::new(FtpTransport {
        wire,
        dir: target.dir.clone(),
    }))
}

/// The root certificates an FTPS send trusts — see the module docs.
fn tls() -> TlsConnector {
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    TlsConnector::from(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

/// A connection, of whichever of the two types it turned out to be.
enum FtpWire {
    /// In the clear, and only where the profile said so.
    Plain(ImplAsyncFtpStream<AsyncNoTlsStream>),
    /// Under TLS.
    Secure(ImplAsyncFtpStream<AsyncRustlsStream>),
}

/// One open FTP session.
struct FtpTransport {
    wire: FtpWire,
    dir: String,
}

#[async_trait]
impl Transport for FtpTransport {
    async fn ensure_dir(&mut self) -> Result<(), TransportError> {
        // An endpoint's path is a URL's, so a leading slash names the
        // server's root and not wherever the login landed. FTP has no
        // way to say that in a relative walk, so the walk starts from
        // the root explicitly; `sftp://` resolves the same path itself.
        // Without this one endpoint would mean two directories
        // depending on its scheme, on every server that does not put
        // the account in a chroot.
        if self.dir.starts_with('/') {
            with_wire!(self.wire, session => session.cwd("/").await).map_err(|err| {
                TransportError::Refused(format!("reach the root {} starts at: {err}", self.dir))
            })?;
        }
        // `MKD` per component, because the protocol has no recursive
        // form. A component already there answers with a refusal this
        // walk ignores; the `CWD` beside it is what says whether the
        // directory is reachable, and it leaves the session sitting in
        // the one the send is for.
        for segment in self.dir.split('/') {
            if segment.is_empty() || segment == "." {
                continue;
            }
            let _ = with_wire!(self.wire, session => session.mkdir(segment).await);
            with_wire!(self.wire, session => session.cwd(segment).await).map_err(|err| {
                TransportError::Refused(format!("reach {segment} of {}: {err}", self.dir))
            })?;
        }
        Ok(())
    }

    async fn put(&mut self, name: &str, bytes: &[u8]) -> Result<(), TransportError> {
        // The session is already in the send's directory, so a name is
        // a name. `put_with_stream` rather than `put_file` because the
        // bytes are already in memory, and `put_file` wants something to
        // read them out of.
        with_wire!(self.wire, session => {
            let mut upload = session
                .put_with_stream(name)
                .await
                .map_err(|err| TransportError::Failed(format!("open {name}: {err}")))?;
            let written = upload.write_all(bytes).await;
            // Finished either way: until it is, the data connection is
            // still open and every later command on this session is
            // refused for that reason rather than its own.
            let finished = upload.finish().await;
            written.map_err(|err| TransportError::Failed(format!("write {name}: {err}")))?;
            finished.map_err(|err| TransportError::Failed(format!("finish {name}: {err}")))
        })
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        with_wire!(self.wire, session => session.quit().await)
            .map_err(|err| TransportError::Failed(format!("close the session: {err}")))
    }
}
