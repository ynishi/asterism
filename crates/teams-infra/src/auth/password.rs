//! `auth::password` — the v0 instance-local credential adapter
//! (#83 §5).
//!
//! Three decisions carry this module:
//!
//! - **Hashing is argon2id through RustCrypto's `argon2`** (OWASP's
//!   current first choice), default parameters, PHC-string storage —
//!   never a hand-rolled construction. Verification runs *inside* the
//!   SQLite isle closure: the isle's dedicated thread keeps the key
//!   derivation off the async executor without a second thread pool,
//!   at the cost of serialising logins behind one another and behind
//!   other DB work — an accepted trade at this plane's team-scale,
//!   single-server topology (#83 §4).
//! - **Sessions are DB-backed opaque tokens** (#83 §1: a session is a
//!   short-lived infra artifact the ledger never sees). The client
//!   holds 32 CSPRNG bytes hex-encoded; the table holds the SHA-256 of
//!   that, so a leaked database never contains a usable bearer token.
//!   Expiry is enforced twice: [`PasswordAuth::resolve_session`]
//!   rejects **and deletes** an expired row on touch, and
//!   [`PasswordAuth::cleanup_expired`] sweeps in bulk (the server runs
//!   it on every login, so the table cannot accumulate dead rows
//!   faster than logins happen).
//! - **No default credentials exist** ([`reject_default_credential`]):
//!   the bootstrap admin (#83 §5, the §1 InstanceOperator) is created
//!   only from operator-supplied values, and a blank, too-short,
//!   login-equal, or well-known placeholder password is *refused* at
//!   creation time rather than warned about.
//!
//! The port implementation ([`CredentialVerifier`]) keeps the port's
//! one-arm contract: a wrong password and an unknown login are the
//! same `Ok(None)`, and the unknown-login path verifies against a
//! process-local dummy hash so the two answers cost the same work
//! (username-enumeration resistance on the timing side too).

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use async_trait::async_trait;
use rand::TryRngCore;
use rusqlite::{OptionalExtension, params};
use rusqlite_isle::AsyncIsle;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use teams_core::DomainError;
use teams_core::domain::identity::User;
use teams_core::port::auth::CredentialVerifier;
use uuid::Uuid;

use crate::sqlite::map::infra_err;

/// Length of the opaque session token in raw bytes (256 bits).
const SESSION_TOKEN_BYTES: usize = 32;

/// Passwords the instance refuses to ever store — the "no fixed
/// defaults" rule (#83 §5) enforced at the door, compared
/// case-insensitively.
const REFUSED_PASSWORDS: &[&str] = &[
    "admin", "password", "changeme", "default", "letmein", "root", "operator",
];

/// Minimum password length. Eight is the NIST SP 800-63B floor; the
/// point of the check is not strength estimation but making the
/// refusal of trivial credentials structural.
const MIN_PASSWORD_CHARS: usize = 8;

/// One credential-store row, as the server's gate consumes it: who the
/// session resolves to, and whether that account is the env/CLI
/// bootstrap identity (#83 §1 InstanceOperator).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRecord {
    /// The account's stable id — what memberships and stamps refer to.
    pub user_id: Uuid,
    /// The login name credentials are presented under.
    pub login: String,
    /// The display name the ledger would stamp for this account.
    pub display_name: String,
    /// Whether this account is the InstanceOperator. A property of the
    /// *account*, never of any membership row — the operator lives
    /// outside the membership table (#83 §1).
    pub operator: bool,
}

/// The v0 password + session adapter over the teams database.
#[derive(Clone)]
pub struct PasswordAuth {
    isle: AsyncIsle,
}

impl PasswordAuth {
    /// Wraps a writer `AsyncIsle` handle — the same handle (or a clone
    /// of it) the repository uses, so credentials and state share one
    /// database and one writer.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }

