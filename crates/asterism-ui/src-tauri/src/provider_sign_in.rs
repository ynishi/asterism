//! The desktop's half of a sign-in through the team's identity
//! provider (#163): the secret the server sees only as a hash, and the
//! loopback listener the browser is sent back to.
//!
//! The server's `oidc` module states the shape and what each leg
//! closes; this is the app's end of it. The app makes a secret and
//! starts an attempt with its SHA-256 and a port it is listening on at
//! `127.0.0.1`. The person signs in in their browser. The server's
//! callback sends that browser to this port with a one-time grant —
//! or with `refused=1` — and the listener answers with a `303` to the
//! server's done page, so the tab ends on the server saying what
//! happened rather than on this process. The app then collects the
//! session with the secret and the grant together.
//!
//! **Why loopback and not a poll** is the server's argument, and the
//! one sentence of it that matters here: the grant lands on a port of
//! the machine the browser is on, so an attempt somebody else started
//! delivers its grant to a port on *their* victim's machine where
//! nothing of theirs is listening. This listener is what makes that
//! true, which is why it binds `127.0.0.1` and nothing wider, answers
//! one attempt and nothing else, and goes away the moment it has
//! answered.
//!
//! The listener is an `axum` router on an ephemeral port — the same
//! server the app already runs for its own local serve, so no new
//! dependency — serving exactly one route for exactly one request.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rand::TryRngCore as _;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

/// The path the server's callback sends the browser to on this port.
/// The server chooses it (`teams_server::oidc`'s redirect to loopback)
/// and the wire crate states it on `OidcAttemptDto`'s doc; this is
/// that path, not a choice made here.
const LOOPBACK_PATH: &str = "/teams/auth/oidc/loopback";

/// What the browser brought back.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The provider's answer resolved to an account; this collects it.
    Granted(String),
    /// The provider's answer resolved to nobody here.
    Refused,
}

/// The secret an attempt is bound to, and the hash the server is told.
///
/// 256 random bits, hex. The server stores only the hash and the
/// collect presents the value, so an attempt id that leaks through a
/// browser's history collects nothing without this.
pub struct Secret {
    value: String,
}

impl Secret {
    /// A fresh one.
    pub fn new() -> Result<Self, std::io::Error> {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(|e| std::io::Error::other(format!("OS CSPRNG failure: {e}")))?;
        Ok(Self { value: hex(&bytes) })
    }

    /// The value, for the collect.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The SHA-256, hex, for starting the attempt.
    pub fn collector(&self) -> String {
        hex(&Sha256::digest(self.value.as_bytes()))
    }
}

/// A listener bound and waiting, before the attempt exists.
///
/// Bound before the attempt is started because the attempt has to be
/// started *with* the port, and served only once the attempt id is
/// known, because the listener answers for one attempt and nothing
/// else.
pub struct Listener {
    listener: tokio::net::TcpListener,
    port: u16,
}

impl Listener {
    /// Binds an ephemeral port on `127.0.0.1` — loopback and nothing
    /// wider, which is the whole of what the listener is for.
    pub async fn bind() -> Result<Self, std::io::Error> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        Ok(Self { listener, port })
    }

    /// The port to start the attempt with.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Serves until the browser brings the attempt's answer, then
    /// stops. `done_url` is where the browser is sent afterwards — the
    /// server's page for this attempt.
    ///
    /// A request naming another attempt, or naming neither a grant nor
    /// a refusal, is answered `404` and the listener keeps waiting: the
    /// port is ephemeral and nothing else should reach it, but a stray
    /// request must not be what ends the sign-in.
    ///
    /// Stops when this future ends, however it ends. The acceptor runs
    /// on its own task, and a task is not cancelled by dropping the
    /// future that spawned it — so a caller that gives up on the wait
    /// (a timeout around it, say) would otherwise leave the port bound
    /// for the life of the process, once per abandoned sign-in. The
    /// guard below aborts it on drop instead.
    pub async fn serve(self, attempt_id: String, done_url: String) -> Result<Outcome, Waited> {
        let (tx, rx) = oneshot::channel();
        let shared = Arc::new(Shared {
            attempt_id,
            done_url,
            answer: Mutex::new(Some(tx)),
        });
        let router = Router::new()
            .route(LOOPBACK_PATH, get(loopback))
            .with_state(shared);
        let _serving = AbortOnDrop(tokio::spawn(async move {
            let _ = axum::serve(
                self.listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await;
        }));
        rx.await.map_err(|_| Waited::Closed)
    }
}

