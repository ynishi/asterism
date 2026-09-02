//! Sign-in through the instance's identity provider (#163): the
//! attempts a desktop app starts, the pages a browser walks, the
//! loopback hand-back that ties the answer to the machine the app runs
//! on, and the collect that turns a provider's answer into an ordinary
//! session.
//!
//! ## The shape
//!
//! ```text
//! app ──► listens on http://127.0.0.1:<port>
//! app ──► POST /teams/auth/oidc/attempts {collector, label, loopback_port}
//!                                        ──► {attempt_id, start_url}
//! app ──► opens start_url in the system browser
//!   browser ──► GET  …/attempts/{id}            the page: "sign in <label>?"
//!   browser ──► POST …/attempts/{id}/authorize  303 to the provider
//!   browser ──► provider ──► GET …/callback?code&state
//!                                                exchange, verify, resolve
//!   browser ◄── 303 http://127.0.0.1:<port>/…?attempt={id}&grant=…
//!   browser ──► the app's listener ──► 303 …/attempts/{id}/done  (a page)
//! app ──► POST …/attempts/{id}/collect {secret, grant} ──► session, once
//! ```
//!
//! The listener's two lines are the contract the app is to meet, not
//! something this crate holds; the wire crate's `OidcAttemptDto` is
//! where it is stated for the app.
//!
//! From the session on, nothing is new: the app mints a device token
//! on it the way it does after a password (#204), and the gate never
//! learns which way in was taken.
//!
//! ## What binds the answer to the app, and to its machine
//!
//! Two things, for two attackers.
//!
//! The attempt id travels through a browser's history and a provider's
//! logs, so it is not what collects the session. The app keeps a secret
//! and starts the attempt with its SHA-256; the collect presents the
//! secret, and a collect that presents anything else is answered as
//! though the attempt did not exist. That is one answer for six cases
//! — a wrong secret, a wrong grant, an id nothing names, an attempt
//! past its expiry, one already collected, one the browser has not
//! finished — so that none of them can be told apart from outside.
//! That closes the case of a third party who *learns* an id.
//!
//! It closes nothing against somebody who *started* the attempt and
//! gets a person to finish it in their browser — the shape device-code
//! phishing takes — because the starter holds the secret. What closes
//! that case is where the provider's answer goes: not to the app's
//! poll, but to the browser, as a redirect to the loopback address the
//! attempt was started with, carrying a one-time grant the collect
//! also requires. The browser that finished the sign-in is on the
//! person's machine, `127.0.0.1` on that machine is the person's
//! machine, and an app listening there is the app the person is
//! running. An attempt started elsewhere sends its grant to a port on
//! the victim's machine where nothing of the attacker's is listening,
//! and the grant is never collected. There is no poll to fall back to;
//! a client that cannot listen on loopback cannot sign in this way,
//! which is the price, stated. RFC 8252 §7.3 is the loopback shape,
//! and the AWS CLI's move from device code to loopback is the
//! precedent for choosing it over polling for exactly this attack.
//!
//! The page before the provider is still there and still asks, with
//! the label the attempt was started with. It is a courtesy and a
//! speed bump, not the defence: the label is text whoever started the
//! attempt typed. What the page does have to do is not be skippable —
//! its button takes a token only the page hands out, and the page
//! refuses to be framed — so that the person does see it.
//!
//! ## In memory, not in the database
//!
//! Attempts live in a map on the context, swept on every start and
//! gone with the process. An attempt is ten minutes of state with no
//! meaning after the session it produced, and a restart mid-sign-in
//! costs the person a second click, which is the same cost the
//! limiter's in-memory buckets accept for the same reason.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use asterism_teams_wire::command::{CollectOidcAttemptCommand, OidcAttemptCommand};
use asterism_teams_wire::dto::{AuthProvidersDto, OidcAttemptDto, OidcProviderDto, SessionDto};
use axum::Json;
use axum::extract::{Form, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use teams_core::DomainError;
use teams_infra::auth::oidc::{Exchange, OidcClient, OidcIdentities, sha256_hex};
use teams_infra::auth::password::AccountRecord;

use crate::http::{ApiError, session_dto};
use crate::state::{TeamsCtx, now_ms};

/// How long an attempt may sit between being started and being
/// collected: **ten minutes**, so that a start page left in a tab is
/// not a standing invitation.
pub const ATTEMPT_TTL_MS: i64 = 10 * 60 * 1000;

/// The provider half of the context: the client, the bindings, and the
/// attempts in flight.
pub struct OidcSignIn {
    client: OidcClient,
    identities: OidcIdentities,
    public_url: String,
    attempts: Mutex<HashMap<String, Attempt>>,
}

impl std::fmt::Debug for OidcSignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcSignIn")
            .field("client", &self.client)
            .field("public_url", &self.public_url)
            .finish_non_exhaustive()
    }
}

