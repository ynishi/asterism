//! OIDC sign-in (#163): the instance is the provider's OAuth client,
//! and the invite still says whether.
//!
//! Two halves, kept apart because they answer different questions.
//! [`OidcClient`] talks to the provider: discovery, the authorization
//! URL a browser is sent to, the code exchange, and the check of the
//! ID token that comes back — signature against the provider's
//! published keys, issuer and audience against configuration, expiry
//! against the clock, nonce against the attempt. [`OidcIdentities`]
//! talks to the database: which account a verified identity resolves
//! to, and the pinning that makes that answer stable.
//!
//! ## Why the server is the client
//!
//! The desktop app never speaks to the provider. It opens a browser at
//! this instance, and this instance runs the whole authorization-code
//! exchange as a confidential client — client secret here, never on a
//! device; one callback URL, registered once by whoever hosts the
//! instance. That is the backend-for-frontend shape
//! draft-ietf-oauth-v2-1 §2.1 recommends for a native application
//! that wishes to use client credentials, and what it buys is stated
//! where it is used: a provider outage stops new sign-ins and nothing
//! else, the device listing is the instance's, and a hosted deployment
//! with several providers changes nothing on a member's machine.
//!
//! ## What a verified token is not
//!
//! Proof of membership. The provider answers who; the binding row
//! answers whether that person holds an account here, and the roster
//! answers whether they belong to a team. A token that verifies and
//! resolves to nobody is refused with the same one-armed answer a
//! wrong password gets, and nothing here provisions an account from a
//! claim.
//!
//! ## Pinning
//!
//! An admin binds an account to an email at the provider. The first
//! sign-in whose token carries that email — **verified**, or it does
//! not count — pins the token's `sub` to the row, and from then on the
//! subject is what resolves and the email is inert. A provider that
//! later hands the address to somebody else hands them a different
//! subject; an unverified email claim never matches anything. The two
//! account-takeover shapes the issue names are closed by those two
//! rules, and `sqlite::migrations::V11_OIDC_IDENTITY` is where the
//! indexes make them structural.

use base64::Engine as _;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use rand::TryRngCore;
use rusqlite::{OptionalExtension, params};
use rusqlite_isle::AsyncIsle;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use teams_core::DomainError;
use uuid::Uuid;

use crate::auth::password::AccountRecord;
use crate::sqlite::map::infra_err;

/// The scopes every authorization asks for. `openid` is what makes
/// the answer an ID token; `email` is what the first sign-in matches
/// on; `profile` is where a display name would come from.
const SCOPES: &str = "openid email profile";

/// The signature algorithms an ID token may carry: the RSA, RSA-PSS,
/// ECDSA and EdDSA families the signature crate verifies with a
/// published key. Never an HMAC family, whose "key" would be the
/// client secret and whose signature anybody holding it could forge.
/// What is absent is what the crate does not offer (ES512), not a
/// choice made here.
const ALLOWED_ALGORITHMS: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::EdDSA,
];

/// How the instance reaches its provider — what `teams-server serve`
/// takes from its arguments and the environment.
#[derive(Clone)]
pub struct OidcConfig {
    /// The provider's issuer URL, exactly as the ID token will name
    /// it in `iss`. Discovery is at `<issuer>/.well-known/openid-configuration`.
    pub issuer: String,
    /// The client id the provider registered this instance under.
    pub client_id: String,
    /// The client secret. Held here and nowhere else — the app never
    /// sees it, which is the point of the instance being the client.
    pub client_secret: String,
    /// Where the provider sends the browser back: this instance's
    /// callback route on its public URL.
    pub redirect_url: String,
    /// What a connect form calls the provider — "Google", "Okta",
    /// whatever the person hosting wrote.
    pub display_name: String,
}

impl std::fmt::Debug for OidcConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcConfig")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("client_secret", &"<not shown>")
            .field("redirect_url", &self.redirect_url)
            .field("display_name", &self.display_name)
            .finish()
    }
}

/// The three endpoints discovery names, and nothing else it says.
#[derive(Clone, Debug, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

/// What the token endpoint answers with, of which one field matters.
#[derive(Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
}

/// The claims this instance reads. Signature, `iss`, `aud` and `exp`
/// are the library's to check before any of these is looked at.
#[derive(Deserialize)]
struct IdTokenClaims {
    sub: String,
    nonce: Option<String>,
    email: Option<String>,
    email_verified: Option<bool>,
    name: Option<String>,
}