    /// Creates an account, refusing default-shaped credentials.
    ///
    /// The display name goes through the domain's [`User::new`] so the
    /// blank-name refusal (the ledger stamps names at write time) is
    /// the domain's, not a second spelling of it. A duplicate login is
    /// a validation refusal, checked inside the same isle closure that
    /// inserts — the isle serialises access, so the check cannot race
    /// a concurrent insert, and the unique index backs it up against
    /// raw SQL.
    ///
    /// `operator = true` is the bootstrap path, and it runs **once**:
    /// the operator is an instance capacity with exactly one holder in
    /// v0 (#83 §1), so a second operator creation is refused with
    /// "already bootstrapped" and writes nothing.
    pub async fn create_account(
        &self,
        login: &str,
        display_name: &str,
        password: &str,
        operator: bool,
        created_at_ms: i64,
    ) -> Result<Uuid, DomainError> {
        let login = login.trim().to_string();
        if login.is_empty() {
            return Err(DomainError::Validation("login is blank".into()));
        }
        let user = User::new(Uuid::now_v7(), display_name)?;
        reject_default_credential(&login, password)?;
        let user_id = user.user_id();
        let display_name = user.display_name().to_string();
        let password = password.to_string();
        self.isle
            .call(move |conn| {
                let taken: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM user_account WHERE login = ?1)",
                    params![login],
                    |row| row.get(0),
                )?;
                if taken {
                    return Ok(Err(DomainError::Validation(format!(
                        "login {login:?} is already registered"
                    ))));
                }
                // The operator is an instance capacity and v0 has
                // exactly one (#83 §1): bootstrap is not re-runnable,
                // and minting further operators is a later deliberate
                // feature, not a second bootstrap. Checked inside the
                // same closure as the insert, so two racing bootstraps
                // cannot both pass.
                if operator {
                    let bootstrapped: bool = conn.query_row(
                        "SELECT EXISTS(SELECT 1 FROM user_account WHERE is_operator = 1)",
                        [],
                        |row| row.get(0),
                    )?;
                    if bootstrapped {
                        return Ok(Err(DomainError::Validation(
                            "this instance is already bootstrapped: an operator account \
                             exists, and v0 has exactly one"
                                .into(),
                        )));
                    }
                }
                // Hashing on the isle thread — see the module doc.
                let hash = match hash_password(&password) {
                    Ok(hash) => hash,
                    Err(refused) => return Ok(Err(refused)),
                };
                conn.execute(
                    "INSERT INTO user_account
                     (user_id, login, display_name, password_hash, is_operator, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![user_id, login, display_name, hash, operator, created_at_ms],
                )?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)??;
        Ok(user_id)
    }