/// Where an attempt is.
enum Phase {
    /// Started; the browser has not come back.
    Waiting,
    /// The provider's answer resolved to this account, and the grant
    /// the browser was sent to the app with is what collects it.
    Resolved {
        account: AccountRecord,
        grant: String,
    },
    /// Collected: the session is the app's, and this is only here so
    /// the page the browser lands on afterwards can say it went well.
    Collected,
    /// The provider's answer resolved to nobody, or did not verify.
    Refused(&'static str),
}

struct Attempt {
    collector_hash: String,
    label: String,
    /// The port the app listens on at `127.0.0.1`, where the callback
    /// sends the browser.
    loopback_port: u16,
    /// What the page's form carries and the button presents. The page
    /// is what stands between somebody else's attempt and a person's
    /// browser, and a form on another origin could post to the button
    /// without ever showing the page — unless the button asks for
    /// something only the page hands out, which another origin cannot
    /// read. This is that.
    page_token: String,
    nonce: String,
    pkce_verifier: String,
    pkce_challenge: String,
    expires_at_ms: i64,
    phase: Phase,
}

/// What a collect comes to, before it is a status code.
pub enum Collect {
    /// Nothing to collect from — the module doc lists the six cases
    /// this covers and says why they are one answer.
    Unknown,
    /// The provider's answer authenticated nobody here.
    Refused,
    /// This account, once.
    Resolved(AccountRecord),
}

/// Where the callback sends the browser.
enum Completed {
    /// To the app, with the grant.
    SignedIn { port: u16, grant: String },
    /// To the app, saying it was refused, so the app can stop waiting.
    Refused { port: u16 },
    /// Nowhere: the attempt is not one this instance knows.
    Unknown,
}

/// What the page after the hand-back says. Not who: the page is keyed
/// by the attempt id, which is not a secret, and a name on it would
/// tell whoever holds the id that a person finished and what the
/// instance calls them. The app has the session and can say the name
/// itself.
enum Done {
    SignedIn,
    Refused,
    Unknown,
}

impl OidcSignIn {
    /// Assembles the half. `public_url` is the origin members' browsers
    /// reach this instance at, which the start URL is built on.
    pub fn new(client: OidcClient, identities: OidcIdentities, public_url: &str) -> Self {
        Self {
            client,
            identities,
            public_url: public_url.trim_end_matches('/').to_string(),
            attempts: Mutex::new(HashMap::new()),
        }
    }

    /// The provider as a connect form names it.
    pub fn provider(&self) -> OidcProviderDto {
        OidcProviderDto {
            name: self.client.display_name().to_string(),
        }
    }

