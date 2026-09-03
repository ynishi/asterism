//! End-to-end guard for sign-in through an identity provider (#163):
//! the attempt an app starts, the pages a browser walks, the exchange
//! the instance runs as the provider's client, and the collect that
//! ends in the same session a password ends in.
//!
//! The provider is real in the one way that matters — it is an HTTP
//! server on a port, because the instance under test reaches it with
//! `reqwest` and nothing short of a socket exercises that — and fake
//! in every other: it signs whatever the test tells it to, with a key
//! it made for this run. The browser is the test itself, following
//! each `Location` by hand through `oneshot`, which is how a suite
//! sees the redirect a browser would follow blindly.
//!
//! What the last test claims is the half of #163's own verification
//! sentence that is this suite's to claim: on an instance with no
//! provider configured, every new route answers that there is no
//! provider. That the routes which existed before answer as they did
//! is the other suites' claim.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post as post_route};
use axum::{Form, Json};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use p256::pkcs8::{EncodePrivateKey, LineEnding};
use rand::TryRngCore;
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver};
use sha2::{Digest, Sha256};
use teams_core::domain::identity::RegistrationPolicy;
use teams_infra::auth::oidc::{OidcClient, OidcConfig, OidcIdentities};
use teams_infra::auth::password::PasswordAuth;
use teams_infra::sqlite::SqliteTeamsRepository;
use teams_server::oidc::OidcSignIn;
use teams_server::rate_limit::RateLimiter;
use teams_server::state::{TeamsCtx, now_ms};
use tower::ServiceExt;

const CLIENT_ID: &str = "asterism-teams";
const CLIENT_SECRET: &str = "not-a-real-secret-but-a-fixed-one";
const PUBLIC_URL: &str = "https://teams.example";
const KID: &str = "run-key";
const GOOD: &str = "correct horse battery staple";

// ----------------------------------------------------------------------
// The provider.
// ----------------------------------------------------------------------

/// How the next token should be wrong, if at all.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tamper {
    None,
    WrongIssuer,
    WrongAudience,
    Expired,
    WrongNonce,
}

/// Who the provider will say the next person is.
#[derive(Clone)]
struct Person {
    subject: String,
    email: String,
    verified: bool,
}

/// What an authorization left behind for the exchange to check.
#[derive(Clone)]
struct Issued {
    nonce: String,
    challenge: String,
    redirect_uri: String,
}

struct ProviderState {
    base: String,
    key_pem: String,
    jwk: serde_json::Value,
    person: Person,
    tamper: Tamper,
    codes: HashMap<String, Issued>,
    exchanges: u32,
}

#[derive(Clone)]
struct Provider {
    state: Arc<Mutex<ProviderState>>,
    router: Router,
}

async fn provider() -> Provider {
    // A P-256 key for this run. `from_slice` refuses the one value in
    // 2^256 that is not a scalar, so the loop is the honest spelling.
    let secret = loop {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.try_fill_bytes(&mut bytes).unwrap();
        if let Ok(key) = p256::SecretKey::from_slice(&bytes) {
            break key;
        }
    };
    let key_pem = secret.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
    let mut jwk: serde_json::Value =
        serde_json::from_str(&secret.public_key().to_jwk_string()).unwrap();
    jwk["kid"] = KID.into();
    jwk["use"] = "sig".into();
    jwk["alg"] = "ES256".into();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral port for the provider");
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(ProviderState {
        base: format!("http://{addr}"),
        key_pem,
        jwk,
        person: Person {
            subject: "sub-hoshino".into(),
            email: "hoshino@example.com".into(),
            verified: true,
        },
        tamper: Tamper::None,
        codes: HashMap::new(),
        exchanges: 0,
    }));
    let router = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/authorize", get(authorize))
        .route("/token", post_route(token))
        .with_state(state.clone());
    let served = router.clone();
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            served.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    Provider { state, router }
}

impl Provider {
    fn issuer(&self) -> String {
        self.state.lock().unwrap().base.clone()
    }

    fn next_person(&self, subject: &str, email: &str, verified: bool) {
        self.state.lock().unwrap().person = Person {
            subject: subject.into(),
            email: email.into(),
            verified,
        };
    }

    fn tamper(&self, tamper: Tamper) {
        self.state.lock().unwrap().tamper = tamper;
    }

    fn exchanges(&self) -> u32 {
        self.state.lock().unwrap().exchanges
    }
}