    /// Looks an account up by id — the read the server's gate performs
    /// after a session resolves, and the existence check invite /
    /// founding-owner validation performs before writing a membership.
    pub async fn account(&self, user_id: Uuid) -> Result<Option<AccountRecord>, DomainError> {
        self.isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT user_id, login, display_name, is_operator
                     FROM user_account WHERE user_id = ?1",
                    params![user_id],
                    account_from_row,
                )
                .optional()
            })
            .await
            .map_err(infra_err)
    }

    /// Mints a session for `user_id`: 256 random bits to the caller,
    /// their SHA-256 to the table. The expiry is absolute
    /// (`now + ttl`), computed here so the row and the caller agree on
    /// it.
    pub async fn create_session(
        &self,
        user_id: Uuid,
        now_ms: i64,
        ttl_ms: i64,
    ) -> Result<String, DomainError> {
        if ttl_ms <= 0 {
            return Err(DomainError::Validation(format!(
                "session ttl {ttl_ms}ms is not positive"
            )));
        }
        let mut bytes = [0u8; SESSION_TOKEN_BYTES];
        rand::rngs::OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("OS CSPRNG failure: {e}")))?;
        let token = hex(&bytes);
        let token_hash = sha256_hex(&token);
        let expires_at = now_ms.saturating_add(ttl_ms);
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
                conn.execute(
                    "INSERT INTO auth_session (token_hash, user_id, created_at, expires_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![token_hash, user_id, now_ms, expires_at],
                )?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)??;
        Ok(token)
    }

    /// Resolves an opaque token to its account, or `None` for an
    /// unknown **or expired** token. An expired row is deleted on the
    /// way out — rejection *is* cleanup for the row that was touched;
    /// [`Self::cleanup_expired`] handles the ones nothing touches.
    pub async fn resolve_session(
        &self,
        token: &str,
        now_ms: i64,
    ) -> Result<Option<AccountRecord>, DomainError> {
        let token_hash = sha256_hex(token);
        self.isle
            .call(move |conn| {
                let row = conn
                    .query_row(
                        "SELECT s.expires_at, a.user_id, a.login, a.display_name, a.is_operator
                         FROM auth_session s
                         JOIN user_account a ON a.user_id = s.user_id
                         WHERE s.token_hash = ?1",
                        params![token_hash],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                AccountRecord {
                                    user_id: row.get(1)?,
                                    login: row.get(2)?,
                                    display_name: row.get(3)?,
                                    operator: row.get(4)?,
                                },
                            ))
                        },
                    )
                    .optional()?;
                match row {
                    None => Ok(None),
                    Some((expires_at, _)) if expires_at <= now_ms => {
                        conn.execute(
                            "DELETE FROM auth_session WHERE token_hash = ?1",
                            params![token_hash],
                        )?;
                        Ok(None)
                    }
                    Some((_, account)) => Ok(Some(account)),
                }
            })
            .await
            .map_err(infra_err)
    }

    /// Destroys a session (logout). Idempotent: destroying a token
    /// that never existed or already expired away is not an error —
    /// the caller's goal is "this token resolves to nothing", which
    /// both spellings reach.
    pub async fn destroy_session(&self, token: &str) -> Result<(), DomainError> {
        let token_hash = sha256_hex(token);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM auth_session WHERE token_hash = ?1",
                    params![token_hash],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    /// Sweeps every session whose expiry has passed, returning how
    /// many rows went. The bulk half of the expiry contract — walks
    /// the `expires_at` index.
    pub async fn cleanup_expired(&self, now_ms: i64) -> Result<u64, DomainError> {
        let swept = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM auth_session WHERE expires_at <= ?1",
                    params![now_ms],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(swept as u64)
    }
}

#[async_trait]
impl CredentialVerifier for PasswordAuth {
    async fn verify(&self, login: &str, secret: &str) -> Result<Option<Uuid>, DomainError> {
        let login = login.trim().to_string();
        let secret = secret.to_string();
        self.isle
            .call(move |conn| {
                let row: Option<(Uuid, String)> = conn
                    .query_row(
                        "SELECT user_id, password_hash FROM user_account WHERE login = ?1",
                        params![login],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                Ok(match row {
                    Some((user_id, phc)) => {
                        verify_password(&secret, &phc).map(|ok| ok.then_some(user_id))
                    }
                    None => {
                        // Same work as the found path, so an unknown
                        // login costs what a wrong password costs —
                        // the port's one-arm contract, kept on the
                        // timing side too.
                        let _ = verify_password(&secret, dummy_hash());
                        Ok(None)
                    }
                })
            })
            .await
            .map_err(infra_err)?
    }
}

/// Refuses the credentials the "no fixed defaults" rule (#83 §5)
/// exists to keep out: blank, shorter than the floor, equal to the
/// login, or on the well-known-placeholder list.
pub fn reject_default_credential(login: &str, password: &str) -> Result<(), DomainError> {
    if password.trim().is_empty() {
        return Err(DomainError::Validation("password is blank".into()));
    }
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(DomainError::Validation(format!(
            "password is shorter than {MIN_PASSWORD_CHARS} characters"
        )));
    }
    if password.eq_ignore_ascii_case(login) {
        return Err(DomainError::Validation(
            "password equals the login it protects".into(),
        ));
    }
    if REFUSED_PASSWORDS
        .iter()
        .any(|known| password.eq_ignore_ascii_case(known))
    {
        return Err(DomainError::Validation(
            "password is a well-known default; this instance refuses default credentials".into(),
        ));
    }
    Ok(())
}

