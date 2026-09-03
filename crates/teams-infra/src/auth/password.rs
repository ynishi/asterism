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
//!   expiry — [`PasswordAuth::mint_device_token`] and its siblings
//!   differ from the session verbs in what the row carries
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
//! one-arm contract: a wrong password, an unknown login, an account
//! that holds no password ([`LOCKED_PASSWORD`], #163) and an account
//! an admin has locked (`locked_at`, #213 — a different thing from
//! holding no password, which V13's doc separates) are the same
//! `Ok(None)`, and the paths that check no hash — none to check, or
//! one they decline — verify
//! against a process-local dummy hash so every answer costs the same
//! work (username-enumeration resistance on the timing side too).

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

/// How a device token stops resolving, and why (#163, #213).
///
/// Each end told apart on the wire so that an app
/// can say to a person which of them happened — a token the owner
/// revoked from another machine and one that sat unused for a month
/// are different news, and an app that shows one password form for
/// both is an app whose person cannot act on either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceTokenResolution {
    /// The token is live; this is its account, and its last use is
    /// now.
    Resolved(AccountRecord),
    /// The token's fixed window has closed. The row is gone.
    Expired,
    /// The token went unpresented for longer than the instance
    /// allows. The row is gone.
    Idle,
    /// An admin took the token back (#213): the instance signed this
    /// device out, and the person holding it did not do it. The row
    /// was kept to say so — through any idle bound, until presented or
    /// until the window the mint wrote closes, on [`ended_sql`]'s
    /// terms — and is gone now that it has.
    RevokedByInstance,
    /// The account is locked (#213): the token is what it was, and
    /// resolves nothing until the lock is lifted. The row stays.
    Locked,
    /// No row holds this token's digest — never minted here, revoked
    /// by its owner, or a tombstone that has already said what it was
    /// kept to say or was swept before it could. They are one answer,
    /// because a handle nobody holds and a handle its owner took back
    /// are both nothing to present, the owner needs no news of their
    /// own act, and the news a tombstone carries is given once.
    Unknown,
}

#[cfg(test)]
impl DeviceTokenResolution {
    /// The account, for an assertion that does not need the reason.
    fn account(self) -> Option<AccountRecord> {
        match self {
            Self::Resolved(account) => Some(account),
            Self::Expired | Self::Idle | Self::RevokedByInstance | Self::Locked | Self::Unknown => {
                None
            }
        }
    }
}

/// What [`PasswordAuth::lock_account`] did (#213).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockOutcome {
    /// This call locked the account.
    Locked,
    /// Nothing changed: the account was already locked, or there is
    /// no such account — a caller that has to tell those apart asks
    /// [`PasswordAuth::account`] first.
    Unchanged,
    /// Refused: the account is the last unlocked admin, and locking it
    /// would leave the instance with none — counted as unlocked
    /// admins, which is what the statement can count; an admin
    /// provisioned without a password would be counted too.
    LastAdmin,
}

/// One row of the instance's record of acts on accounts (#213): who
/// did what to whom, and when. `subject_user_id` is `None` for an act
/// on every account at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountEvent {
    /// The row's place in the record; ascending is the order the acts
    /// happened in.
    pub seq: i64,
    /// When, epoch ms.
    pub occurred_at_ms: i64,
    /// The admin who acted.
    pub actor_user_id: Uuid,
    /// The name they had at the time, as a ledger stamp keeps it.
    pub actor_name: String,
    /// The account acted on, or `None` for the whole instance.
    pub subject_user_id: Option<Uuid>,
    /// What was done: one of the closed set the wire's
    /// `AccountEventDto::kind` lists, written by the routes in
    /// `teams-server` that are its definition.
    pub kind: String,
}

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

/// What `password_hash` holds for an account that has no password
/// (#163): the Unix convention for a locked account, a value no PHC
/// parser accepts and no `hash_password` produces.
///
/// An account provisioned for a provider authenticates through the
/// provider and nothing else; why the column stays `NOT NULL` and
/// holds this rather than nothing is V11's doc
/// (`sqlite::migrations`). [`CredentialVerifier::verify`] recognises
/// the sentinel and answers the one-armed `None` after the same argon2
/// work a wrong password costs, so a locked account is not
/// distinguishable from an unknown login on the password arm, by
/// timing or by answer.
const LOCKED_PASSWORD: &str = "!";