    /// Starts an attempt. Sweeps the expired ones first, so the map is
    /// bounded by how many people are mid-sign-in rather than by how
    /// many ever were.
    pub fn start(
        &self,
        collector_hash: &str,
        label: &str,
        loopback_port: u16,
        now: i64,
    ) -> Result<OidcAttemptDto, DomainError> {
        let label = label.trim().to_string();
        if label.is_empty() {
            return Err(DomainError::Validation(
                "device label is blank; the start page shows it so a person can tell an \
                 attempt of their own from somebody else's"
                    .into(),
            ));
        }
        let collector_hash = collector_hash.trim().to_lowercase();
        if collector_hash.len() != 64 || !collector_hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DomainError::Validation(
                "collector is not a SHA-256 in hex".into(),
            ));
        }
        if loopback_port == 0 {
            return Err(DomainError::Validation(
                "loopback_port is 0; the app has to be listening somewhere for the answer \
                 to reach it"
                    .into(),
            ));
        }
        let id = OidcClient::random_token()?;
        let nonce = OidcClient::random_token()?;
        let page_token = OidcClient::random_token()?;
        let (pkce_verifier, pkce_challenge) = OidcClient::pkce_pair()?;
        let expires_at_ms = now.saturating_add(ATTEMPT_TTL_MS);
        let mut attempts = self.attempts.lock().expect("attempts lock");
        attempts.retain(|_, attempt| attempt.expires_at_ms > now);
        attempts.insert(
            id.clone(),
            Attempt {
                collector_hash,
                label,
                loopback_port,
                page_token,
                nonce,
                pkce_verifier,
                pkce_challenge,
                expires_at_ms,
                phase: Phase::Waiting,
            },
        );
        Ok(OidcAttemptDto {
            start_url: format!("{}/teams/auth/oidc/attempts/{id}", self.public_url),
            attempt_id: id,
            expires_at_ms,
        })
    }

    /// The label a live attempt was started with and the token its page
    /// hands the button, for the page.
    fn page_of(&self, id: &str, now: i64) -> Option<(String, String)> {
        let attempts = self.attempts.lock().expect("attempts lock");
        attempts
            .get(id)
            .filter(|attempt| attempt.expires_at_ms > now)
            .map(|attempt| (attempt.label.clone(), attempt.page_token.clone()))
    }

    /// Where to send the browser for a live attempt whose page token
    /// the button presented, or `None` for one this instance does not
    /// know — which a wrong token is, deliberately.
    async fn authorization_url(
        &self,
        id: &str,
        page_token: &str,
        now: i64,
    ) -> Result<Option<String>, DomainError> {
        let (nonce, challenge) = {
            let attempts = self.attempts.lock().expect("attempts lock");
            let Some(attempt) = attempts.get(id).filter(|a| a.expires_at_ms > now) else {
                return Ok(None);
            };
            if attempt.page_token != page_token {
                return Ok(None);
            }
            (attempt.nonce.clone(), attempt.pkce_challenge.clone())
        };
        self.client
            .authorization_url(id, &nonce, &challenge)
            .await
            .map(Some)
    }

    /// The callback: what the provider sent the browser back with, run
    /// to a phase. The lock is taken twice and held across no await —
    /// once to read what the exchange needs, once to record what it
    /// came to — so a slow provider blocks nothing but this attempt.
    async fn complete(
        &self,
        state: &str,
        code: Option<&str>,
        error: Option<&str>,
        now: i64,
    ) -> Result<Completed, DomainError> {
        let (nonce, verifier) = {
            let attempts = self.attempts.lock().expect("attempts lock");
            let Some(attempt) = attempts.get(state).filter(|a| a.expires_at_ms > now) else {
                return Ok(Completed::Unknown);
            };
            if !matches!(attempt.phase, Phase::Waiting) {
                return Ok(Completed::Unknown);
            }
            (attempt.nonce.clone(), attempt.pkce_verifier.clone())
        };
        let outcome = match (code, error) {
            (_, Some(_)) | (None, None) => {
                Phase::Refused("the provider sent the browser back without a code")
            }
            (Some(code), None) => match self.client.exchange(code, &verifier, &nonce).await? {
                Exchange::Refused(reason) => Phase::Refused(reason),
                Exchange::Verified(identity) => {
                    match self.identities.resolve(&identity, now).await? {
                        Some(account) => Phase::Resolved {
                            account,
                            grant: OidcClient::random_token()?,
                        },
                        None => {
                            Phase::Refused("the provider vouched for nobody this instance knows")
                        }
                    }
                }
            },
        };
        let mut attempts = self.attempts.lock().expect("attempts lock");
        let Some(attempt) = attempts.get_mut(state) else {
            return Ok(Completed::Unknown);
        };
        // Two callbacks for one attempt — a browser that retried, a
        // provider that sent twice — race here, and the second must
        // not overwrite what the first recorded, whichever it was.
        if !matches!(attempt.phase, Phase::Waiting) {
            return Ok(Completed::Unknown);
        }
        let port = attempt.loopback_port;
        let completed = match &outcome {
            Phase::Resolved { grant, .. } => Completed::SignedIn {
                port,
                grant: grant.clone(),
            },
            Phase::Refused(reason) => {
                eprintln!("teams-server: sign-in through the provider refused: {reason}");
                Completed::Refused { port }
            }
            Phase::Waiting | Phase::Collected => {
                unreachable!("an outcome is never waiting or collected")
            }
        };
        attempt.phase = outcome;
        Ok(completed)
    }

    /// What the page after the hand-back has to say about a live
    /// attempt.
    fn done(&self, id: &str, now: i64) -> Done {
        let attempts = self.attempts.lock().expect("attempts lock");
        let Some(attempt) = attempts.get(id).filter(|a| a.expires_at_ms > now) else {
            return Done::Unknown;
        };
        match &attempt.phase {
            Phase::Resolved { .. } | Phase::Collected => Done::SignedIn,
            Phase::Refused(_) => Done::Refused,
            Phase::Waiting => Done::Unknown,
        }
    }

    /// Collects an attempt: the secret says the caller is the app that
    /// started it, the grant says the browser that finished it landed
    /// on that app's machine. A resolved attempt is collected once; a
    /// refused one is answered as such to a caller with the secret,
    /// and left where it is until the sweep takes it, so the page the
    /// browser reaches afterwards can still say it was refused.
    pub fn collect(&self, id: &str, secret: &str, grant: &str, now: i64) -> Collect {
        let mut attempts = self.attempts.lock().expect("attempts lock");
        let Some(attempt) = attempts.get_mut(id).filter(|a| a.expires_at_ms > now) else {
            return Collect::Unknown;
        };
        if attempt.collector_hash != sha256_hex(secret) {
            return Collect::Unknown;
        }
        match &attempt.phase {
            Phase::Refused(_) => Collect::Refused,
            Phase::Resolved {
                account,
                grant: held,
            } if held == grant => {
                let account = account.clone();
                attempt.phase = Phase::Collected;
                Collect::Resolved(account)
            }
            Phase::Resolved { .. } | Phase::Collected | Phase::Waiting => Collect::Unknown,
        }
    }
}

