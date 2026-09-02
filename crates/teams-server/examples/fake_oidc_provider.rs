//! A stand-in identity provider for end-to-end runs (#163).
//!
//! The provider `teams-server`'s own sign-in suite builds in-process
//! for its tests (`tests/oidc_sign_in_e2e.rs`, which says what it
//! serves), as a process and with less: this one vouches for the one
//! person it was started as, to whoever presents the right client
//! secret, cannot switch the person, and varies nothing in the token
//! but the one claim `--email-unverified` names. Nothing here is a
//! provider anybody should trust, which is
//! what a fixture is for and why it is an example rather than a
//! binary.
//!
//! Started by `wdio.teams.conf.ts` beside the team server, so the
//! app's browser round trip crosses three real processes: the app's
//! loopback listener, the team server as the OAuth client, and this.
//!
//! Prints one line to stderr once it is serving, which is what the
//! harness waits for rather than probing the port.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine as _;
use clap::Parser;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use p256::pkcs8::{EncodePrivateKey, LineEnding};
use rand::TryRngCore as _;
use sha2::{Digest, Sha256};

/// A stand-in OIDC provider for end-to-end runs. Consents as one
/// person, to one client.
#[derive(Parser)]
struct Args {
    /// Listen port on 127.0.0.1.
    #[arg(long)]
    port: u16,
    /// The client id the team server was registered under.
    #[arg(long)]
    client_id: String,
    /// The client secret the team server presents at the exchange.
    #[arg(long)]
    client_secret: String,
    /// The `sub` every ID token carries.
    #[arg(long, default_value = "e2e-subject")]
    subject: String,
    /// The `email` every ID token carries.
    #[arg(long)]
    email: String,
    /// Say the email is not verified — the claim the server refuses
    /// to bind on. Absent, the token says it is.
    #[arg(long)]
    email_unverified: bool,
}

struct Issued {
    nonce: String,
    challenge: String,
    redirect_uri: String,
}

struct Provider {
    base: String,
    args: Args,
    key_pem: String,
    jwk: serde_json::Value,
    codes: Mutex<HashMap<String, Issued>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let secret = loop {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.try_fill_bytes(&mut bytes)?;
        if let Ok(key) = p256::SecretKey::from_slice(&bytes) {
            break key;
        }
    };
    let key_pem = secret.to_pkcs8_pem(LineEnding::LF)?.to_string();
    let mut jwk: serde_json::Value = serde_json::from_str(&secret.public_key().to_jwk_string())?;
    jwk["kid"] = "run-key".into();
    jwk["use"] = "sig".into();
    jwk["alg"] = "ES256".into();

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let base = format!("http://{addr}");
    let provider = Arc::new(Provider {
        base: base.clone(),
        args,
        key_pem,
        jwk,
        codes: Mutex::new(HashMap::new()),
    });
    let router = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .with_state(provider);
    eprintln!("fake-oidc-provider: {base}");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn discovery(State(p): State<Arc<Provider>>) -> Json<serde_json::Value> {
    let base = &p.base;
    Json(serde_json::json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "jwks_uri": format!("{base}/jwks"),
        "response_types_supported": ["code"],
        "id_token_signing_alg_values_supported": ["ES256"],
    }))
}

async fn jwks(State(p): State<Arc<Provider>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "keys": [p.jwk.clone()] }))
}

/// Consents without asking, which is the whole of what makes this a
/// fixture: the person is whoever the process was started as.
async fn authorize(
    State(p): State<Arc<Provider>>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let get = |key: &str| query.get(key).cloned().unwrap_or_default();
    if get("response_type") != "code"
        || get("client_id") != p.args.client_id
        || get("code_challenge_method") != "S256"
    {
        return (
            StatusCode::BAD_REQUEST,
            "not an authorization this provider answers",
        )
            .into_response();
    }
    let code = {
        let mut codes = p.codes.lock().expect("codes lock");
        let code = format!("code-{}", codes.len());
        codes.insert(
            code.clone(),
            Issued {
                nonce: get("nonce"),
                challenge: get("code_challenge"),
                redirect_uri: get("redirect_uri"),
            },
        );
        code
    };
    Redirect::to(&format!(
        "{}?code={code}&state={}",
        get("redirect_uri"),
        get("state")
    ))
    .into_response()
}

async fn token(
    State(p): State<Arc<Provider>>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", p.args.client_id, p.args.client_secret))
    );
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if presented != expected {
        return (StatusCode::UNAUTHORIZED, "client authentication failed").into_response();
    }
    if form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return (StatusCode::BAD_REQUEST, "unsupported grant").into_response();
    }
    let issued = form
        .get("code")
        .and_then(|code| p.codes.lock().expect("codes lock").remove(code));
    let Some(issued) = issued else {
        return (StatusCode::BAD_REQUEST, "unknown code").into_response();
    };
    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    if challenge != issued.challenge {
        return (StatusCode::BAD_REQUEST, "PKCE verifier does not match").into_response();
    }
    if form.get("redirect_uri") != Some(&issued.redirect_uri) {
        return (StatusCode::BAD_REQUEST, "redirect_uri does not match").into_response();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_secs() as i64;
    let claims = serde_json::json!({
        "iss": p.base,
        "aud": p.args.client_id,
        "sub": p.args.subject,
        "iat": now - 5,
        "exp": now + 300,
        "nonce": issued.nonce,
        "email": p.args.email,
        "email_verified": !p.args.email_unverified,
    });
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some("run-key".into());
    let id_token = match EncodingKey::from_ec_pem(p.key_pem.as_bytes())
        .and_then(|key| jsonwebtoken::encode(&header, &claims, &key))
    {
        Ok(token) => token,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not sign: {err}"),
            )
                .into_response();
        }
    };
    Json(serde_json::json!({
        "id_token": id_token,
        "access_token": "opaque",
        "token_type": "Bearer",
    }))
    .into_response()
}