async fn discovery(State(state): State<Arc<Mutex<ProviderState>>>) -> Json<serde_json::Value> {
    let base = state.lock().unwrap().base.clone();
    Json(serde_json::json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "jwks_uri": format!("{base}/jwks"),
        "response_types_supported": ["code"],
        "id_token_signing_alg_values_supported": ["ES256"],
    }))
}

async fn jwks(State(state): State<Arc<Mutex<ProviderState>>>) -> Json<serde_json::Value> {
    let jwk = state.lock().unwrap().jwk.clone();
    Json(serde_json::json!({ "keys": [jwk] }))
}

/// The person "signs in" and consents; the provider sends the browser
/// back with a code that remembers what it was asked with.
async fn authorize(
    State(state): State<Arc<Mutex<ProviderState>>>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let get = |key: &str| query.get(key).cloned().unwrap_or_default();
    assert_eq!(get("response_type"), "code");
    assert_eq!(get("client_id"), CLIENT_ID);
    assert_eq!(get("code_challenge_method"), "S256");
    assert!(get("scope").split(' ').any(|s| s == "openid"));
    let code = format!("code-{}", state.lock().unwrap().codes.len());
    state.lock().unwrap().codes.insert(
        code.clone(),
        Issued {
            nonce: get("nonce"),
            challenge: get("code_challenge"),
            redirect_uri: get("redirect_uri"),
        },
    );
    Redirect::to(&format!(
        "{}?code={code}&state={}",
        get("redirect_uri"),
        get("state")
    ))
    .into_response()
}

/// The exchange: client authentication, the code, PKCE — then a token
/// shaped as the test asked.
async fn token(
    State(state): State<Arc<Mutex<ProviderState>>>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let expected = format!(
        "Basic {}",
        base64_encode(format!("{CLIENT_ID}:{CLIENT_SECRET}").as_bytes())
    );
    if authorization != expected {
        return (StatusCode::UNAUTHORIZED, "client authentication failed").into_response();
    }
    if form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return (StatusCode::BAD_REQUEST, "unsupported grant").into_response();
    }
    let mut guard = state.lock().unwrap();
    guard.exchanges += 1;
    let Some(issued) = form.get("code").and_then(|code| guard.codes.remove(code)) else {
        return (StatusCode::BAD_REQUEST, "unknown code").into_response();
    };
    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
    if base64url(&Sha256::digest(verifier.as_bytes())) != issued.challenge {
        return (StatusCode::BAD_REQUEST, "PKCE verifier does not match").into_response();
    }
    if form.get("redirect_uri") != Some(&issued.redirect_uri) {
        return (StatusCode::BAD_REQUEST, "redirect_uri does not match").into_response();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let tamper = guard.tamper;
    let person = guard.person.clone();
    let claims = serde_json::json!({
        "iss": if tamper == Tamper::WrongIssuer { "https://somebody-else.example".to_string() } else { guard.base.clone() },
        "aud": if tamper == Tamper::WrongAudience { "another-client" } else { CLIENT_ID },
        "sub": person.subject,
        "iat": now - 5,
        "exp": if tamper == Tamper::Expired { now - 600 } else { now + 300 },
        "nonce": if tamper == Tamper::WrongNonce { "not-the-nonce".to_string() } else { issued.nonce },
        "email": person.email,
        "email_verified": person.verified,
        "name": "Hoshino",
    });
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(KID.into());
    let id_token = jsonwebtoken::encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(guard.key_pem.as_bytes()).unwrap(),
    )
    .unwrap();
    Json(serde_json::json!({
        "id_token": id_token,
        "access_token": "opaque",
        "token_type": "Bearer",
    }))
    .into_response()
}

fn base64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

// ----------------------------------------------------------------------
// The instance under test.
// ----------------------------------------------------------------------

struct Harness {
    ctx: Arc<TeamsCtx>,
    router: Router,
    provider: Option<Provider>,
    #[allow(dead_code)] // Held so the isle outlives every request.
    isle: AsyncIsle,
    driver: AsyncIsleDriver,
    #[allow(dead_code)] // Held so the blob root outlives every request.
    blob_dir: tempfile::TempDir,
}