/// Who a provider vouched for, after every check passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedIdentity {
    /// The provider, as the token named it — equal to the configured
    /// issuer by construction, carried so the binding lookup keys on
    /// what was verified rather than on what was configured.
    pub issuer: String,
    /// The provider's stable id for the person.
    pub subject: String,
    /// The email claim, lower-cased, if the token carried one.
    pub email: Option<String>,
    /// Whether the provider said it had verified that email. Only a
    /// verified one may pin a subject.
    pub email_verified: bool,
    /// The `name` claim, if any. Read for nothing yet.
    pub name: Option<String>,
}

/// What a code exchange comes to.
#[derive(Debug)]
pub enum Exchange {
    /// Every check passed; this is who the provider said.
    Verified(VerifiedIdentity),
    /// The provider answered and the answer does not authenticate
    /// anybody — a refused code, a token that does not verify, a nonce
    /// that is not the attempt's. The reason is for the instance's log;
    /// the browser and the app get the one-armed answer.
    Refused(&'static str),
}

/// The provider-facing half: discovery, the authorization URL, the
/// exchange, the token check.
pub struct OidcClient {
    config: OidcConfig,
    http: reqwest::Client,
    discovery: tokio::sync::Mutex<Option<Discovery>>,
    keys: tokio::sync::Mutex<Option<JwkSet>>,
}

impl std::fmt::Debug for OidcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl OidcClient {
    /// Points at a provider. No request is made here — discovery runs
    /// on the first authorization, so an instance that starts while
    /// its provider is down still starts.
    pub fn new(config: OidcConfig) -> Self {
        let config = OidcConfig {
            issuer: config.issuer.trim().trim_end_matches('/').to_string(),
            ..config
        };
        Self {
            config,
            http: reqwest::Client::new(),
            discovery: tokio::sync::Mutex::new(None),
            keys: tokio::sync::Mutex::new(None),
        }
    }

    /// The configured issuer, trailing slash trimmed.
    pub fn issuer(&self) -> &str {
        &self.config.issuer
    }

    /// What a connect form calls this provider.
    pub fn display_name(&self) -> &str {
        &self.config.display_name
    }

    /// 256 random bits, hex — the shape every attempt id, state and
    /// nonce here takes.
    pub fn random_token() -> Result<String, DomainError> {
        Ok(hex(&random_bytes()?))
    }

    /// A PKCE verifier and its S256 challenge (RFC 7636 §4).
    pub fn pkce_pair() -> Result<(String, String), DomainError> {
        let verifier = base64url(&random_bytes()?);
        let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
        Ok((verifier, challenge))
    }

    /// The URL a browser is sent to. `state` is the attempt's id, which
    /// is how the callback finds it again; `nonce` is what the ID token
    /// must echo; `code_challenge` is the PKCE half the exchange will
    /// prove it knows the verifier of.
    pub async fn authorization_url(
        &self,
        state: &str,
        nonce: &str,
        code_challenge: &str,
    ) -> Result<String, DomainError> {
        let discovery = self.discovery().await?;
        let url = reqwest::Url::parse_with_params(
            &discovery.authorization_endpoint,
            &[
                ("response_type", "code"),
                ("client_id", self.config.client_id.as_str()),
                ("redirect_uri", self.config.redirect_url.as_str()),
                ("scope", SCOPES),
                ("state", state),
                ("nonce", nonce),
                ("code_challenge", code_challenge),
                ("code_challenge_method", "S256"),
            ],
        )
        .map_err(|e| {
            DomainError::Infra(anyhow::anyhow!(
                "the provider's authorization endpoint is not a URL: {e}"
            ))
        })?;
        Ok(url.to_string())
    }