// ----------------------------------------------------------------------
// Handlers.
// ----------------------------------------------------------------------

fn provider_of(ctx: &TeamsCtx) -> Result<&Arc<OidcSignIn>, ApiError> {
    ctx.oidc.as_ref().ok_or(ApiError::NoProvider)
}

/// `GET /teams/auth/providers` — what this instance offers besides a
/// password. Public: a connect form reads it before anybody is in.
pub(crate) async fn providers(State(ctx): State<Arc<TeamsCtx>>) -> Json<AuthProvidersDto> {
    Json(AuthProvidersDto {
        oidc: ctx.oidc.as_ref().map(|oidc| oidc.provider()),
    })
}

/// `POST /teams/auth/oidc/attempts` — starts an attempt.
pub(crate) async fn attempt_start(
    State(ctx): State<Arc<TeamsCtx>>,
    Json(cmd): Json<OidcAttemptCommand>,
) -> Result<Json<OidcAttemptDto>, ApiError> {
    let oidc = provider_of(&ctx)?;
    Ok(Json(oidc.start(
        &cmd.collector,
        &cmd.label,
        cmd.loopback_port,
        now_ms(),
    )?))
}

/// `GET /teams/auth/oidc/attempts/{id}` — the page a browser lands on:
/// which device is asking, and a button that goes on to the provider.
pub(crate) async fn attempt_page(
    State(ctx): State<Arc<TeamsCtx>>,
    Path(id): Path<String>,
) -> Response {
    let Some(oidc) = ctx.oidc.as_ref() else {
        return page(StatusCode::NOT_FOUND, "No provider", NO_PROVIDER);
    };
    let Some((label, page_token)) = oidc.page_of(&id, now_ms()) else {
        return page(StatusCode::NOT_FOUND, "Sign-in expired", EXPIRED);
    };
    let body = format!(
        "<p>A device called <strong>{label}</strong> is asking to sign in to this team server \
         through <strong>{provider}</strong>.</p>\
         <p>If that is not a device of yours, close this page.</p>\
         <form method=\"post\" action=\"/teams/auth/oidc/attempts/{id}/authorize\">\
         <input type=\"hidden\" name=\"token\" value=\"{token}\">\
         <button type=\"submit\">Continue to {provider}</button></form>",
        label = escape(&label),
        provider = escape(oidc.client.display_name()),
        id = escape(&id),
        token = escape(&page_token),
    );
    page(StatusCode::OK, "Sign in", &body)
}