async fn harness(with_provider: bool) -> Harness {
    let (isle, driver) = teams_infra::sqlite::open_and_migrate_in_memory()
        .await
        .expect("open in-memory teams db");
    let blob_dir = tempfile::tempdir().expect("blob tempdir");
    let blobs = teams_infra::blob::LocalFileStorageAdapter::open(blob_dir.path().join("blobs"))
        .await
        .expect("open blob store");
    let provider = if with_provider {
        Some(provider().await)
    } else {
        None
    };
    let oidc = provider.as_ref().map(|provider| {
        let client = OidcClient::new(OidcConfig {
            issuer: provider.issuer(),
            client_id: CLIENT_ID.into(),
            client_secret: CLIENT_SECRET.into(),
            redirect_url: format!("{PUBLIC_URL}/teams/auth/oidc/callback"),
            display_name: "Example IdP".into(),
        });
        Arc::new(OidcSignIn::new(
            client,
            OidcIdentities::new(isle.clone()),
            PUBLIC_URL,
        ))
    });
    let ctx = Arc::new(TeamsCtx {
        repo: SqliteTeamsRepository::new(isle.clone()),
        auth: PasswordAuth::new(isle.clone()),
        oidc,
        projections: teams_infra::sqlite::projection::SqliteProjectionStore::new(isle.clone()),
        blobs,
        registration: RegistrationPolicy::Open,
        session_ttl_ms: 60_000,
        device_token_ttl_ms: teams_server::state::DEFAULT_DEVICE_TOKEN_TTL_MS,
        device_token_idle_ms: None,
        auth_limiter: RateLimiter::new(1_000, Duration::from_secs(60)),
        purge_grace_ms: 0,
        gc_guard: Arc::new(teams_infra::gc::GcGuard::new()),
    });
    let router = teams_server::http::router(ctx.clone());
    Harness {
        ctx,
        router,
        provider,
        isle,
        driver,
        blob_dir,
    }
}

async fn call(router: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

async fn call_json(router: &Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let (status, _, body) = call(router, request).await;
    let json = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("body is not JSON ({e}): {body}"))
    };
    (status, json)
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build POST")
}

fn post_authed(uri: &str, token: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .expect("build authed POST")
}

fn get_plain(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("build GET")
}

fn post_form(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .expect("build form POST")
}

/// The token the page's form carries — what a browser would post
/// back, read out of the HTML the way a browser would.
fn page_token(page: &str) -> String {
    let marker = "name=\"token\" value=\"";
    let start = page.find(marker).expect("the form carries a token") + marker.len();
    let end = page[start..].find('"').expect("the value closes") + start;
    page[start..end].to_string()
}

fn location(headers: &HeaderMap) -> String {
    headers
        .get(header::LOCATION)
        .expect("a Location header")
        .to_str()
        .unwrap()
        .to_string()
}

/// Everything after the origin of an absolute URL — what `oneshot`
/// routes on.
fn path_of(url: &str) -> String {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    match after_scheme.find('/') {
        Some(index) => after_scheme[index..].to_string(),
        None => "/".to_string(),
    }
}