/// Argon2id (default parameters) over a fresh 16-byte OS-CSPRNG salt,
/// PHC-string out.
fn hash_password(password: &str) -> Result<String, DomainError> {
    let mut salt_bytes = [0u8; 16];
    rand::rngs::OsRng
        .try_fill_bytes(&mut salt_bytes)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("OS CSPRNG failure: {e}")))?;
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("salt encoding failed: {e}")))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("password hashing failed: {e}")))
}

/// Verifies a password against a stored PHC string. `Ok(false)` is the
/// mismatch arm; `Err` is reserved for a corrupt stored hash.
fn verify_password(password: &str, phc: &str) -> Result<bool, DomainError> {
    let parsed = PasswordHash::new(phc)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("corrupt password_hash column: {e}")))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(DomainError::Infra(anyhow::anyhow!(
            "password verification failed: {e}"
        ))),
    }
}

/// The equal-work target for the unknown-login arm: a process-local
/// PHC string that is never any account's stored hash — it is minted
/// fresh per process from a fixed input and a random salt, and no
/// creation path ever stores it. What the presented secret is does not
/// matter (the arm discards the verify result unconditionally); the
/// hash exists so the arm performs a real argon2 verification rather
/// than returning early.
fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        hash_password("dummy-credential-for-timing-equalisation")
            .expect("hashing a constant with default parameters cannot fail")
    })
}