/// What the page's form posts to the button.
#[derive(Deserialize)]
pub(crate) struct AuthorizeForm {
    token: String,
}

/// `POST /teams/auth/oidc/attempts/{id}/authorize` — the button: on to
/// the provider. A `303`, so a browser that is sent back here by
/// history re-`GET`s the page rather than re-posting. The form's
/// token is what makes this the page's button and not anybody's: a
/// post from another origin cannot carry it, and is answered as
/// though the attempt did not exist.
pub(crate) async fn attempt_authorize(
    State(ctx): State<Arc<TeamsCtx>>,
    Path(id): Path<String>,
    Form(form): Form<AuthorizeForm>,
) -> Response {
    let Some(oidc) = ctx.oidc.as_ref() else {
        return page(StatusCode::NOT_FOUND, "No provider", NO_PROVIDER);
    };
    match oidc.authorization_url(&id, &form.token, now_ms()).await {
        Ok(Some(url)) => (StatusCode::SEE_OTHER, [(header::LOCATION, url)]).into_response(),
        Ok(None) => page(StatusCode::NOT_FOUND, "Sign-in expired", EXPIRED),
        Err(err) => {
            eprintln!("teams-server: could not reach the provider: {err}");
            page(StatusCode::BAD_GATEWAY, "Provider unreachable", UNREACHABLE)
        }
    }
}

/// What the provider sends the browser back with (RFC 6749 §4.1.2).
#[derive(Deserialize)]
pub(crate) struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

/// `GET /teams/auth/oidc/callback` — the provider's answer, run to a
/// phase, and the browser sent on to the app's loopback listener with
/// what it came to. Presents a credential (the code) and sits under
/// the limiter for it.
///
/// The redirect carries the grant in the query, which is the one
/// place a browser can carry it to a listener without script. It is
/// single-use, bound to the attempt, and worthless without the secret
/// the app never sent anywhere — a copy in the browser's history
/// collects nothing.
pub(crate) async fn callback(
    State(ctx): State<Arc<TeamsCtx>>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let Some(oidc) = ctx.oidc.as_ref() else {
        return page(StatusCode::NOT_FOUND, "No provider", NO_PROVIDER);
    };
    let Some(state) = query.state.as_deref() else {
        return page(StatusCode::NOT_FOUND, "Sign-in expired", EXPIRED);
    };
    match oidc
        .complete(
            state,
            query.code.as_deref(),
            query.error.as_deref(),
            now_ms(),
        )
        .await
    {
        Ok(Completed::SignedIn { port, grant }) => to_loopback(port, state, Some(&grant)),
        Ok(Completed::Refused { port }) => to_loopback(port, state, None),
        Ok(Completed::Unknown) => page(StatusCode::NOT_FOUND, "Sign-in expired", EXPIRED),
        Err(err) => {
            eprintln!("teams-server: could not reach the provider: {err}");
            page(StatusCode::BAD_GATEWAY, "Provider unreachable", UNREACHABLE)
        }
    }
}