/// A spawned task that ends with the future holding it.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Why a wait ended without an answer.
#[derive(Debug)]
pub enum Waited {
    /// The listener went away before the browser came back.
    Closed,
}

struct Shared {
    attempt_id: String,
    done_url: String,
    answer: Mutex<Option<oneshot::Sender<Outcome>>>,
}

/// What the server's callback put on the redirect.
#[derive(Deserialize)]
struct LoopbackQuery {
    attempt: Option<String>,
    grant: Option<String>,
    refused: Option<String>,
}

async fn loopback(
    State(shared): State<Arc<Shared>>,
    Query(query): Query<LoopbackQuery>,
) -> Response {
    if query.attempt.as_deref() != Some(shared.attempt_id.as_str()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let outcome = match (query.grant, query.refused) {
        (Some(grant), _) if !grant.is_empty() => Outcome::Granted(grant),
        (_, Some(_)) => Outcome::Refused,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    // The first answer is the answer; a browser that retries the
    // redirect after the app has collected still lands on the done
    // page, which is what tells it what happened.
    if let Some(tx) = shared.answer.lock().expect("answer lock").take() {
        let _ = tx.send(outcome);
    }
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, shared.done_url.clone())],
    )
        .into_response()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_collector_is_the_secrets_digest_and_two_secrets_differ() {
        let secret = Secret::new().unwrap();
        assert_eq!(secret.value().len(), 64);
        assert_eq!(
            secret.collector(),
            hex(&Sha256::digest(secret.value().as_bytes()))
        );
        assert_ne!(Secret::new().unwrap().value(), secret.value());
    }

    /// The listener answers one attempt: the wrong id and a redirect
    /// carrying neither a grant nor a refusal are `404` and leave it
    /// waiting; the right one is the answer and is sent on to the done
    /// page.
    #[tokio::test]
    async fn the_listener_answers_its_own_attempt_and_nothing_else() {
        let listener = Listener::bind().await.unwrap();
        let port = listener.port();
        let done = "https://teams.example/teams/auth/oidc/attempts/a1/done".to_string();
        let waiting = tokio::spawn(listener.serve("a1".into(), done.clone()));

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let base = format!("http://127.0.0.1:{port}{LOOPBACK_PATH}");
        let wrong = client
            .get(format!("{base}?attempt=other&grant=g"))
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), 404);
        let empty = client
            .get(format!("{base}?attempt=a1"))
            .send()
            .await
            .unwrap();
        assert_eq!(empty.status(), 404);
        let right = client
            .get(format!("{base}?attempt=a1&grant=the-grant"))
            .send()
            .await
            .unwrap();
        assert_eq!(right.status(), 303);
        assert_eq!(
            right.headers().get("location").unwrap().to_str().unwrap(),
            done
        );
        assert_eq!(
            waiting.await.unwrap().unwrap(),
            Outcome::Granted("the-grant".into())
        );
    }

    /// Giving up on the wait releases the port: the acceptor is
    /// aborted when the `serve` future is dropped, not only when it
    /// answers.
    #[tokio::test]
    async fn an_abandoned_wait_releases_the_port() {
        let listener = Listener::bind().await.unwrap();
        let port = listener.port();
        let waiting = listener.serve("a3".into(), "https://teams.example/done".into());
        let abandoned = tokio::time::timeout(std::time::Duration::from_millis(50), waiting).await;
        assert!(
            abandoned.is_err(),
            "nothing came back, so the wait timed out"
        );
        // The acceptor's abort lands on the runtime asynchronously, so
        // the port is bindable again soon rather than at once; what
        // the test pins is that it is bindable at all, within a bound
        // no scheduler should miss.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                Ok(_) => break,
                Err(_) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Err(err) => panic!("the port was still held: {err}"),
            }
        }
    }

    #[tokio::test]
    async fn a_refusal_reaches_the_app_as_such() {
        let listener = Listener::bind().await.unwrap();
        let port = listener.port();
        let waiting =
            tokio::spawn(listener.serve("a2".into(), "https://teams.example/done".into()));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let answered = client
            .get(format!(
                "http://127.0.0.1:{port}{LOOPBACK_PATH}?attempt=a2&refused=1"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(answered.status(), 303);
        assert_eq!(waiting.await.unwrap().unwrap(), Outcome::Refused);
    }
}