    /// Exchanges the code the callback carried for an ID token and
    /// checks it. `Err` is the provider not answering; a provider that
    /// answered with anything but a token this instance accepts is
    /// [`Exchange::Refused`].
    pub async fn exchange(
        &self,
        code: &str,
        pkce_verifier: &str,
        nonce: &str,
    ) -> Result<Exchange, DomainError> {
        let discovery = self.discovery().await?;
        let response = self
            .http
            .post(&discovery.token_endpoint)
            .basic_auth(&self.config.client_id, Some(&self.config.client_secret))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", self.config.redirect_url.as_str()),
                ("client_id", self.config.client_id.as_str()),
                ("code_verifier", pkce_verifier),
            ])
            .send()
            .await
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("token endpoint unreachable: {e}")))?;
        if !response.status().is_success() {
            return Ok(Exchange::Refused("the provider refused the code"));
        }
        let token: TokenResponse = response
            .json()
            .await
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("token response undecodable: {e}")))?;
        let Some(id_token) = token.id_token else {
            return Ok(Exchange::Refused("the provider answered with no ID token"));
        };
        self.verify(&id_token, nonce).await
    }

    /// Checks an ID token: signature against a published key, issuer
    /// and audience against configuration, expiry against the clock,
    /// nonce against the attempt.
    async fn verify(&self, id_token: &str, nonce: &str) -> Result<Exchange, DomainError> {
        let Ok(header) = decode_header(id_token) else {
            return Ok(Exchange::Refused("the ID token's header does not parse"));
        };
        if !ALLOWED_ALGORITHMS.contains(&header.alg) {
            return Ok(Exchange::Refused(
                "the ID token's algorithm is not one a provider signs with",
            ));
        }
        let Some(key) = self.decoding_key(header.kid.as_deref(), header.alg).await? else {
            return Ok(Exchange::Refused("no published key signs this ID token"));
        };
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[self.config.issuer.as_str()]);
        validation.set_audience(&[self.config.client_id.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        let claims = match decode::<IdTokenClaims>(id_token, &key, &validation) {
            Ok(data) => data.claims,
            Err(_) => return Ok(Exchange::Refused("the ID token did not verify")),
        };
        if claims.nonce.as_deref() != Some(nonce) {
            return Ok(Exchange::Refused(
                "the ID token's nonce is not this attempt's",
            ));
        }
        Ok(Exchange::Verified(VerifiedIdentity {
            issuer: self.config.issuer.clone(),
            subject: claims.sub,
            email: claims.email.map(|it| normalize_email(&it)),
            email_verified: claims.email_verified.unwrap_or(false),
            name: claims.name,
        }))
    }

    /// The key the header names, fetching the set once if it is not in
    /// the cached one — a provider that rotated its keys publishes the
    /// new one before signing with it, so one refetch is the whole of
    /// rotation handling.
    async fn decoding_key(
        &self,
        kid: Option<&str>,
        alg: Algorithm,
    ) -> Result<Option<DecodingKey>, DomainError> {
        if let Some(key) = self.cached_key(kid, alg).await? {
            return Ok(Some(key));
        }
        let discovery = self.discovery().await?;
        let set: JwkSet = self
            .http
            .get(&discovery.jwks_uri)
            .send()
            .await
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("JWKS unreachable: {e}")))?
            .error_for_status()
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("JWKS refused: {e}")))?
            .json()
            .await
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("JWKS undecodable: {e}")))?;
        *self.keys.lock().await = Some(set);
        self.cached_key(kid, alg).await
    }

    async fn cached_key(
        &self,
        kid: Option<&str>,
        alg: Algorithm,
    ) -> Result<Option<DecodingKey>, DomainError> {
        let keys = self.keys.lock().await;
        let Some(set) = keys.as_ref() else {
            return Ok(None);
        };
        let candidates: Vec<_> = set
            .keys
            .iter()
            .filter(|jwk| key_fits(jwk, alg))
            .filter(|jwk| match kid {
                Some(kid) => jwk.common.key_id.as_deref() == Some(kid),
                None => true,
            })
            .collect();
        // Without a `kid` the set has to be unambiguous; two keys that
        // both fit is a provider this instance will not guess between.
        let Some(jwk) = (match (kid, candidates.as_slice()) {
            (Some(_), [jwk, ..]) | (None, [jwk]) => Some(*jwk),
            _ => None,
        }) else {
            return Ok(None);
        };
        DecodingKey::from_jwk(jwk)
            .map(Some)
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("published key unusable: {e}")))
    }

    /// The provider's discovery document, fetched once per process. A
    /// provider that moves an endpoint is a provider whose operators
    /// announce it, and a restart is what picks it up; the keys are the
    /// half that rotates unannounced, and [`Self::decoding_key`] is
    /// where that is handled.
    async fn discovery(&self) -> Result<Discovery, DomainError> {
        let mut cached = self.discovery.lock().await;
        if let Some(found) = cached.as_ref() {
            return Ok(found.clone());
        }
        let url = format!("{}/.well-known/openid-configuration", self.config.issuer);
        let found: Discovery = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| {
                DomainError::Infra(anyhow::anyhow!("provider discovery unreachable: {e}"))
            })?
            .error_for_status()
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("provider discovery refused: {e}")))?
            .json()
            .await
            .map_err(|e| {
                DomainError::Infra(anyhow::anyhow!("provider discovery undecodable: {e}"))
            })?;
        // The document has to be the configured issuer's own: a
        // discovery URL that answers for another issuer is the mix-up
        // RFC 9700 §4.4 describes, caught here before any token is
        // trusted on its say-so.
        if found.issuer.trim_end_matches('/') != self.config.issuer {
            return Err(DomainError::Infra(anyhow::anyhow!(
                "provider discovery names issuer {:?}, not the configured {:?}",
                found.issuer,
                self.config.issuer
            )));
        }
        *cached = Some(found.clone());
        Ok(found)
    }
}