/// The one predicate for a `device_token` row that may go: the window
/// the mint wrote has closed, or — while the row is still a token
/// somebody might present, not a tombstone — it has gone unused past
/// the idle bound, when the instance has one. Shared by the sweep, the
/// listing and the instance's revokes (#213), so that "live" means one
/// thing wherever it is asked: what the sweep would remove, the
/// listing does not show and the revoke does not stamp.
///
/// A tombstone (`revoked_at` set) is exempt from the idle arm on
/// purpose. Its job is to outlive the credential's clocks until it has
/// been presented once and said who ended the token; idleness is a
/// fact about a token nobody is using, and a tombstone is not a token.
/// It goes when presented, or with the window — which is what bounds
/// the table. `now` and `idle` are the positional parameter indexes
/// the caller binds the instant and the optional bound at.
/// [`PasswordAuth::resolve_device_token`] spells these ends out in
/// Rust, plus the tombstone's own, because it has to say which one a
/// token met.
///
/// The price of one predicate: the sweep no longer walks
/// `idx_device_token_expires` alone — the idle arm reads
/// `last_used_at` and `revoked_at`, so it scans. The table is bounded
/// by the same sweep, and small; one definition of "live" is worth
/// more than the index walk was.
fn ended_sql(now: usize, idle: usize) -> String {
    format!(
        "(expires_at <= ?{now} \
          OR (revoked_at IS NULL AND ?{idle} IS NOT NULL \
              AND coalesce(last_used_at, created_at) + ?{idle} <= ?{now}))"
    )
}

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

    /// Creates an account that holds no password (#163): one whose
    /// only way in is the identity provider the instance is configured
    /// with, bound to it through
    /// [`OidcIdentities`](crate::auth::oidc::OidcIdentities).
    ///
    /// The same refusals as [`Self::create_account`] for the login and
    /// the display name; nothing to refuse about a password, because
    /// there is none. What the row holds instead is [`LOCKED_PASSWORD`],
    /// and what that buys is stated on the constant.
    pub async fn create_account_locked(
        &self,
        login: &str,
        display_name: &str,
        admin: bool,
        created_at_ms: i64,
    ) -> Result<Uuid, DomainError> {
        let login = login.trim().to_string();
        if login.is_empty() {
            return Err(DomainError::Validation("login is blank".into()));
        }
        let user = User::new(Uuid::now_v7(), display_name)?;
        let user_id = user.user_id();
        let display_name = user.display_name().to_string();
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
                conn.execute(
                    "INSERT INTO user_account
                     (user_id, login, display_name, password_hash, is_admin, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        user_id,
                        login,
                        display_name,
                        LOCKED_PASSWORD,
                        admin,
                        created_at_ms
                    ],
                )?;
                Ok(Ok(()))
            })
            .await
            .map_err(infra_err)??;
        Ok(user_id)
    }

    /// Looks an account up by login — what a provisioning verb uses to
    /// find the account a binding is being written for (#163).
    pub async fn account_by_login(
        &self,
        login: &str,
    ) -> Result<Option<AccountRecord>, DomainError> {
        let login = login.trim().to_string();
        self.isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT user_id, login, display_name, is_admin
                     FROM user_account WHERE login = ?1",
                    params![login],
                    account_from_row,
                )
                .optional()
            })
            .await
            .map_err(infra_err)
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
    /// unknown **or expired** token, or one whose account is locked
    /// (#213). An expired row is deleted on the way out — rejection
    /// *is* cleanup for the row that was touched;
    /// [`Self::cleanup_expired`] handles the ones nothing touches. A
    /// locked account's row is kept: the lock is the account's state
    /// and not the session's, and lifting it gives the session back
    /// for whatever life it has left.
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
                        "SELECT s.expires_at, a.locked_at,
                                a.user_id, a.login, a.display_name, a.is_admin
                         FROM auth_session s
                         JOIN user_account a ON a.user_id = s.user_id
                         WHERE s.token_hash = ?1",
                        params![token_hash],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, Option<i64>>(1)?,
                                AccountRecord {
                                    user_id: row.get(2)?,
                                    login: row.get(3)?,
                                    display_name: row.get(4)?,
                                    admin: row.get(5)?,
                                },
                            ))
                        },
                    )
                    .optional()?;
                match row {
                    None => Ok(None),
                    Some((expires_at, _, _)) if expires_at <= now_ms => {
                        conn.execute(
                            "DELETE FROM auth_session WHERE token_hash = ?1",
                            params![token_hash],
                        )?;
                        Ok(None)
                    }
                    Some((_, Some(_), _)) => Ok(None),
                    Some((_, None, account)) => Ok(Some(account)),
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
    /// **The mint never learns which way in was taken**, and that is
    /// the whole of #204's leverage: it takes an account id, so the
    /// password flow reaches it as login → session → mint, and a
    /// sign-in through the provider (#163) reaches the same verb the
    /// same way without this code changing. Whether a *live session*
    /// is enough to ask is the route's decision, argued there.
    ///
    /// A blank label is refused. The label is what a person reads in
    /// the list before revoking, and a set of unnamed rows is a list
    /// that cannot be acted on — the same reasoning that makes the
    /// domain refuse a blank display name, applied to the one field
    /// this row has for a human.
    ///
    /// The lifetime is the caller's — the server's context carries the
    /// instance's policy, and says why it is policy — and the expiry
    /// is absolute, computed here so the row and the caller agree on
    /// it. **Fixed at the mint, not slid forward on use**: sliding
    /// would leave the list unable to say a date, and make a stolen
    /// token's life a function of how often the thief presents it. An
    /// idle timeout is the opposite thing — it ends a token early and
    /// never extends one — and is applied wherever [`ended_sql`] is.
    pub async fn mint_device_token(
        &self,
        user_id: Uuid,
        label: &str,
        now_ms: i64,
        ttl_ms: i64,
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
        let expires_at = now_ms.saturating_add(ttl_ms);
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

    /// Resolves a device token to its account, or says which of the
    /// ways it stopped resolving ([`DeviceTokenResolution`]). The
    /// token's own ends come first and delete the row on the way out
    /// — an expired or idle one just as an expired session's is, and a
    /// tombstone an admin left once it has said what it was kept to
    /// say; the account's lock is read only of a token that would
    /// otherwise resolve, and that row is kept, because the lock is
    /// the account's and lifting it gives the token back to whatever
    /// life it has left.
    ///
    /// `idle_ms` is the instance's idle timeout, if it has one: a token
    /// last presented (or, never presented, minted) longer ago than
    /// that is [`DeviceTokenResolution::Idle`]. It ends a token early
    /// and never extends one — the fixed window the mint wrote is the
    /// ceiling either way.
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
        idle_ms: Option<i64>,
    ) -> Result<DeviceTokenResolution, DomainError> {
        let token_hash = sha256_hex(token);
        self.isle
            .call(move |conn| {
                let row = conn
                    .query_row(
                        "SELECT d.expires_at, d.created_at, d.last_used_at, d.revoked_at,
                                a.locked_at,
                                a.user_id, a.login, a.display_name, a.is_admin
                         FROM device_token d
                         JOIN user_account a ON a.user_id = d.user_id
                         WHERE d.token_hash = ?1",
                        params![token_hash],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, Option<i64>>(2)?,
                                row.get::<_, Option<i64>>(3)?,
                                row.get::<_, Option<i64>>(4)?,
                                AccountRecord {
                                    user_id: row.get(5)?,
                                    login: row.get(6)?,
                                    display_name: row.get(7)?,
                                    admin: row.get(8)?,
                                },
                            ))
                        },
                    )
                    .optional()?;
                let Some((expires_at, created_at, last_used_at, revoked_at, locked_at, account)) =
                    row
                else {
                    return Ok(DeviceTokenResolution::Unknown);
                };
                // The ends first, the lock after — do not hoist the
                // lock above this chain: "sign everybody out, then
                // lock" is the order #213 names, and a lock read first
                // would answer `Locked` for a token that had already
                // ended.
                let ended = if revoked_at.is_some() {
                    Some(DeviceTokenResolution::RevokedByInstance)
                } else if expires_at <= now_ms {
                    Some(DeviceTokenResolution::Expired)
                } else if idle_ms.is_some_and(|idle| {
                    last_used_at.unwrap_or(created_at).saturating_add(idle) <= now_ms
                }) {
                    Some(DeviceTokenResolution::Idle)
                } else {
                    None
                };
                if ended.is_none() && locked_at.is_some() {
                    return Ok(DeviceTokenResolution::Locked);
                }
                if let Some(ended) = ended {
                    conn.execute(
                        "DELETE FROM device_token WHERE token_hash = ?1",
                        params![token_hash],
                    )?;
                    return Ok(ended);
                }
                conn.execute(
                    "UPDATE device_token SET last_used_at = ?2 WHERE token_hash = ?1",
                    params![token_hash, now_ms],
                )?;
                Ok(DeviceTokenResolution::Resolved(account))
            })
            .await
            .map_err(infra_err)
    }

    /// This instance's stable id (#163), minted once by the migration
    /// that made the table and never changed — what a client is to
    /// key a stored connection by, because a server's URL is a name
    /// that moves and this is not.
    pub async fn instance_id(&self) -> Result<String, DomainError> {
        self.isle
            .call(|conn| {
                conn.query_row(
                    "SELECT value FROM instance_identity WHERE key = 'instance_id'",
                    [],
                    |row| row.get(0),
                )
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
    /// Rows that are live on [`ended_sql`]'s terms and that nobody has
    /// taken back — a row that has ended but not yet been swept is
    /// not shown, and neither is a tombstone an admin left (#213), so
    /// what this shows is every token still inside its own life —
    /// which is what the instance's revoke would take back, and, for
    /// an account that is not locked, what still resolves. Every row
    /// carries its `expires_at_ms`
    /// so a reader can see the end coming; the routes run
    /// [`Self::cleanup_expired_device_tokens`] before they read to
    /// keep the table bounded, not to keep this honest — the predicate
    /// does that.
    pub async fn list_device_tokens(
        &self,
        user_id: Uuid,
        now_ms: i64,
        idle_ms: Option<i64>,
    ) -> Result<Vec<DeviceTokenRecord>, DomainError> {
        self.isle
            .call(move |conn| {
                let mut statement = conn.prepare(&format!(
                    "SELECT id, label, created_at, last_used_at, expires_at
                     FROM device_token
                     WHERE user_id = ?1 AND revoked_at IS NULL AND NOT {}
                     ORDER BY created_at, id",
                    ended_sql(2, 3)
                ))?;
                let rows = statement
                    .query_map(params![user_id, now_ms, idle_ms], |row| {
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
    /// existed, one already revoked, one belonging to somebody else,
    /// and one an admin has already taken back (#213) are the same
    /// `Ok(())`. Distinguishing them would answer "does this id exist"
    /// for ids the caller has no business knowing about, and the
    /// caller's goal — this token resolves to nothing *for me* — is
    /// true in every case.
    ///
    /// The sessions this token minted are not revoked with it: they
    /// die at their own TTL (#204). A session outliving the credential
    /// that produced it is the honest reading of a device token — it
    /// authorises the *making* of sessions, and taking it away stops
    /// the next one. The instance's revokes (#213) leave sessions the
    /// same way, and the lock is the verb that stops one resolving.
    pub async fn revoke_device_token(&self, user_id: Uuid, id: Uuid) -> Result<(), DomainError> {
        self.isle
            .call(move |conn| {
                // A tombstone an admin left (#213) is not the owner's
                // to delete: it holds news for the device, and a
                // handle from a listing taken before the admin acted
                // must not silence it.
                conn.execute(
                    "DELETE FROM device_token
                     WHERE id = ?1 AND user_id = ?2 AND revoked_at IS NULL",
                    params![id, user_id],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    /// Takes back every live device token of `user_id` as the instance
    /// (#213), leaving each row as a tombstone so the next
    /// presentation is answered [`DeviceTokenResolution::RevokedByInstance`]
    /// rather than the `Unknown` an owner's own revoke earns. Returns
    /// how many were live.
    ///
    /// Live, in the statement, on [`ended_sql`]'s terms: a row that
    /// has already ended is left for the sweep rather than stamped —
    /// the tombstone's whole content is that somebody else ended the
    /// token, and a token that had ended on its own is not that.
    /// Sessions are left as [`Self::revoke_device_token`] leaves them,
    /// for the reason given there.
    pub async fn revoke_device_tokens_of(
        &self,
        user_id: Uuid,
        now_ms: i64,
        idle_ms: Option<i64>,
    ) -> Result<u64, DomainError> {
        let revoked = self
            .isle
            .call(move |conn| {
                conn.execute(
                    &format!(
                        "UPDATE device_token SET revoked_at = ?2
                         WHERE user_id = ?1 AND revoked_at IS NULL AND NOT {}",
                        ended_sql(2, 3)
                    ),
                    params![user_id, now_ms, idle_ms],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(revoked as u64)
    }

    /// [`Self::revoke_device_tokens_of`] over every account at once,
    /// on the same terms. Returns how many were live.
    pub async fn revoke_every_device_token(
        &self,
        now_ms: i64,
        idle_ms: Option<i64>,
    ) -> Result<u64, DomainError> {
        let revoked = self
            .isle
            .call(move |conn| {
                conn.execute(
                    &format!(
                        "UPDATE device_token SET revoked_at = ?1
                         WHERE revoked_at IS NULL AND NOT {}",
                        ended_sql(1, 2)
                    ),
                    params![now_ms, idle_ms],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(revoked as u64)
    }

    /// Locks `user_id` (#213): from now every credential of the
    /// account resolves nothing — the password arm, the device arm,
    /// a provider sign-in, and the sessions it already holds — while
    /// its rows stay, so its ledger stamps keep resolving to a name.
    /// This is the definition of the lock: the lock is the account's
    /// state, so every arm that resolves a credential reads it before
    /// answering.
    ///
    /// Answers what this call did ([`LockOutcome`]). Decided in one
    /// statement on one connection, so a caller recording the act
    /// records it once however many times it is asked — and so that
    /// the last unlocked admin is locked by nobody: two admins each
    /// locking the other, both sessions resolved before either write
    /// lands, would otherwise leave an instance with no admin who can
    /// authenticate, and `bootstrap-admin` as the only way back. The
    /// refusal is the statement's own subquery, which the isle's one
    /// thread serialises against every other write.
    pub async fn lock_account(
        &self,
        user_id: Uuid,
        now_ms: i64,
    ) -> Result<LockOutcome, DomainError> {
        self.isle
            .call(move |conn| {
                let changed = conn.execute(
                    "UPDATE user_account SET locked_at = ?2
                     WHERE user_id = ?1 AND locked_at IS NULL
                       AND (is_admin = 0
                            OR (SELECT count(*) FROM user_account
                                WHERE is_admin = 1 AND locked_at IS NULL) > 1)",
                    params![user_id, now_ms],
                )?;
                if changed == 1 {
                    return Ok(LockOutcome::Locked);
                }
                let unlocked_admin: Option<bool> = conn
                    .query_row(
                        "SELECT is_admin = 1 AND locked_at IS NULL
                         FROM user_account WHERE user_id = ?1",
                        params![user_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(match unlocked_admin {
                    Some(true) => LockOutcome::LastAdmin,
                    _ => LockOutcome::Unchanged,
                })
            })
            .await
            .map_err(infra_err)
    }

    /// Lifts a lock (#213). Answers whether this call is what lifted
    /// it, on [`Self::lock_account`]'s terms.
    pub async fn unlock_account(&self, user_id: Uuid) -> Result<bool, DomainError> {
        let changed = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE user_account SET locked_at = NULL
                     WHERE user_id = ?1 AND locked_at IS NOT NULL",
                    params![user_id],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(changed == 1)
    }

    /// When `user_id` was locked, or `None` for an account that is not
    /// (or does not exist). For assertions; the server reads it
    /// through [`Self::account_page`], in one call with the rows.
    #[cfg(test)]
    pub async fn locked_at(&self, user_id: Uuid) -> Result<Option<i64>, DomainError> {
        self.isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT locked_at FROM user_account WHERE user_id = ?1",
                    params![user_id],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()
                .map(Option::flatten)
            })
            .await
            .map_err(infra_err)
    }

    /// Appends one act to the instance's record of acts on accounts
    /// (#213), and hands back its place in it.
    pub async fn record_account_event(
        &self,
        actor: &AccountRecord,
        subject_user_id: Option<Uuid>,
        kind: &str,
        now_ms: i64,
    ) -> Result<i64, DomainError> {
        let actor_user_id = actor.user_id;
        let actor_name = actor.display_name.clone();
        let kind = kind.to_string();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO account_event
                     (occurred_at, actor_user_id, actor_name, subject_user_id, kind)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![now_ms, actor_user_id, actor_name, subject_user_id, kind],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .map_err(infra_err)
    }

    /// The instance's whole record of acts on accounts, oldest first
    /// — a walk of the key. One account's page is
    /// [`Self::account_page`].
    pub async fn account_events(&self) -> Result<Vec<AccountEvent>, DomainError> {
        self.isle
            .call(move |conn| {
                conn.prepare(
                    "SELECT seq, occurred_at, actor_user_id, actor_name,
                            subject_user_id, kind
                     FROM account_event ORDER BY seq",
                )?
                .query_map([], account_event_from_row)?
                .collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(infra_err)
    }

    /// One account's page of the instance record (#213): when it was
    /// locked, if it is, together with its rows and the rows of the
    /// acts on every account, which touched it too — oldest first.
    ///
    /// One call for the lock and the rows, so a lock landing between
    /// two reads cannot answer a page whose lock and whose `locked`
    /// row disagree. The rows are two probes of the subject index
    /// (the account's, and the subject-less), each already in `seq`
    /// order, merged; a single `subject_user_id = ?1 OR subject_user_id
    /// IS NULL` predicate probes the same index twice and then sorts
    /// the union in a temporary B-tree, which the `UNION ALL` does not
    /// need.
    pub async fn account_page(
        &self,
        subject_user_id: Uuid,
    ) -> Result<(Option<i64>, Vec<AccountEvent>), DomainError> {
        self.isle
            .call(move |conn| {
                let locked_at = conn
                    .query_row(
                        "SELECT locked_at FROM user_account WHERE user_id = ?1",
                        params![subject_user_id],
                        |row| row.get::<_, Option<i64>>(0),
                    )
                    .optional()?
                    .flatten();
                let events = conn
                    .prepare(
                        "SELECT seq, occurred_at, actor_user_id, actor_name,
                                subject_user_id, kind
                         FROM account_event WHERE subject_user_id = ?1
                         UNION ALL
                         SELECT seq, occurred_at, actor_user_id, actor_name,
                                subject_user_id, kind
                         FROM account_event WHERE subject_user_id IS NULL
                         ORDER BY seq",
                    )?
                    .query_map(params![subject_user_id], account_event_from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((locked_at, events))
            })
            .await
            .map_err(infra_err)
    }

    /// Sweeps every device token that may go on [`ended_sql`]'s terms
    /// — its window closed, or, for a token and not a tombstone, idle
    /// past the bound — returning how many rows went;
    /// [`Self::cleanup_expired`]'s sibling over the other table.
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
    ///
    /// Takes an idle token too when the instance has a bound, and a
    /// tombstone only with its window (#213): [`ended_sql`] is the one
    /// predicate, so a token this leaves is one the listing shows and
    /// the instance's revoke stamps, and a tombstone this leaves is
    /// one that has news to give.
    pub async fn cleanup_expired_device_tokens(
        &self,
        now_ms: i64,
        idle_ms: Option<i64>,
    ) -> Result<u64, DomainError> {
        let swept = self
            .isle
            .call(move |conn| {
                conn.execute(
                    &format!("DELETE FROM device_token WHERE {}", ended_sql(1, 2)),
                    params![now_ms, idle_ms],
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
                let row: Option<(Uuid, String, Option<i64>)> = conn
                    .query_row(
                        "SELECT user_id, password_hash, locked_at
                         FROM user_account WHERE login = ?1",
                        params![login],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                Ok(match row {
                    // An account with no password (#163) and an
                    // account an admin locked (#213) both take the
                    // unknown-login arm: the same work, the same
                    // answer, so the password arm cannot say which
                    // accounts sign in elsewhere or which are locked.
                    Some((_, phc, locked_at)) if phc == LOCKED_PASSWORD || locked_at.is_some() => {
                        let _ = verify_password(&secret, dummy_hash());
                        Ok(None)
                    }
                    Some((user_id, phc, _)) => {
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

/// Maps a `seq, occurred_at, actor_user_id, actor_name,
/// subject_user_id, kind` row into an [`AccountEvent`] — the one place
/// the row-to-field mapping is spelled; the selects that produce the
/// row spell the same order.
fn account_event_from_row(row: &rusqlite::Row<'_>) -> Result<AccountEvent, rusqlite::Error> {
    Ok(AccountEvent {
        seq: row.get(0)?,
        occurred_at_ms: row.get(1)?,
        actor_user_id: row.get(2)?,
        actor_name: row.get(3)?,
        subject_user_id: row.get(4)?,
        kind: row.get(5)?,
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
    /// The lifetime these tests mint with — the binary's default,
    /// restated here because the policy is the server's and this
    /// crate only takes what it is handed.
    const TTL: i64 = 90 * 24 * 60 * 60 * 1000;

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
            .mint_device_token(user_id, "Hoshino's MacBook", T0, TTL)
            .await
            .unwrap();
        assert_eq!(minted.expires_at_ms, T0 + TTL);

        // Nothing has presented it yet, and the listing says so
        // instead of borrowing the mint instant.
        let listed = auth
            .list_device_tokens(user_id, T0 + 5_000, None)
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, minted.id);
        assert_eq!(listed[0].label, "Hoshino's MacBook");
        assert_eq!(listed[0].last_used_at_ms, None);

        let resolved = auth
            .resolve_device_token(&minted.token, T0 + 5_000, None)
            .await
            .unwrap()
            .account()
            .unwrap();
        assert_eq!(resolved.user_id, user_id);
        assert_eq!(resolved.display_name, "Hoshino");
        assert_eq!(
            auth.list_device_tokens(user_id, T0 + 5_000, None)
                .await
                .unwrap()[0]
                .last_used_at_ms,
            Some(T0 + 5_000),
            "a resolve stamps the use it was"
        );

        auth.revoke_device_token(user_id, minted.id).await.unwrap();
        assert_eq!(
            auth.resolve_device_token(&minted.token, T0 + 6_000, None)
                .await
                .unwrap(),
            DeviceTokenResolution::Unknown
        );
        assert_eq!(device_rows(&isle).await, 0);
        // Revoking again is idempotent, exactly as destroying a
        // session is.
        auth.revoke_device_token(user_id, minted.id).await.unwrap();

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_token_the_instance_took_back_says_so_once_and_is_then_unknown() {
        let (auth, isle, driver) = auth().await;
        let hoshino = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let kanade = auth
            .create_account("kanade", "Kanade", GOOD, false, T0)
            .await
            .unwrap();
        let laptop = auth
            .mint_device_token(hoshino, "laptop", T0, TTL)
            .await
            .unwrap();
        let phone = auth
            .mint_device_token(hoshino, "phone", T0, TTL)
            .await
            .unwrap();
        let theirs = auth
            .mint_device_token(kanade, "theirs", T0, TTL)
            .await
            .unwrap();

        assert_eq!(
            auth.revoke_device_tokens_of(hoshino, T0 + 1, None)
                .await
                .unwrap(),
            2
        );
        // The tombstones are not listed as live tokens…
        assert!(
            auth.list_device_tokens(hoshino, T0 + 2, None)
                .await
                .unwrap()
                .is_empty()
        );
        // …but they are rows, until presented.
        assert_eq!(device_rows(&isle).await, 3);
        for token in [&laptop.token, &phone.token] {
            assert_eq!(
                auth.resolve_device_token(token, T0 + 2, None)
                    .await
                    .unwrap(),
                DeviceTokenResolution::RevokedByInstance
            );
            assert_eq!(
                auth.resolve_device_token(token, T0 + 3, None)
                    .await
                    .unwrap(),
                DeviceTokenResolution::Unknown,
                "the news is given once"
            );
        }
        // Somebody else's token is untouched.
        assert!(
            auth.resolve_device_token(&theirs.token, T0 + 4, None)
                .await
                .unwrap()
                .account()
                .is_some()
        );
        // Everybody, then.
        assert_eq!(
            auth.revoke_every_device_token(T0 + 5, None).await.unwrap(),
            1
        );
        assert_eq!(
            auth.resolve_device_token(&theirs.token, T0 + 6, None)
                .await
                .unwrap(),
            DeviceTokenResolution::RevokedByInstance
        );
        // A tombstone nothing presents goes with the expired.
        let ghost = auth
            .mint_device_token(kanade, "ghost", T0, TTL)
            .await
            .unwrap();
        auth.revoke_device_tokens_of(kanade, T0 + 7, None)
            .await
            .unwrap();
        assert_eq!(
            auth.cleanup_expired_device_tokens(T0 + TTL + 1, None)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            auth.resolve_device_token(&ghost.token, T0 + TTL + 2, None)
                .await
                .unwrap(),
            DeviceTokenResolution::Unknown
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_lock_never_masks_the_end_a_token_met() {
        let (auth, isle, driver) = auth().await;
        let hoshino = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let taken = auth
            .mint_device_token(hoshino, "taken back", T0, TTL)
            .await
            .unwrap();
        // A token that had already expired, and one that had gone
        // idle, when the instance took everything back: neither is
        // stamped, because neither was ended by the instance.
        let stale = auth
            .mint_device_token(hoshino, "stale", T0, 1_000)
            .await
            .unwrap();
        let idle = auth
            .mint_device_token(hoshino, "idle", T0, TTL)
            .await
            .unwrap();
        // `taken` is in use — presented once, which is what keeps it
        // inside the idle bound at the revoke.
        assert!(
            auth.resolve_device_token(&taken.token, T0 + 1_000, None)
                .await
                .unwrap()
                .account()
                .is_some()
        );
        // #213's own order: sign everybody out, then lock — with a
        // token minted between the two that would otherwise resolve.
        assert_eq!(
            auth.revoke_device_tokens_of(hoshino, T0 + 2_000, Some(1_500))
                .await
                .unwrap(),
            1,
            "only the live token was taken back"
        );
        let live = auth
            .mint_device_token(hoshino, "live", T0 + 2_001, TTL)
            .await
            .unwrap();
        auth.lock_account(hoshino, T0 + 2_002).await.unwrap();

        // A token that no longer exists says how it ended, lock or no
        // lock, and its row goes; only the token that would otherwise
        // resolve is answered with the lock, and stays.
        assert_eq!(
            auth.resolve_device_token(&taken.token, T0 + 2_003, None)
                .await
                .unwrap(),
            DeviceTokenResolution::RevokedByInstance
        );
        assert_eq!(
            auth.resolve_device_token(&stale.token, T0 + 2_004, None)
                .await
                .unwrap(),
            DeviceTokenResolution::Expired
        );
        assert_eq!(
            auth.resolve_device_token(&idle.token, T0 + 2_005, Some(1_500))
                .await
                .unwrap(),
            DeviceTokenResolution::Idle
        );
        assert_eq!(
            auth.resolve_device_token(&live.token, T0 + 2_006, None)
                .await
                .unwrap(),
            DeviceTokenResolution::Locked
        );
        assert_eq!(device_rows(&isle).await, 1);
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_tombstone_outlives_the_idle_bound_and_goes_with_the_window() {
        let (auth, isle, driver) = auth().await;
        let hoshino = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        // Used once, long ago: one tick inside a 30-unit idle bound at
        // the revoke, and far past it by the time anything sweeps.
        let old = auth
            .mint_device_token(hoshino, "old phone", T0, 1_000)
            .await
            .unwrap();
        assert!(
            auth.resolve_device_token(&old.token, T0 + 10, None)
                .await
                .unwrap()
                .account()
                .is_some()
        );
        assert_eq!(
            auth.revoke_device_tokens_of(hoshino, T0 + 39, Some(30))
                .await
                .unwrap(),
            1
        );
        // A plain token idle past the bound goes with the same sweep
        // that leaves the tombstone: it is a token, and idleness is
        // its end.
        let _forgotten = auth
            .mint_device_token(hoshino, "forgotten tablet", T0 + 100, 1_000)
            .await
            .unwrap();
        assert_eq!(
            auth.cleanup_expired_device_tokens(T0 + 500, Some(30))
                .await
                .unwrap(),
            1,
            "the idle token went; the tombstone did not"
        );
        assert_eq!(device_rows(&isle).await, 1);
        // Presented, it says what it was kept to say — not `Idle`, not
        // `Unknown`.
        assert_eq!(
            auth.resolve_device_token(&old.token, T0 + 600, Some(30))
                .await
                .unwrap(),
            DeviceTokenResolution::RevokedByInstance
        );
        assert_eq!(device_rows(&isle).await, 0);
        // And one nothing presents goes with its window.
        let ghost = auth
            .mint_device_token(hoshino, "ghost", T0, 1_000)
            .await
            .unwrap();
        auth.revoke_device_tokens_of(hoshino, T0 + 1, Some(30))
            .await
            .unwrap();
        assert_eq!(
            auth.cleanup_expired_device_tokens(T0 + 999, Some(30))
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            auth.cleanup_expired_device_tokens(T0 + 1_000, Some(30))
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            auth.resolve_device_token(&ghost.token, T0 + 1_001, Some(30))
                .await
                .unwrap(),
            DeviceTokenResolution::Unknown
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_locked_account_resolves_no_credential_and_gets_them_back_unlocked() {
        let (auth, _isle, driver) = auth().await;
        let hoshino = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let session = auth.create_session(hoshino, T0, 60_000).await.unwrap();
        let device = auth
            .mint_device_token(hoshino, "laptop", T0, TTL)
            .await
            .unwrap();
        assert_eq!(auth.locked_at(hoshino).await.unwrap(), None);

        assert_eq!(
            auth.lock_account(hoshino, T0 + 1).await.unwrap(),
            LockOutcome::Locked
        );
        assert_eq!(auth.locked_at(hoshino).await.unwrap(), Some(T0 + 1));
        // Locking again is not what locked it, and keeps the first
        // instant.
        assert_eq!(
            auth.lock_account(hoshino, T0 + 2).await.unwrap(),
            LockOutcome::Unchanged
        );
        assert_eq!(auth.locked_at(hoshino).await.unwrap(), Some(T0 + 1));
        // An account that does not exist is the same answer.
        assert_eq!(
            auth.lock_account(Uuid::now_v7(), T0 + 2).await.unwrap(),
            LockOutcome::Unchanged
        );

        // The password arm, the device arm, the session it held.
        let verifier: &dyn CredentialVerifier = &auth;
        assert_eq!(verifier.verify("hoshino", GOOD).await.unwrap(), None);
        assert_eq!(
            auth.resolve_device_token(&device.token, T0 + 3, None)
                .await
                .unwrap(),
            DeviceTokenResolution::Locked
        );
        assert_eq!(auth.resolve_session(&session, T0 + 3).await.unwrap(), None);
        // The rows stay: the account is still an account with a name.
        assert_eq!(
            auth.account(hoshino).await.unwrap().unwrap().display_name,
            "Hoshino"
        );

        assert!(auth.unlock_account(hoshino).await.unwrap());
        assert_eq!(auth.locked_at(hoshino).await.unwrap(), None);
        assert_eq!(
            verifier.verify("hoshino", GOOD).await.unwrap(),
            Some(hoshino)
        );
        assert!(
            auth.resolve_device_token(&device.token, T0 + 4, None)
                .await
                .unwrap()
                .account()
                .is_some()
        );
        assert!(
            auth.resolve_session(&session, T0 + 4)
                .await
                .unwrap()
                .is_some()
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_last_admin_who_can_authenticate_is_locked_by_nobody() {
        let (auth, _isle, driver) = auth().await;
        let first = auth
            .create_account("first", "First", GOOD, true, T0)
            .await
            .unwrap();
        let second = auth
            .create_account("second", "Second", GOOD, true, T0)
            .await
            .unwrap();
        let member = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        // Two admins can authenticate: locking one is allowed.
        assert_eq!(
            auth.lock_account(second, T0 + 1).await.unwrap(),
            LockOutcome::Locked
        );
        // Now one can: locking it is refused, whoever asks, and the
        // refusal says why rather than "nothing changed".
        assert_eq!(
            auth.lock_account(first, T0 + 2).await.unwrap(),
            LockOutcome::LastAdmin
        );
        assert_eq!(auth.locked_at(first).await.unwrap(), None);
        // A member is never the last admin.
        assert_eq!(
            auth.lock_account(member, T0 + 3).await.unwrap(),
            LockOutcome::Locked
        );
        // Lift the second's lock and the first may be locked again.
        assert!(auth.unlock_account(second).await.unwrap());
        assert_eq!(
            auth.lock_account(first, T0 + 4).await.unwrap(),
            LockOutcome::Locked
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_instance_record_keeps_every_act_and_answers_per_account() {
        let (auth, _isle, driver) = auth().await;
        let operator = auth
            .create_account("operator", "Operator", GOOD, true, T0)
            .await
            .unwrap();
        let hoshino = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let kanade = auth
            .create_account("kanade", "Kanade", GOOD, false, T0)
            .await
            .unwrap();
        let actor = auth.account(operator).await.unwrap().unwrap();
        auth.record_account_event(&actor, Some(hoshino), "locked", T0 + 1)
            .await
            .unwrap();
        auth.record_account_event(&actor, None, "devices_revoked", T0 + 2)
            .await
            .unwrap();
        auth.record_account_event(&actor, Some(kanade), "locked", T0 + 3)
            .await
            .unwrap();

        let all = auth.account_events().await.unwrap();
        assert_eq!(
            all.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            ["locked", "devices_revoked", "locked"]
        );
        assert_eq!(all[0].actor_name, "Operator");
        assert_eq!(all[1].subject_user_id, None);
        // One account's page: its own acts and the acts on everybody.
        let (lock, hers) = auth.account_page(hoshino).await.unwrap();
        assert_eq!(lock, None, "the page carries the lock, and there is none");
        assert_eq!(
            hers.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [all[0].seq, all[1].seq]
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn an_owners_revoke_reaches_only_their_own_handle() {
        let (auth, isle, driver) = auth().await;
        let mine = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let theirs = auth
            .create_account("someone", "Someone Else", GOOD, false, T0)
            .await
            .unwrap();
        let minted = auth
            .mint_device_token(mine, "MacBook", T0, TTL)
            .await
            .unwrap();

        // The other account's revoke is not an error and is also not a
        // deletion: it matched nothing, which is the same answer a
        // handle that never existed gets.
        auth.revoke_device_token(theirs, minted.id).await.unwrap();
        assert_eq!(device_rows(&isle).await, 1);
        assert!(
            auth.resolve_device_token(&minted.token, T0 + 1, None)
                .await
                .unwrap()
                .account()
                .is_some()
        );
        // And the listing is scoped the same way.
        assert!(
            auth.list_device_tokens(theirs, T0, None)
                .await
                .unwrap()
                .is_empty()
        );

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
            .mint_device_token(user_id, "MacBook", T0, TTL)
            .await
            .unwrap();
        assert_eq!(device_rows(&isle).await, 1);

        assert_eq!(
            auth.resolve_device_token(&minted.token, T0 + TTL, None)
                .await
                .unwrap(),
            DeviceTokenResolution::Expired
        );
        assert_eq!(
            device_rows(&isle).await,
            0,
            "rejection is cleanup for the row that was touched"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn an_idle_device_token_ends_early_and_a_use_resets_the_clock() {
        // A second instance, opened first so the fixture is still in
        // scope: this one has no idle bound, for the contrast below.
        let (auth2, isle2, driver2) = auth().await;
        let (auth, isle, driver) = auth().await;
        let user_id = auth
            .create_account("hoshino", "Hoshino", GOOD, false, T0)
            .await
            .unwrap();
        let idle = Some(7 * 24 * 60 * 60 * 1000);
        let minted = auth
            .mint_device_token(user_id, "MacBook", T0, TTL)
            .await
            .unwrap();

        // Never presented: the mint is what the idle clock runs from.
        assert!(
            auth.resolve_device_token(&minted.token, T0 + 6 * 24 * 60 * 60 * 1000, idle)
                .await
                .unwrap()
                .account()
                .is_some()
        );
        // That use moved the clock, so six more days is still live...
        assert!(
            auth.resolve_device_token(&minted.token, T0 + 12 * 24 * 60 * 60 * 1000, idle)
                .await
                .unwrap()
                .account()
                .is_some()
        );
        // ...and seven from the last use is not, for an instance with
        // the bound; the same moment resolves for one without it, since
        // the fixed window is nowhere near.
        let later = T0 + 19 * 24 * 60 * 60 * 1000;
        let user2 = auth2
            .create_account("kanade", "Kanade", GOOD, false, T0)
            .await
            .unwrap();
        let unbounded = auth2
            .mint_device_token(user2, "MacBook", T0, TTL)
            .await
            .unwrap();
        assert!(
            auth2
                .resolve_device_token(&unbounded.token, later, None)
                .await
                .unwrap()
                .account()
                .is_some()
        );
        assert_eq!(
            auth.resolve_device_token(&minted.token, later, idle)
                .await
                .unwrap(),
            DeviceTokenResolution::Idle
        );
        assert_eq!(device_rows(&isle).await, 0, "idle is cleanup too");
        assert_eq!(device_rows(&isle2).await, 1);

        driver.shutdown().await.unwrap();
        driver2.shutdown().await.unwrap();
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
            .mint_device_token(user_id, "old laptop", T0 - TTL, TTL)
            .await
            .unwrap();
        let live = auth
            .mint_device_token(user_id, "MacBook", T0, TTL)
            .await
            .unwrap();

        assert_eq!(
            auth.cleanup_expired_device_tokens(T0, None).await.unwrap(),
            1
        );
        assert_eq!(device_rows(&isle).await, 1);
        assert_eq!(
            auth.resolve_device_token(&dead.token, T0 + 1, None)
                .await
                .unwrap(),
            DeviceTokenResolution::Unknown
        );
        assert!(
            auth.resolve_device_token(&live.token, T0 + 1, None)
                .await
                .unwrap()
                .account()
                .is_some()
        );
        // The two sweeps answer for their own tables and nothing else.
        let session = auth.create_session(user_id, T0, 60_000).await.unwrap();
        assert_eq!(
            auth.cleanup_expired_device_tokens(T0 + 1, None)
                .await
                .unwrap(),
            0
        );
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
            let refused = auth.mint_device_token(user_id, label, T0, TTL).await;
            assert!(
                matches!(refused, Err(DomainError::Validation(_))),
                "{label:?} must be refused"
            );
        }
        let unknown = auth
            .mint_device_token(Uuid::now_v7(), "MacBook", T0, TTL)
            .await;
        assert!(matches!(unknown, Err(DomainError::Validation(_))));
        assert_eq!(device_rows(&isle).await, 0, "nothing landed");

        // The label is stored trimmed, so the list shows what a person
        // typed rather than what their terminal added.
        auth.mint_device_token(user_id, "  MacBook  ", T0, TTL)
            .await
            .unwrap();
        assert_eq!(
            auth.list_device_tokens(user_id, T0 + 5_000, None)
                .await
                .unwrap()[0]
                .label,
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
            .mint_device_token(user_id, "MacBook", T0, TTL)
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
