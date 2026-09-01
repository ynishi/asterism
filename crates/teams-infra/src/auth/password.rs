//! `auth::password` — the v0 instance-local credential adapter
//! (#83 §5).
//!
//! The decisions that carry this module:
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
//! - **A device token is that construction with a longer life and a
//!   name** (#204). Same bytes, same hash-at-rest rule, same two-sided
//!   expiry — [`PasswordAuth::mint_device_token`] and its four
//!   siblings differ from the session verbs in what the row carries
//!   (a label, a handle, a last-use stamp) rather than in how the
//!   secret is handled. What it is *for* is the invariant #204 fixes:
//!   the disk may hold this and no primary credential, whichever
//!   verifier said yes — so the mint takes a `user_id` and never asks
//!   how the caller proved they were one.
//! - **No default credentials exist** ([`reject_default_credential`]):
//!   the bootstrap admin (#83 §5, the §1
//!   [`InstanceAdmin`](teams_core::domain::identity::InstanceAdmin)) is
//!   created only from operator-supplied values, and a blank, too-short,
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

/// Length of an opaque token in raw bytes (256 bits) — sessions and
/// device tokens alike, because they are one construction issued for
/// two lifetimes (#204).
const OPAQUE_TOKEN_BYTES: usize = 32;

/// How long a device token lives from the mint: **90 days** (#204).
///
/// **Fixed, not slid forward on use**, which is the open question that
/// issue leaves to whoever implements it. Sliding is kinder to a daily
/// driver and costs two things this shape will not pay: the list stops
/// being able to say a date — "expires when it expires, unless you use
/// it" is not a sentence a person revokes on — and a stolen token's
/// life becomes a function of how often the thief presents it, so the
/// one credential the disk is allowed to hold would be the one whose
/// end nothing bounds. A fixed window ends on a day both the owner and
/// the instance can name.
///
/// A constant here rather than a field on the server's context, which
/// is where the session lifetime sits (`TeamsCtx::session_ttl_ms`).
/// Neither is settable from outside the binary today — the server
/// fills that field from its own `DEFAULT_SESSION_TTL_MS` and only the
/// route suites pass anything else — so the difference is not one of
/// configurability but of where the question belongs. A session
/// lifetime is a deployment's trade between re-logins and exposure,
/// which is why it is a field a context can carry and a test can vary.
/// This one is the bound on a credential at rest, and an instance that
/// could widen it to a year would be answering a question #204
/// settled.
pub const DEVICE_TOKEN_TTL_MS: i64 = 90 * 24 * 60 * 60 * 1000;

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
/// session resolves to, and whether that account holds the env/CLI
/// bootstrap capacity (#83 §1,
/// [`InstanceAdmin`](teams_core::domain::identity::InstanceAdmin)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRecord {
    /// The account's stable id — what memberships and stamps refer to.
    pub user_id: Uuid,
    /// The login name credentials are presented under.
    pub login: String,
    /// The display name the ledger would stamp for this account.
    pub display_name: String,
    /// Whether this account is an instance admin. A property of the
    /// *account*, never of any membership row — an admin lives outside
    /// the membership table (#83 §1).
    pub admin: bool,
}

/// What a device-token mint hands back (#204).
///
/// **No derived `Debug`** — see the hand-written one below. This is
/// the only type in this crate that carries a live credential, and it
/// carries one for exactly one hop: the mint is the single moment the
/// token's value exists outside the client that will hold it.
#[derive(Clone)]
pub struct MintedDeviceToken {
    /// The token itself — 256 CSPRNG bits, hex. The table holds only
    /// its SHA-256, so this value cannot be recovered from the
    /// instance and is never answered with again.
    pub token: String,
    /// The row's handle: what a listing names it by and what a revoke
    /// takes. Not derived from the token, so it may be stored and
    /// shown wherever the token may not.
    pub id: Uuid,
    /// When it stops resolving, epoch ms.
    pub expires_at_ms: i64,
}