/// The `303` to the app's listener. `attempt` is always there so the
/// listener knows which of its attempts came back; `grant` is there
/// only for a sign-in that resolved.
fn to_loopback(port: u16, attempt: &str, grant: Option<&str>) -> Response {
    let query = match grant {
        Some(grant) => format!("attempt={attempt}&grant={grant}"),
        None => format!("attempt={attempt}&refused=1"),
    };
    (
        StatusCode::SEE_OTHER,
        [(
            header::LOCATION,
            format!("http://127.0.0.1:{port}/teams/auth/oidc/loopback?{query}"),
        )],
    )
        .into_response()
}

/// `GET /teams/auth/oidc/attempts/{id}/done` — the page the app's
/// listener is to send the browser on to, so the tab ends on this
/// instance saying what happened rather than on a bare loopback
/// response.
pub(crate) async fn attempt_done(
    State(ctx): State<Arc<TeamsCtx>>,
    Path(id): Path<String>,
) -> Response {
    let Some(oidc) = ctx.oidc.as_ref() else {
        return page(StatusCode::NOT_FOUND, "No provider", NO_PROVIDER);
    };
    match oidc.done(&id, now_ms()) {
        Done::SignedIn => page(StatusCode::OK, "Signed in", SIGNED_IN),
        Done::Refused => page(StatusCode::UNAUTHORIZED, "Not signed in", REFUSED),
        Done::Unknown => page(StatusCode::NOT_FOUND, "Sign-in expired", EXPIRED),
    }
}

/// `POST /teams/auth/oidc/attempts/{id}/collect` — the app's side.
pub(crate) async fn attempt_collect(
    State(ctx): State<Arc<TeamsCtx>>,
    Path(id): Path<String>,
    Json(cmd): Json<CollectOidcAttemptCommand>,
) -> Result<Json<SessionDto>, ApiError> {
    let oidc = provider_of(&ctx)?;
    let now = now_ms();
    match oidc.collect(&id, &cmd.secret, &cmd.grant, now) {
        Collect::Unknown => Err(ApiError::AttemptNotFound),
        Collect::Refused => Err(ApiError::Unauthorized),
        Collect::Resolved(account) => {
            ctx.auth.cleanup_expired(now).await?;
            let token = ctx
                .auth
                .create_session(account.user_id, now, ctx.session_ttl_ms)
                .await?;
            Ok(Json(session_dto(&ctx, token, &account, now).await?))
        }
    }
}

// ----------------------------------------------------------------------
// The pages. Plain HTML, no script, no stylesheet fetched from
// anywhere: a sign-in page that loads nothing from a third origin has
// nothing on it that a third origin could change.
// ----------------------------------------------------------------------

const NO_PROVIDER: &str = "<p>This team server signs people in with passwords only.</p>";
const SIGNED_IN: &str = "<p>Signed in. You can close this page and return to the app.</p>";
const EXPIRED: &str = "<p>This sign-in is no longer waiting. Start again from the app.</p>";
const REFUSED: &str = "<p>This instance did not accept the sign-in. If you were expecting \
     to be let in, ask whoever runs the team server to check that your account is bound \
     to your provider address.</p>";
const UNREACHABLE: &str =
    "<p>The identity provider could not be reached. Try again in a moment.</p>";

fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title} — Asterism teams</title>\
         <style>body{{font:16px/1.5 system-ui,sans-serif;max-width:36rem;margin:4rem auto;\
         padding:0 1rem;color:#222}}button{{font:inherit;padding:.5rem 1rem}}</style>\
         </head><body><h1>{title}</h1>{body}</body></html>",
        title = escape(title),
    );
    // Never framed: a page whose button is the defence must not be
    // clickable through another origin's overlay.
    (
        status,
        [
            (header::X_FRAME_OPTIONS, "DENY"),
            (header::CONTENT_SECURITY_POLICY, "frame-ancestors 'none'"),
        ],
        Html(html),
    )
        .into_response()
}

/// The five characters HTML gives a meaning to, escaped — the whole of
/// what these pages interpolate is a label somebody typed, a name the
/// operator gave the provider, and an attempt id.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pages_escape_what_a_person_or_a_provider_wrote() {
        assert_eq!(
            escape("<script>alert('x')</script> & \"y\""),
            "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;y&quot;"
        );
    }
}