/// Whether a published key could have made a signature under `alg`.
fn key_fits(jwk: &jsonwebtoken::jwk::Jwk, alg: Algorithm) -> bool {
    matches!(
        (&jwk.algorithm, alg),
        (
            AlgorithmParameters::RSA(_),
            Algorithm::RS256
                | Algorithm::RS384
                | Algorithm::RS512
                | Algorithm::PS256
                | Algorithm::PS384
                | Algorithm::PS512,
        ) | (
            AlgorithmParameters::EllipticCurve(_),
            Algorithm::ES256 | Algorithm::ES384
        ) | (AlgorithmParameters::OctetKeyPair(_), Algorithm::EdDSA)
    )
}

/// The database half: the binding rows, and the resolve that pins.
#[derive(Clone)]
pub struct OidcIdentities {
    isle: AsyncIsle,
}

impl std::fmt::Debug for OidcIdentities {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcIdentities").finish_non_exhaustive()
    }
}

/// One account's binding, as an admin reads it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityBinding {
    /// The provider the binding names.
    pub issuer: String,
    /// The address the first sign-in matches on.
    pub email: String,
    /// The pinned subject, once a sign-in has pinned one.
    pub subject: Option<String>,
}

impl OidcIdentities {
    /// Wraps the same writer handle the credential store uses.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }

    /// Binds an account to an email at a provider — what an admin does
    /// when provisioning. An account already bound is rebound, and
    /// **rebinding unpins**: the next verified sign-in with the new
    /// address pins afresh, which is what an admin correcting a typo
    /// wants and what one moving a person between providers needs.
    ///
    /// An address another account already holds at the same issuer is
    /// refused: two rows the first sign-in could pin is a row it
    /// cannot choose between.
    pub async fn bind_email(
        &self,
        user_id: Uuid,
        issuer: &str,
        email: &str,
    ) -> Result<(), DomainError> {
        let issuer = issuer.trim().trim_end_matches('/').to_string();
        if issuer.is_empty() {
            return Err(DomainError::Validation("issuer is blank".into()));
        }
        let email = normalize_email(email);
        if !email.contains('@') {
            return Err(DomainError::Validation(format!(
                "{email:?} is not an email address"
            )));
        }
        self.isle
            .call(move |conn| {
                let known: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM user_account WHERE user_id = ?1)",
                    params![user_id],
                    |row| row.get(0),
                )?;
                if !known {
                    return Ok(Err(DomainError::Validation(format!(
                        "user {user_id} has no account on this instance"
                    ))));
                }
                let taken: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM oidc_identity
                     WHERE issuer = ?1 AND email = ?2 AND user_id <> ?3)",
                    params![issuer, email, user_id],
                    |row| row.get(0),
                )?;
                if taken {
                    return Ok(Err(DomainError::Validation(format!(
                        "{email:?} at {issuer:?} is already bound to another account"
                    ))));
                }
                conn.execute(
                    "INSERT INTO oidc_identity (user_id, issuer, email, subject, bound_at)
                     VALUES (?1, ?2, ?3, NULL, NULL)
                     ON CONFLICT(user_id) DO UPDATE SET
                        issuer = excluded.issuer,
                        email = excluded.email,
                        subject = NULL,
                        bound_at = NULL",
                    params![user_id, issuer, email],
                )?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)?
    }

    /// The binding an account holds, if any.
    pub async fn binding(&self, user_id: Uuid) -> Result<Option<IdentityBinding>, DomainError> {
        self.isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT issuer, email, subject FROM oidc_identity WHERE user_id = ?1",
                    params![user_id],
                    |row| {
                        Ok(IdentityBinding {
                            issuer: row.get(0)?,
                            email: row.get(1)?,
                            subject: row.get(2)?,
                        })
                    },
                )
                .optional()
            })
            .await
            .map_err(infra_err)
    }

    /// The account a verified identity resolves to, or `None` — the
    /// one-armed answer, for the reason the password arm gives one.
    ///
    /// By subject first: a pinned row answers regardless of what the
    /// token says about email. Then, for a token whose email the
    /// provider has verified, by the address of a row nothing has
    /// pinned yet — and that lookup **pins**, in the same closure, so
    /// a resolve either answers and records or does neither. An
    /// unverified email is not looked at.
    pub async fn resolve(
        &self,
        identity: &VerifiedIdentity,
        now_ms: i64,
    ) -> Result<Option<AccountRecord>, DomainError> {
        let issuer = identity.issuer.clone();
        let subject = identity.subject.clone();
        let email = identity
            .email_verified
            .then(|| identity.email.clone())
            .flatten();
        self.isle
            .call(move |conn| {
                let by_subject = conn
                    .query_row(
                        "SELECT a.user_id, a.login, a.display_name, a.is_admin
                         FROM oidc_identity i JOIN user_account a ON a.user_id = i.user_id
                         WHERE i.issuer = ?1 AND i.subject = ?2",
                        params![issuer, subject],
                        account_from_row,
                    )
                    .optional()?;
                if by_subject.is_some() {
                    return Ok(by_subject);
                }
                let Some(email) = email else {
                    return Ok(None);
                };
                let unpinned = conn
                    .query_row(
                        "SELECT a.user_id, a.login, a.display_name, a.is_admin
                         FROM oidc_identity i JOIN user_account a ON a.user_id = i.user_id
                         WHERE i.issuer = ?1 AND i.email = ?2 AND i.subject IS NULL",
                        params![issuer, email],
                        account_from_row,
                    )
                    .optional()?;
                let Some(account) = unpinned else {
                    return Ok(None);
                };
                conn.execute(
                    "UPDATE oidc_identity SET subject = ?2, bound_at = ?3 WHERE user_id = ?1",
                    params![account.user_id, subject, now_ms],
                )?;
                Ok(Some(account))
            })
            .await
            .map_err(infra_err)
    }
}