/// Maps a `user_id, login, display_name, is_operator` row into an
/// [`AccountRecord`] — the one place the column order is spelled.
fn account_from_row(row: &rusqlite::Row<'_>) -> Result<AccountRecord, rusqlite::Error> {
    Ok(AccountRecord {
        user_id: row.get(0)?,
        login: row.get(1)?,
        display_name: row.get(2)?,
        operator: row.get(3)?,
    })
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

fn sha256_hex(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use rusqlite_isle::AsyncIsleDriver;

    const T0: i64 = 1_755_000_000_000;
    const GOOD: &str = "correct horse battery staple";

    async fn auth() -> (PasswordAuth, AsyncIsle, AsyncIsleDriver) {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        (PasswordAuth::new(isle.clone()), isle, driver)
    }

    async fn session_rows(isle: &AsyncIsle) -> i64 {
        isle.call(|conn| conn.query_row("SELECT count(*) FROM auth_session", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_credential_round_trips_and_a_wrong_password_is_none() {
        let (auth, _isle, driver) = auth().await;
        let user_id = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();

        let verifier: &dyn CredentialVerifier = &auth;
        assert_eq!(
            verifier.verify("hoshino", GOOD).await.unwrap(),
            Some(user_id)
        );
        // Wrong password and unknown login are the same arm — the port
        // must not leak which half failed.
        assert_eq!(
            verifier.verify("hoshino", "wrong-password").await.unwrap(),
            None
        );
        assert_eq!(verifier.verify("nobody", GOOD).await.unwrap(), None);

        // What the table stores is a PHC string, never the password.
        let account = auth.account(user_id).await.unwrap().unwrap();
        assert_eq!(account.login, "hoshino");
        assert!(!account.operator);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn default_shaped_credentials_are_refused_at_the_door() {
        let (auth, _isle, driver) = auth().await;
        for password in [
            "", "   ", "short", "admin", "CHANGEME", "hoshino", "Password",
        ] {
            let refused = auth
                .create_account("hoshino", "Hoshino", password, true, T0)
                .await;
            assert!(
                matches!(refused, Err(DomainError::Validation(_))),
                "{password:?} must be refused"
            );
        }
        // Nothing landed.
        assert!(
            matches!(
                (&auth as &dyn CredentialVerifier)
                    .verify("hoshino", "admin")
                    .await,
                Ok(None)
            ),
            "no account may exist after only refused creations"
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_duplicate_login_is_the_stores_refusal() {
        let (auth, _isle, driver) = auth().await;
        auth.create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let duplicate = auth
            .create_account("hoshino", "Someone Else", GOOD, false, T0)
            .await;
        assert!(matches!(duplicate, Err(DomainError::Validation(_))));
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_session_resolves_until_destroyed() {
        let (auth, _isle, driver) = auth().await;
        let user_id = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let token = auth.create_session(user_id, T0, 60_000).await.unwrap();

        let resolved = auth.resolve_session(&token, T0 + 1).await.unwrap().unwrap();
        assert_eq!(resolved.user_id, user_id);
        assert_eq!(resolved.display_name, "Hoshino");

        auth.destroy_session(&token).await.unwrap();
        assert!(
            auth.resolve_session(&token, T0 + 2)
                .await
                .unwrap()
                .is_none()
        );
        // Destroying again is idempotent.
        auth.destroy_session(&token).await.unwrap();

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn an_expired_session_is_rejected_and_its_row_deleted() {
        let (auth, isle, driver) = auth().await;
        let user_id = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let token = auth.create_session(user_id, T0, 1_000).await.unwrap();
        assert_eq!(session_rows(&isle).await, 1);

        // Past the expiry: rejected, and the touched row is gone.
        assert!(
            auth.resolve_session(&token, T0 + 1_000)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(session_rows(&isle).await, 0);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_bulk_sweep_takes_only_what_expired() {
        let (auth, isle, driver) = auth().await;
        let user_id = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let dead = auth.create_session(user_id, T0, 1_000).await.unwrap();
        let live = auth.create_session(user_id, T0, 120_000).await.unwrap();

        assert_eq!(auth.cleanup_expired(T0 + 60_000).await.unwrap(), 1);
        assert_eq!(session_rows(&isle).await, 1);
        assert!(
            auth.resolve_session(&dead, T0 + 60_001)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            auth.resolve_session(&live, T0 + 60_001)
                .await
                .unwrap()
                .is_some()
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_session_for_an_unknown_user_is_refused() {
        let (auth, _isle, driver) = auth().await;
        let refused = auth.create_session(Uuid::now_v7(), T0, 60_000).await;
        assert!(matches!(refused, Err(DomainError::Validation(_))));
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn bootstrap_runs_once_and_a_second_operator_is_refused() {
        let (auth, isle, driver) = auth().await;
        auth.create_account("op", "Operator", GOOD, true, T0)
            .await
            .unwrap();

        // A second bootstrap — different login, so what refuses it is
        // the single-operator rule, not the duplicate-login one.
        let refused = auth
            .create_account("op2", "Second Operator", GOOD, true, T0)
            .await;
        match refused {
            Err(DomainError::Validation(message)) => assert!(
                message.contains("already bootstrapped"),
                "the refusal must say why: {message}"
            ),
            other => panic!("expected the already-bootstrapped refusal, got {other:?}"),
        }

        // …and it created nothing.
        let accounts: i64 = isle
            .call(|conn| conn.query_row("SELECT count(*) FROM user_account", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(accounts, 1);

        // Ordinary accounts are untouched by the rule.
        auth.create_account("alice", "Alice", GOOD, false, T0)
            .await
            .unwrap();

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_operator_flag_rides_the_account_not_a_membership() {
        let (auth, _isle, driver) = auth().await;
        let op_id = auth
            .create_account("op", "Operator", GOOD, true, T0)
            .await
            .unwrap();
        let token = auth.create_session(op_id, T0, 60_000).await.unwrap();
        let resolved = auth.resolve_session(&token, T0 + 1).await.unwrap().unwrap();
        assert!(resolved.operator);
        driver.shutdown().await.unwrap();
    }
}