/// Prints everything about a mint except the value that would let the
/// reader use it.
///
/// The derived one would put a live credential into every panic
/// message, test failure and log line that ever formats this — which
/// is how a secret ends up somewhere nobody meant to put it, copied
/// out by someone who was looking at something else. The token travels
/// from the mint into one HTTP response and into the caller's keychain,
/// and nowhere a human reads.
impl std::fmt::Debug for MintedDeviceToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MintedDeviceToken")
            .field("token", &"<not shown>")
            .field("id", &self.id)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// One device token as its owner sees it — everything the row holds
/// except the digest (#204).
///
/// `Debug` is derived here, and the contrast with
/// [`MintedDeviceToken`] is the point: nothing on this type
/// authenticates anybody, which is the property that lets the listing
/// route exist at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTokenRecord {
    /// The revocation handle.
    pub id: Uuid,
    /// What the client called this device when it asked.
    pub label: String,
    /// When it was minted, epoch ms.
    pub created_at_ms: i64,
    /// When it was last presented, epoch ms — `None` for a token
    /// nobody has used yet.
    pub last_used_at_ms: Option<i64>,
    /// When it stops resolving, epoch ms.
    pub expires_at_ms: i64,
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
    /// `admin = true` provisions an instance admin, and it may be used
    /// more than once. It refused a second admin until #148 revision
    /// 8, on the ground that the capacity had exactly one holder in
    /// v0; the refusal is gone because a single holder is a person who
    /// can be unavailable, and an instance whose only admin is
    /// unreachable has no path back to its own destructive verbs. What
    /// the bootstrap command still is, is how the *first* admin
    /// arrives on an instance with no accounts to authenticate as —
    /// not a limit on how many there may be.
    pub async fn create_account(
        &self,
        login: &str,
        display_name: &str,
        password: &str,
        admin: bool,
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
                // Hashing on the isle thread — see the module doc.
                let hash = match hash_password(&password) {
                    Ok(hash) => hash,
                    Err(refused) => return Ok(Err(refused)),
                };
                conn.execute(
                    "INSERT INTO user_account
                     (user_id, login, display_name, password_hash, is_admin, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![user_id, login, display_name, hash, admin, created_at_ms],
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
                    "SELECT user_id, login, display_name, is_admin
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
        let mut bytes = [0u8; OPAQUE_TOKEN_BYTES];
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
                        "SELECT s.expires_at, a.user_id, a.login, a.display_name, a.is_admin
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
                                    admin: row.get(4)?,
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

    // ------------------------------------------------------------------
    // Device tokens (#204) — the same construction, issued to a device.
    // ------------------------------------------------------------------

    /// Mints a device token for `user_id`: 256 random bits to the
    /// caller, their SHA-256 to the table, and a handle beside it.
    ///
    /// **The mint never learns which verifier said yes**, and that is
    /// the whole of #204's leverage: it takes an account id, so the
    /// password flow reaches it as login → session → mint, and an OIDC
    /// adapter (#163) reaches the same verb the same way without this
    /// code changing. Whether a *live session* is enough to ask is the
    /// route's decision, argued there.
    ///
    /// A blank label is refused. The label is what a person reads in
    /// the list before revoking, and a set of unnamed rows is a list
    /// that cannot be acted on — the same reasoning that makes the
    /// domain refuse a blank display name, applied to the one field
    /// this row has for a human.
    ///
    /// The lifetime is [`DEVICE_TOKEN_TTL_MS`] and is not a parameter;
    /// the expiry is absolute, computed here so the row and the caller
    /// agree on it.
    pub async fn mint_device_token(
        &self,
        user_id: Uuid,
        label: &str,
        now_ms: i64,
    ) -> Result<MintedDeviceToken, DomainError> {
        let label = label.trim().to_string();
        if label.is_empty() {
            return Err(DomainError::Validation(
                "device token label is blank; a device token is named so its owner can \
                 tell one from another when revoking"
                    .into(),
            ));
        }
        let mut bytes = [0u8; OPAQUE_TOKEN_BYTES];
        rand::rngs::OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("OS CSPRNG failure: {e}")))?;
        let token = hex(&bytes);
        let token_hash = sha256_hex(&token);
        let id = Uuid::now_v7();
        let expires_at = now_ms.saturating_add(DEVICE_TOKEN_TTL_MS);
        let stored_label = label.clone();
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
                    "INSERT INTO device_token
                     (token_hash, id, user_id, label, created_at, last_used_at, expires_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
                    params![token_hash, id, user_id, stored_label, now_ms, expires_at],
                )?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)??;
        Ok(MintedDeviceToken {
            token,
            id,
            expires_at_ms: expires_at,
        })
    }

    /// Resolves a device token to its account, or `None` for an
    /// unknown **or expired** one — the same one-armed answer
    /// [`Self::resolve_session`] gives, for the same reason, and an
    /// expired row is deleted on the way out just as a session's is.
    ///
    /// A successful resolve stamps `last_used_at`. That write is the
    /// reason this is not a read: the column is what a person looks at
    /// to decide whether a device is still theirs, and a "last used"
    /// that only moved when somebody remembered to update it would be
    /// worse than absent. It is stamped inside the same closure as the
    /// lookup, so a resolve either answers and records or does
    /// neither.
    pub async fn resolve_device_token(
        &self,
        token: &str,
        now_ms: i64,
    ) -> Result<Option<AccountRecord>, DomainError> {
        let token_hash = sha256_hex(token);
        self.isle
            .call(move |conn| {
                let row = conn
                    .query_row(
                        "SELECT d.expires_at, a.user_id, a.login, a.display_name, a.is_admin
                         FROM device_token d
                         JOIN user_account a ON a.user_id = d.user_id
                         WHERE d.token_hash = ?1",
                        params![token_hash],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                AccountRecord {
                                    user_id: row.get(1)?,
                                    login: row.get(2)?,
                                    display_name: row.get(3)?,
                                    admin: row.get(4)?,
                                },
                            ))
                        },
                    )
                    .optional()?;
                match row {
                    None => Ok(None),
                    Some((expires_at, _)) if expires_at <= now_ms => {
                        conn.execute(
                            "DELETE FROM device_token WHERE token_hash = ?1",
                            params![token_hash],
                        )?;
                        Ok(None)
                    }
                    Some((_, account)) => {
                        conn.execute(
                            "UPDATE device_token SET last_used_at = ?2 WHERE token_hash = ?1",
                            params![token_hash, now_ms],
                        )?;
                        Ok(Some(account))
                    }
                }
            })
            .await
            .map_err(infra_err)
    }

    /// One account's device tokens, oldest mint first, **without the
    /// digest that authenticates any of them**.
    ///
    /// [`DeviceTokenRecord`] is what makes that structural rather than
    /// a discipline: there is no field for a hash to be put in.
    ///
    /// Rows the table holds, which is not quite the same as rows that
    /// still resolve — an expired token sits here until something
    /// touches or sweeps it. That is why every row carries its
    /// `expires_at_ms`, and why the routes run
    /// [`Self::cleanup_expired_device_tokens`] before they read: the
    /// sweep is what keeps the two answers together, and the column is
    /// what lets a reader tell when it has not.
    pub async fn list_device_tokens(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<DeviceTokenRecord>, DomainError> {
        self.isle
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT id, label, created_at, last_used_at, expires_at
                     FROM device_token WHERE user_id = ?1
                     ORDER BY created_at, id",
                )?;
                let rows = statement
                    .query_map(params![user_id], |row| {
                        Ok(DeviceTokenRecord {
                            id: row.get(0)?,
                            label: row.get(1)?,
                            created_at_ms: row.get(2)?,
                            last_used_at_ms: row.get(3)?,
                            expires_at_ms: row.get(4)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)
    }

    /// Revokes one of `user_id`'s device tokens by its handle.
    ///
    /// **Owner-scoped in the statement**, not in a check before it:
    /// the `user_id` predicate is what makes another account's handle
    /// match nothing, so there is no arrangement of arguments that
    /// deletes a row the caller does not own.
    ///
    /// Idempotent, like [`Self::destroy_session`]: a handle that never
    /// existed, one already revoked, and one belonging to somebody
    /// else are the same `Ok(())`. Distinguishing them would answer
    /// "does this id exist" for ids the caller has no business knowing
    /// about, and the caller's goal — this token resolves to nothing
    /// *for me* — is true in all three cases.
    pub async fn revoke_device_token(&self, user_id: Uuid, id: Uuid) -> Result<(), DomainError> {
        self.isle
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM device_token WHERE id = ?1 AND user_id = ?2",
                    params![id, user_id],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    /// Sweeps every device token whose expiry has passed, returning
    /// how many rows went — [`Self::cleanup_expired`]'s sibling over
    /// the other table.
    ///
    /// A sibling rather than a second statement inside that one: the
    /// two sweeps run at different moments, because the tables fill at
    /// different moments. Sessions accumulate as fast as people log
    /// in, so the login path is where that table is swept; device
    /// tokens accumulate as fast as people mint, which is rare, so
    /// theirs is swept where it is read and where one is presented.
    /// Folding them together would make each call site pay for a sweep
    /// it did not need and hide which surface keeps which table
    /// bounded.
    pub async fn cleanup_expired_device_tokens(&self, now_ms: i64) -> Result<u64, DomainError> {
        let swept = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM device_token WHERE expires_at <= ?1",
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

/// Maps a `user_id, login, display_name, is_admin` row into an
/// [`AccountRecord`] — the one place the column order is spelled.
fn account_from_row(row: &rusqlite::Row<'_>) -> Result<AccountRecord, rusqlite::Error> {
    Ok(AccountRecord {
        user_id: row.get(0)?,
        login: row.get(1)?,
        display_name: row.get(2)?,
        admin: row.get(3)?,
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

    async fn device_rows(isle: &AsyncIsle) -> i64 {
        isle.call(|conn| conn.query_row("SELECT count(*) FROM device_token", [], |r| r.get(0)))
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
        assert!(!account.admin);

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
    async fn an_instance_may_hold_more_than_one_admin() {
        let (auth, isle, driver) = auth().await;
        let first = auth
            .create_account("admin", "Admin", GOOD, true, T0)
            .await
            .unwrap();

        // The refusal this used to meet is gone (#148 revision 8): a
        // second admin lands, and both hold the capacity.
        let second = auth
            .create_account("admin2", "Second Admin", GOOD, true, T0)
            .await
            .unwrap();
        assert_ne!(first, second);
        for user_id in [first, second] {
            assert!(auth.account(user_id).await.unwrap().unwrap().admin);
        }

        let accounts: i64 = isle
            .call(|conn| {
                conn.query_row(
                    "SELECT count(*) FROM user_account WHERE is_admin = 1",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(accounts, 2);

        // The one refusal that stays is the duplicate login, which is
        // about the login and not about the capacity.
        assert!(
            auth.create_account("admin", "Impostor", GOOD, true, T0)
                .await
                .is_err()
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_device_token_resolves_until_revoked_and_records_its_last_use() {
        let (auth, isle, driver) = auth().await;
        let user_id = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();

        let minted = auth
            .mint_device_token(user_id, "Hoshino's MacBook", T0)
            .await
            .unwrap();
        assert_eq!(minted.expires_at_ms, T0 + DEVICE_TOKEN_TTL_MS);

        // Nothing has presented it yet, and the listing says so
        // instead of borrowing the mint instant.
        let listed = auth.list_device_tokens(user_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, minted.id);
        assert_eq!(listed[0].label, "Hoshino's MacBook");
        assert_eq!(listed[0].last_used_at_ms, None);

        let resolved = auth
            .resolve_device_token(&minted.token, T0 + 5_000)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.user_id, user_id);
        assert_eq!(resolved.display_name, "Hoshino");
        assert_eq!(
            auth.list_device_tokens(user_id).await.unwrap()[0].last_used_at_ms,
            Some(T0 + 5_000),
            "a resolve stamps the use it was"
        );

        auth.revoke_device_token(user_id, minted.id).await.unwrap();
        assert!(
            auth.resolve_device_token(&minted.token, T0 + 6_000)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(device_rows(&isle).await, 0);
        // Revoking again is idempotent, exactly as destroying a
        // session is.
        auth.revoke_device_token(user_id, minted.id).await.unwrap();

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_device_token_is_only_its_owners_to_revoke() {
        let (auth, isle, driver) = auth().await;
        let mine = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let theirs = auth
            .create_account("someone", "Someone Else", GOOD, false, T0)
            .await
            .unwrap();
        let minted = auth.mint_device_token(mine, "MacBook", T0).await.unwrap();

        // The other account's revoke is not an error and is also not a
        // deletion: it matched nothing, which is the same answer a
        // handle that never existed gets.
        auth.revoke_device_token(theirs, minted.id).await.unwrap();
        assert_eq!(device_rows(&isle).await, 1);
        assert!(
            auth.resolve_device_token(&minted.token, T0 + 1)
                .await
                .unwrap()
                .is_some()
        );
        // And the listing is scoped the same way.
        assert!(auth.list_device_tokens(theirs).await.unwrap().is_empty());

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn an_expired_device_token_is_rejected_and_its_row_deleted() {
        let (auth, isle, driver) = auth().await;
        let user_id = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let minted = auth
            .mint_device_token(user_id, "MacBook", T0)
            .await
            .unwrap();
        assert_eq!(device_rows(&isle).await, 1);

        assert!(
            auth.resolve_device_token(&minted.token, T0 + DEVICE_TOKEN_TTL_MS)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            device_rows(&isle).await,
            0,
            "rejection is cleanup for the row that was touched"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_device_sweep_takes_only_what_expired() {
        let (auth, isle, driver) = auth().await;
        let user_id = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        // One minted long enough ago to have expired, one minted now.
        let dead = auth
            .mint_device_token(user_id, "old laptop", T0 - DEVICE_TOKEN_TTL_MS)
            .await
            .unwrap();
        let live = auth
            .mint_device_token(user_id, "MacBook", T0)
            .await
            .unwrap();

        assert_eq!(auth.cleanup_expired_device_tokens(T0).await.unwrap(), 1);
        assert_eq!(device_rows(&isle).await, 1);
        assert!(
            auth.resolve_device_token(&dead.token, T0 + 1)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            auth.resolve_device_token(&live.token, T0 + 1)
                .await
                .unwrap()
                .is_some()
        );
        // The two sweeps answer for their own tables and nothing else.
        let session = auth.create_session(user_id, T0, 60_000).await.unwrap();
        assert_eq!(auth.cleanup_expired_device_tokens(T0 + 1).await.unwrap(), 0);
        assert!(
            auth.resolve_session(&session, T0 + 1)
                .await
                .unwrap()
                .is_some()
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_device_token_needs_a_name_and_an_account() {
        let (auth, isle, driver) = auth().await;
        let user_id = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();

        for label in ["", "   ", "\n\t"] {
            let refused = auth.mint_device_token(user_id, label, T0).await;
            assert!(
                matches!(refused, Err(DomainError::Validation(_))),
                "{label:?} must be refused"
            );
        }
        let unknown = auth.mint_device_token(Uuid::now_v7(), "MacBook", T0).await;
        assert!(matches!(unknown, Err(DomainError::Validation(_))));
        assert_eq!(device_rows(&isle).await, 0, "nothing landed");

        // The label is stored trimmed, so the list shows what a person
        // typed rather than what their terminal added.
        auth.mint_device_token(user_id, "  MacBook  ", T0)
            .await
            .unwrap();
        assert_eq!(
            auth.list_device_tokens(user_id).await.unwrap()[0].label,
            "MacBook"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_table_holds_a_hash_and_never_the_token() {
        let (auth, isle, driver) = auth().await;
        let user_id = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let minted = auth
            .mint_device_token(user_id, "MacBook", T0)
            .await
            .unwrap();

        let token = minted.token.clone();
        let (stored, matching): (String, i64) = isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT token_hash, (SELECT count(*) FROM device_token WHERE token_hash = ?1)
                     FROM device_token",
                    params![token],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_ne!(stored, minted.token, "the row must not be the credential");
        assert_eq!(stored, sha256_hex(&minted.token));
        assert_eq!(matching, 0, "the token's own value keys nothing");

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_admin_flag_rides_the_account_not_a_membership() {
        let (auth, _isle, driver) = auth().await;
        let admin_id = auth
            .create_account("admin", "Admin", GOOD, true, T0)
            .await
            .unwrap();
        let token = auth.create_session(admin_id, T0, 60_000).await.unwrap();
        let resolved = auth.resolve_session(&token, T0 + 1).await.unwrap().unwrap();
        assert!(resolved.admin);
        driver.shutdown().await.unwrap();
    }
}