fn sha256_hex(input: &str) -> String {
    Sha256::digest(input.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The port the app "listens" on in this suite. Nothing does: the
/// loopback leg is asserted on the redirect the instance issues, since
/// the whole of its property is *where* the browser is sent.
const LOOPBACK_PORT: u16 = 49_152;

/// What the browser's walk came to.
struct Walked {
    id: String,
    /// The grant the loopback redirect carried, for a sign-in that
    /// resolved.
    grant: Option<String>,
    /// The done page's status and body — what the tab ends on.
    status: StatusCode,
    page: String,
}

impl Walked {
    fn grant(&self) -> &str {
        self.grant.as_deref().unwrap_or("")
    }
}

/// An attempt started, and the browser walked to the provider, back,
/// and on to the loopback listener and the done page.
async fn walk_the_browser(h: &Harness, secret: &str, label: &str) -> Walked {
    let (status, attempt) = call_json(
        &h.router,
        post(
            "/teams/auth/oidc/attempts",
            serde_json::json!({
                "collector": sha256_hex(secret),
                "label": label,
                "loopback_port": LOOPBACK_PORT,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "start refused: {attempt}");
    let id = attempt["attempt_id"].as_str().unwrap().to_string();
    let start_url = attempt["start_url"].as_str().unwrap().to_string();
    assert!(start_url.starts_with(PUBLIC_URL), "{start_url}");
    // The life the app waits, as a duration and not only as an instant
    // on this clock (the wire crate's `OidcAttemptDto` says why).
    assert_eq!(
        attempt["ttl_ms"].as_i64(),
        Some(teams_server::oidc::ATTEMPT_TTL_MS),
        "the attempt does not state its life as a duration: {attempt}"
    );

    // The page names the device and asks — escaped, which is why the
    // labels in this suite carry an apostrophe — and refuses to be
    // framed, which is what makes its button the person's click.
    let (status, headers, page) = call(&h.router, get_plain(&path_of(&start_url))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(&label.replace('\'', "&#39;")), "{page}");
    assert!(page.contains("Example IdP"), "{page}");
    assert_eq!(headers.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
    assert_eq!(
        headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        "frame-ancestors 'none'"
    );

    // The button without the page's token first — a form from another
    // origin — is answered as no attempt at all.
    let authorize = format!("{}/authorize", path_of(&start_url));
    let (status, _, _) = call(&h.router, post_form(&authorize, "token=guessed")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The button as the page posts it: on to the provider.
    let form = format!("token={}", page_token(&page));
    let (status, headers, _) = call(&h.router, post_form(&authorize, &form)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let to_provider = location(&headers);
    assert!(
        to_provider.starts_with(&h.provider.as_ref().unwrap().issuer()),
        "{to_provider}"
    );

    // The provider: sign in, consent, back to the instance.
    let (status, headers, _) = call(
        &h.provider.as_ref().unwrap().router,
        get_plain(&path_of(&to_provider)),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let back = location(&headers);
    assert!(back.starts_with(PUBLIC_URL), "{back}");

    // The callback: on to the app's loopback listener, whatever the
    // outcome — that is the leg that ties the answer to the machine.
    let (status, headers, _) = call(&h.router, get_plain(&path_of(&back))).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let to_app = location(&headers);
    let expected =
        format!("http://127.0.0.1:{LOOPBACK_PORT}/teams/auth/oidc/loopback?attempt={id}&");
    assert!(to_app.starts_with(&expected), "{to_app}");
    let tail = &to_app[expected.len()..];
    let grant = tail.strip_prefix("grant=").map(str::to_string).or_else(|| {
        assert_eq!(tail, "refused=1", "{to_app}");
        None
    });

    // The listener sends the browser on to the done page.
    let (status, _, page) = call(
        &h.router,
        get_plain(&format!("/teams/auth/oidc/attempts/{id}/done")),
    )
    .await;
    Walked {
        id,
        grant,
        status,
        page,
    }
}

async fn collect(
    h: &Harness,
    id: &str,
    secret: &str,
    grant: &str,
) -> (StatusCode, serde_json::Value) {
    call_json(
        &h.router,
        post(
            &format!("/teams/auth/oidc/attempts/{id}/collect"),
            serde_json::json!({ "secret": secret, "grant": grant }),
        ),
    )
    .await
}

async fn bound_account(h: &Harness, login: &str, email: &str) {
    let user_id = h
        .ctx
        .auth
        .create_account_locked(login, login, false, now_ms())
        .await
        .expect("create the locked account");
    OidcIdentities::new(h.isle.clone())
        .bind_email(user_id, &h.provider.as_ref().unwrap().issuer(), email)
        .await
        .expect("bind the address");
}

// ----------------------------------------------------------------------
// The tests.
// ----------------------------------------------------------------------

#[tokio::test]
async fn a_bound_address_signs_in_by_email_once_and_by_subject_after() {
    let h = harness(true).await;
    bound_account(&h, "hoshino", "Hoshino@Example.com").await;

    let (_, providers) = call_json(&h.router, get_plain("/teams/auth/providers")).await;
    assert_eq!(providers["oidc"]["name"], "Example IdP");

    // First time: the verified email finds the row and pins the subject.
    let secret = "app-secret-one";
    let w = walk_the_browser(&h, secret, "Hoshino's MacBook").await;
    assert_eq!(w.status, StatusCode::OK, "{}", w.page);
    assert!(w.page.contains("Signed in."), "{}", w.page);
    // ...and not who: the page is keyed by an id that is not a secret.
    assert!(!w.page.contains("hoshino"), "{}", w.page);
    assert!(w.grant.is_some(), "the loopback redirect carried a grant");

    // The secret alone collects nothing, and neither does the grant
    // alone: the app that started it and the machine that finished it
    // are both asked for.
    let (status, _) = collect(&h, &w.id, secret, "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = collect(&h, &w.id, "not-the-secret", w.grant()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, session) = collect(&h, &w.id, secret, w.grant()).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    assert_eq!(session["login"], "hoshino");
    assert_eq!(session["display_name"], "hoshino");
    // The stable id a client keys its store by, and the tenant it
    // belongs to — opaque, present, and the instance's own.
    let instance_id = session["instance_id"].as_str().unwrap().to_string();
    assert_eq!(instance_id.len(), 32, "{instance_id}");
    assert_eq!(session["tenant_id"], instance_id);
    let token = session["token"].as_str().unwrap().to_string();

    // Collected once. The done page still says who, for the tab that
    // arrives after the app has already collected.
    let (status, _) = collect(&h, &w.id, secret, w.grant()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, page) = call(
        &h.router,
        get_plain(&format!("/teams/auth/oidc/attempts/{}/done", w.id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Signed in."), "{page}");

    // The session is an ordinary one: it mints a device token like any
    // other, which is the whole of what #204 promised #163.
    let (status, minted) = call_json(
        &h.router,
        post_authed(
            "/teams/auth/device",
            &token,
            serde_json::json!({ "label": "Hoshino's MacBook" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{minted}");
    let (status, session) = call_json(
        &h.router,
        post(
            "/teams/auth/device/login",
            serde_json::json!({ "token": minted["token"] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["login"], "hoshino");
    assert_eq!(
        session["instance_id"], instance_id,
        "one instance, one id, whichever arm minted the session"
    );

    // Second time: the provider says a different email for the same
    // subject, and the pinned subject is what answers.
    h.provider
        .as_ref()
        .unwrap()
        .next_person("sub-hoshino", "moved@example.com", true);
    let secret = "app-secret-two";
    let w = walk_the_browser(&h, secret, "Hoshino's phone").await;
    assert_eq!(w.status, StatusCode::OK);
    let (status, session) = collect(&h, &w.id, secret, w.grant()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["login"], "hoshino");

    // And the address, in somebody else's hands, matches nothing.
    h.provider
        .as_ref()
        .unwrap()
        .next_person("sub-somebody", "hoshino@example.com", true);
    let secret = "app-secret-three";
    let w = walk_the_browser(&h, secret, "A stranger's laptop").await;
    assert_eq!(w.status, StatusCode::UNAUTHORIZED, "{}", w.page);
    assert!(w.grant.is_none(), "a refusal carries no grant");
    let (status, _) = collect(&h, &w.id, secret, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_unverified_email_and_an_unknown_person_are_the_same_refusal() {
    let h = harness(true).await;
    bound_account(&h, "hoshino", "hoshino@example.com").await;
    let provider = h.provider.as_ref().unwrap();

    provider.next_person("sub-hoshino", "hoshino@example.com", false);
    let w = walk_the_browser(&h, "s1", "MacBook").await;
    assert_eq!(w.status, StatusCode::UNAUTHORIZED, "{}", w.page);
    let (status, body) = collect(&h, &w.id, "s1", "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let unverified = body.to_string();

    provider.next_person("sub-nobody", "nobody@example.com", true);
    let w = walk_the_browser(&h, "s2", "MacBook").await;
    assert_eq!(w.status, StatusCode::UNAUTHORIZED);
    let (status, body) = collect(&h, &w.id, "s2", "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body.to_string(), unverified, "one arm, one body");

    // Nothing was pinned by either.
    assert_eq!(
        OidcIdentities::new(h.isle.clone())
            .binding(
                h.ctx
                    .auth
                    .account_by_login("hoshino")
                    .await
                    .unwrap()
                    .unwrap()
                    .user_id
            )
            .await
            .unwrap()
            .unwrap()
            .subject,
        None
    );
    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_token_that_is_wrong_in_any_one_way_is_refused() {
    let h = harness(true).await;
    bound_account(&h, "hoshino", "hoshino@example.com").await;
    let provider = h.provider.as_ref().unwrap();
    for (n, tamper) in [
        Tamper::WrongIssuer,
        Tamper::WrongAudience,
        Tamper::Expired,
        Tamper::WrongNonce,
    ]
    .into_iter()
    .enumerate()
    {
        provider.tamper(tamper);
        let secret = format!("s{n}");
        let w = walk_the_browser(&h, &secret, "MacBook").await;
        assert_eq!(w.status, StatusCode::UNAUTHORIZED, "tamper {n}: {}", w.page);
        let (status, _) = collect(&h, &w.id, &secret, "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "tamper {n}");
    }
    // Every one of them reached the exchange: the refusal is the
    // token's, not the provider's.
    assert_eq!(provider.exchanges(), 4);

    provider.tamper(Tamper::None);
    let w = walk_the_browser(&h, "good", "MacBook").await;
    assert_eq!(w.status, StatusCode::OK);
    let (status, _) = collect(&h, &w.id, "good", w.grant()).await;
    assert_eq!(status, StatusCode::OK);
    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn nothing_collects_before_the_browser_comes_back_and_a_refusal_reaches_the_app() {
    let h = harness(true).await;
    bound_account(&h, "hoshino", "hoshino@example.com").await;

    let (status, attempt) = call_json(
        &h.router,
        post(
            "/teams/auth/oidc/attempts",
            serde_json::json!({
                "collector": sha256_hex("mine"),
                "label": "MacBook",
                "loopback_port": LOOPBACK_PORT,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = attempt["attempt_id"].as_str().unwrap().to_string();

    // Not finished in the browser: there is no grant yet, so nothing
    // collects — not the right secret with a guessed grant, not the
    // wrong secret, not an id nothing names.
    let (status, _) = collect(&h, &id, "mine", "guessed").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = collect(&h, &id, "theirs", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = collect(&h, "no-such-attempt", "mine", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The done page, before anything has happened, says nothing.
    let (status, _, _) = call(
        &h.router,
        get_plain(&format!("/teams/auth/oidc/attempts/{id}/done")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The provider sending the browser back with an error is a refusal,
    // and the browser is still sent to the app so it can stop waiting.
    let (status, headers, _) = call(
        &h.router,
        get_plain(&format!(
            "/teams/auth/oidc/callback?state={id}&error=access_denied"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        location(&headers),
        format!("http://127.0.0.1:{LOOPBACK_PORT}/teams/auth/oidc/loopback?attempt={id}&refused=1")
    );
    let (status, _, page) = call(
        &h.router,
        get_plain(&format!("/teams/auth/oidc/attempts/{id}/done")),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{page}");
    let (status, _) = collect(&h, &id, "mine", "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // A second callback for the same attempt changes nothing.
    let (status, _, _) = call(
        &h.router,
        get_plain(&format!("/teams/auth/oidc/callback?state={id}&code=late")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A blank label, a collector that is not a digest, and a port the
    // app cannot be listening on are refused before anything is stored.
    for body in [
        serde_json::json!({ "collector": sha256_hex("x"), "label": "  ", "loopback_port": 1 }),
        serde_json::json!({ "collector": "not-hex", "label": "MacBook", "loopback_port": 1 }),
        serde_json::json!({ "collector": sha256_hex("x"), "label": "MacBook", "loopback_port": 0 }),
    ] {
        let (status, _) = call_json(&h.router, post("/teams/auth/oidc/attempts", body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_password_arm_now_says_the_login_too_and_a_locked_account_never_takes_one() {
    let h = harness(true).await;
    bound_account(&h, "hoshino", "hoshino@example.com").await;
    h.ctx
        .auth
        .create_account("kanade", "Kanade", GOOD, false, now_ms())
        .await
        .unwrap();
    let (status, session) = call_json(
        &h.router,
        post(
            "/teams/auth/login",
            serde_json::json!({ "login": "kanade", "password": GOOD }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["login"], "kanade");
    for password in ["!", "", GOOD] {
        let (status, _) = call_json(
            &h.router,
            post(
                "/teams/auth/login",
                serde_json::json!({ "login": "hoshino", "password": password }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_instance_without_a_provider_is_what_it_was() {
    let h = harness(false).await;
    let (status, providers) = call_json(&h.router, get_plain("/teams/auth/providers")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(providers["oidc"].is_null(), "{providers}");
    let (status, body) = call_json(
        &h.router,
        post(
            "/teams/auth/oidc/attempts",
            serde_json::json!({
                "collector": sha256_hex("x"),
                "label": "MacBook",
                "loopback_port": LOOPBACK_PORT,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, _, _) = call(&h.router, get_plain("/teams/auth/oidc/attempts/anything")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = collect(&h, "anything", "x", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The binding table exists and holds nothing.
    let rows: i64 = h
        .isle
        .call(|conn| conn.query_row("SELECT count(*) FROM oidc_identity", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(rows, 0);
    h.driver.shutdown().await.unwrap();
}