/// A `user_id, login, display_name, is_admin` row — the column order
/// `password::account_from_row` spells, repeated here because that one
/// is private to its module and this one reads the same four columns
/// through a join.
fn account_from_row(row: &rusqlite::Row<'_>) -> Result<AccountRecord, rusqlite::Error> {
    Ok(AccountRecord {
        user_id: row.get(0)?,
        login: row.get(1)?,
        display_name: row.get(2)?,
        admin: row.get(3)?,
    })
}

/// Lower-cased and trimmed: the one form an address is stored and
/// compared in.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// SHA-256 of a string, hex — how an attempt's collector is compared
/// against the secret the app kept.
pub fn sha256_hex(input: &str) -> String {
    hex(&Sha256::digest(input.as_bytes()))
}

fn random_bytes() -> Result<[u8; 32], DomainError> {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("OS CSPRNG failure: {e}")))?;
    Ok(bytes)
}

fn base64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
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
    use crate::auth::password::PasswordAuth;
    use rusqlite_isle::AsyncIsleDriver;

    const T0: i64 = 1_755_000_000_000;
    const ISSUER: &str = "https://issuer.example";

    async fn fixture() -> (PasswordAuth, OidcIdentities, AsyncIsle, AsyncIsleDriver) {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        (
            PasswordAuth::new(isle.clone()),
            OidcIdentities::new(isle.clone()),
            isle,
            driver,
        )
    }

    fn identity(subject: &str, email: &str, verified: bool) -> VerifiedIdentity {
        VerifiedIdentity {
            issuer: ISSUER.into(),
            subject: subject.into(),
            email: Some(normalize_email(email)),
            email_verified: verified,
            name: None,
        }
    }

    #[test]
    fn a_pkce_challenge_is_the_s256_of_its_verifier() {
        let (verifier, challenge) = OidcClient::pkce_pair().unwrap();
        assert_eq!(verifier.len(), 43);
        assert_eq!(challenge, base64url(&Sha256::digest(verifier.as_bytes())));
        assert_ne!(OidcClient::pkce_pair().unwrap().0, verifier);
    }

    #[tokio::test]
    async fn the_first_verified_sign_in_pins_the_subject_and_the_email_is_inert_after() {
        let (auth, identities, _isle, driver) = fixture().await;
        let hoshino = auth
            .create_account_locked("hoshino", "Hoshino", false, T0)
            .await
            .unwrap();
        identities
            .bind_email(hoshino, ISSUER, " Hoshino@Example.com ")
            .await
            .unwrap();

        // An unverified email pins nothing.
        assert!(
            identities
                .resolve(&identity("sub-1", "hoshino@example.com", false), T0)
                .await
                .unwrap()
                .is_none()
        );
        // A verified one pins.
        let found = identities
            .resolve(&identity("sub-1", "hoshino@example.com", true), T0 + 1)
            .await
            .unwrap()
            .expect("pinned by email");
        assert_eq!(found.user_id, hoshino);
        assert_eq!(
            identities.binding(hoshino).await.unwrap().unwrap().subject,
            Some("sub-1".into())
        );
        // From then on the subject answers, whatever the email says.
        let again = identities
            .resolve(&identity("sub-1", "moved@example.com", true), T0 + 2)
            .await
            .unwrap()
            .expect("resolved by subject");
        assert_eq!(again.user_id, hoshino);
        // ...and the address, re-issued to somebody else, matches nothing.
        assert!(
            identities
                .resolve(&identity("sub-2", "hoshino@example.com", true), T0 + 3)
                .await
                .unwrap()
                .is_none()
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn rebinding_unpins_and_a_taken_address_is_refused() {
        let (auth, identities, _isle, driver) = fixture().await;
        let hoshino = auth
            .create_account_locked("hoshino", "Hoshino", false, T0)
            .await
            .unwrap();
        let kanade = auth
            .create_account_locked("kanade", "Kanade", false, T0)
            .await
            .unwrap();
        identities
            .bind_email(hoshino, ISSUER, "hoshino@example.com")
            .await
            .unwrap();
        identities
            .resolve(&identity("sub-1", "hoshino@example.com", true), T0)
            .await
            .unwrap()
            .expect("pinned");
        assert!(matches!(
            identities
                .bind_email(kanade, ISSUER, "hoshino@example.com")
                .await,
            Err(DomainError::Validation(_))
        ));
        identities
            .bind_email(hoshino, ISSUER, "h.new@example.com")
            .await
            .unwrap();
        assert_eq!(
            identities.binding(hoshino).await.unwrap().unwrap().subject,
            None
        );
        assert!(matches!(
            identities
                .bind_email(hoshino, ISSUER, "not-an-address")
                .await,
            Err(DomainError::Validation(_))
        ));
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_locked_account_never_answers_the_password_arm() {
        use teams_core::port::auth::CredentialVerifier;
        let (auth, _identities, _isle, driver) = fixture().await;
        auth.create_account_locked("hoshino", "Hoshino", false, T0)
            .await
            .unwrap();
        assert_eq!(auth.verify("hoshino", "!").await.unwrap(), None);
        assert_eq!(auth.verify("hoshino", "").await.unwrap(), None);
        assert_eq!(
            auth.verify("hoshino", "correct horse battery staple")
                .await
                .unwrap(),
            None
        );
        driver.shutdown().await.unwrap();
    }

    #[test]
    fn a_published_rsa_key_becomes_a_decoding_key_and_an_hmac_key_never_fits() {
        // The RSA key is the public one RFC 7517 Appendix A.1 prints —
        // an IETF RFC, redistributable under the IETF Trust Legal
        // Provisions, and public material with no private half. The
        // `oct` key beside it is made up here: its `k` is the base64url
        // of "not-a-key", which is enough for the one thing it is for —
        // a symmetric key is never a key a provider signs with.
        let set: JwkSet = serde_json::from_str(
            r#"{"keys":[{"kty":"RSA","kid":"2011-04-29","use":"sig",
            "n":"0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
            "e":"AQAB"},
            {"kty":"oct","kid":"hmac","k":"bm90LWEta2V5"}]}"#,
        )
        .unwrap();
        let rsa = set.find("2011-04-29").unwrap();
        assert!(key_fits(rsa, Algorithm::RS256));
        assert!(!key_fits(rsa, Algorithm::ES256));
        assert!(DecodingKey::from_jwk(rsa).is_ok());
        let hmac = set.find("hmac").unwrap();
        assert!(!key_fits(hmac, Algorithm::HS256));
        assert!(!key_fits(hmac, Algorithm::RS256));
    }
}
