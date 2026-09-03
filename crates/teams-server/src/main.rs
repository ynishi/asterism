//! `teams-server` — the hosted Team plane's binary (#83 §4/§5).
//!
//! ## Role
//!
//! Serves the `/teams/*` HTTP API (auth v0 + team/membership routes —
//! the #91 slice; blobs — #93; purge — #95; the hosted forge and the
//! subject-filtered ledger read — #151) over the teams-owned
//! SQLite database, and carries the instance's maintenance verbs:
//! `gc` (the zero-link sweep on demand) and `backup` (quiesce →
//! `VACUUM INTO` → one DB-first archive), both #95. Fully separate
//! from the `asterism-server` binary: what the two share is library
//! crates that depend on no teams-\* crate, so the license boundary
//! sits at the bin edge (#83 §4). Which ones, and why each is
//! permitted, is stated once — in this crate's `Cargo.toml`, beside
//! the dependency lines themselves.
//! The MCP surface is a later slice.
//!
//! ## Identity bootstrap
//!
//! No fixed default credentials exist (#83 §5). The initial admin —
//! the `InstanceAdmin` of #83 §1, outside every membership table — is
//! created explicitly with `bootstrap-admin`, taking its password from
//! `$ASTERISM_TEAMS_ADMIN_PASSWORD` (an environment variable rather
//! than an argument, so the secret stays out of shell history and
//! process listings). Placeholder passwords are refused outright.
//! `create-user` provisions ordinary accounts the same way (from
//! `$ASTERISM_TEAMS_USER_PASSWORD`) — v0's account source until a
//! registration surface ships; invited members must already hold an
//! account.
//!
//! ## The identity provider (#163)
//!
//! An instance may sign its members in through one OIDC provider
//! instead of a password: `serve --oidc-issuer … --oidc-client-id …
//! --public-url …` with the client secret in
//! `$ASTERISM_TEAMS_OIDC_CLIENT_SECRET` — an environment variable for
//! the reason the passwords are. `create-user --oidc-email` and
//! `link-oidc` say which account an address at the provider signs in
//! as. Why the instance rather than the app is the provider's client
//! is `teams_infra::auth::oidc`'s. Without the arguments the instance
//! is what it was.
//!
//! ## Binding
//!
//! Loopback by default, like the sibling binary. Unlike the local app,
//! a team server is meant to be reached by its members, so `--bind`
//! exists — making the instance reachable is the operator's explicit
//! decision, never a default.

#![warn(missing_docs)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use teams_core::domain::identity::RegistrationPolicy;
use teams_infra::auth::oidc::{OidcClient, OidcConfig, OidcIdentities};
use teams_infra::auth::password::PasswordAuth;
use teams_infra::sqlite::SqliteTeamsRepository;
use teams_server::http;
use teams_server::oidc::OidcSignIn;
use teams_server::rate_limit::RateLimiter;
use teams_server::state::{
    AUTH_RATE_LIMIT_WINDOW, DEFAULT_AUTH_RATE_LIMIT, DEFAULT_DEVICE_TOKEN_IDLE_MS,
    DEFAULT_DEVICE_TOKEN_TTL_MS, DEFAULT_SESSION_TTL_MS, TeamsCtx, now_ms,
};

/// A millisecond span as whole days, for the two device-token
/// arguments: the context's constants are the numbers, and the
/// arguments are how an operator spells them.
const fn days(ms: i64) -> u32 {
    (ms / (24 * 60 * 60 * 1000)) as u32
}

/// Default HTTP port. Its own number, near the local app's profile
/// ports (8989 / 18989 / 28989) but colliding with none of them — the
/// two binaries are expected to coexist on one host.
const DEFAULT_PORT: u16 = 9989;

/// Where `bootstrap-admin` reads its password from.
const ADMIN_PASSWORD_ENV: &str = "ASTERISM_TEAMS_ADMIN_PASSWORD";
/// Where `create-user` reads its password from.
const USER_PASSWORD_ENV: &str = "ASTERISM_TEAMS_USER_PASSWORD";
/// Where `serve` reads the provider's client secret from (#163) — an
/// environment variable for the reason the passwords are: it stays out
/// of shell history and process listings.
const OIDC_CLIENT_SECRET_ENV: &str = "ASTERISM_TEAMS_OIDC_CLIENT_SECRET";

/// Asterism teams server (hosted Team plane, #83).
#[derive(Parser)]
#[command(name = "teams-server", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serves the `/teams/*` HTTP API.
    Serve {
        /// SQLite database path (default: the active teams profile;
        /// override with `$ASTERISM_TEAMS_HOME`).
        #[arg(long)]
        db: Option<PathBuf>,
        /// Blob store root (default: `blobs/` in the active teams
        /// profile home — beside the database, so the #83 §3 ordering
        /// rule spans one machine's durability). Opening it sweeps
        /// `staging/` of any interrupted copies.
        #[arg(long)]
        blobs: Option<PathBuf>,
        /// Listen port.
        #[arg(long)]
        port: Option<u16>,
        /// Bind address. Loopback by default; binding a reachable
        /// address is the operator's explicit call.
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        /// Closed registration (#83 §1): only admins may create teams.
        #[arg(long)]
        closed_registration: bool,
        /// Purge grace window in seconds (#95): how long a marked blob
        /// link stays restorable before reclaim may remove it. Default
        /// 7 days — GitLab's delayed-deletion period, the precedent
        /// #83 §1 names for the trash→purge shape.
        #[arg(long, default_value_t = 7 * 24 * 60 * 60)]
        purge_grace_seconds: u32,
        /// How many days a device token lives from its mint (#204,
        /// #163). A ceiling, fixed per token at the mint; the context's
        /// default is the number and says why it is a setting.
        #[arg(long, default_value_t = days(DEFAULT_DEVICE_TOKEN_TTL_MS))]
        device_token_days: u32,
        /// How many days a device token may go unpresented before it
        /// stops resolving. `0` turns the bound off.
        #[arg(long, default_value_t = days(DEFAULT_DEVICE_TOKEN_IDLE_MS))]
        device_token_idle_days: u32,
        /// How many hits a minute one client address may make on the
        /// auth routes the limiter covers (#83 §5) — which those are,
        /// and why the one that presents no credential (starting a
        /// provider attempt) is among them, is the router's comment in
        /// `http`. Why the default is what it is, and who raises it, is
        /// on the constant. `0` refuses every hit; it does not turn the
        /// limiter off.
        #[arg(long, default_value_t = DEFAULT_AUTH_RATE_LIMIT)]
        auth_rate_limit: u32,
        /// The identity provider's issuer URL (#163) — the `iss` its
        /// ID tokens carry, and where `/.well-known/openid-configuration`
        /// is found. Given together with `--oidc-client-id` and
        /// `--public-url`, and with the client secret in
        /// `$ASTERISM_TEAMS_OIDC_CLIENT_SECRET`, it makes this instance
        /// the provider's OAuth client; absent, the instance signs
        /// people in with passwords only.
        #[arg(long, requires_all = ["oidc_client_id", "public_url"])]
        oidc_issuer: Option<String>,
        /// The client id the provider registered this instance under.
        #[arg(long, requires = "oidc_issuer")]
        oidc_client_id: Option<String>,
        /// What the app calls the provider on its button — "Google",
        /// "Okta". Default: the issuer's host.
        #[arg(long, requires = "oidc_issuer")]
        oidc_name: Option<String>,
        /// The origin members' browsers reach this instance at,
        /// `https://teams.example.com` — what the provider is told to
        /// send the browser back to, and what a sign-in page's URL is
        /// built on. Register `<public-url>/teams/auth/oidc/callback`
        /// as the redirect URI at the provider. Nothing reads it
        /// without a provider, so alone it is refused.
        #[arg(long, requires = "oidc_issuer")]
        public_url: Option<String>,
    },
    /// Creates the database (if missing) and applies every pending
    /// migration. Idempotent — safe to re-run.
    Init {
        /// SQLite database path.
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Creates an instance admin (#83 §1), outside every membership
    /// table. Password from `$ASTERISM_TEAMS_ADMIN_PASSWORD`; there is
    /// no default, and placeholder passwords are refused.
    ///
    /// Named for the case it exists for — the first admin on an
    /// instance with no account to authenticate as — but not limited
    /// to it. It refused a second admin until #148 revision 8, on the
    /// ground that the capacity had exactly one holder; an instance
    /// whose only admin is unreachable has no way back to its own
    /// destructive verbs, so provisioning another is an ordinary run
    /// of this command rather than a feature somebody has to add.
    BootstrapAdmin {
        /// SQLite database path.
        #[arg(long)]
        db: Option<PathBuf>,
        /// The admin's login name.
        #[arg(long)]
        login: String,
        /// Display name for ledger stamps (default: the login).
        #[arg(long)]
        display_name: Option<String>,
    },
    /// Provisions an ordinary user account. Password from
    /// `$ASTERISM_TEAMS_USER_PASSWORD`; same refusals as the admin's.
    /// With `--oidc-email`, no password: the account signs in through
    /// the provider and holds none (#163).
    CreateUser {
        /// SQLite database path.
        #[arg(long)]
        db: Option<PathBuf>,
        /// The user's login name.
        #[arg(long)]
        login: String,
        /// Display name for ledger stamps (default: the login).
        #[arg(long)]
        display_name: Option<String>,
        /// The address the person signs in with at the provider. The
        /// first sign-in whose verified email matches pins the
        /// provider's subject to the account; no password is read or
        /// stored.
        #[arg(long, requires = "oidc_issuer")]
        oidc_email: Option<String>,
        /// The provider the address belongs to — the same issuer URL
        /// `serve --oidc-issuer` is given. Meaningless without
        /// `--oidc-email`, and refused alone rather than silently
        /// making a password account.
        #[arg(long, requires = "oidc_email")]
        oidc_issuer: Option<String>,
    },
    /// Binds an existing account to its address at the identity
    /// provider (#163). Rebinding unpins: the next verified sign-in
    /// with the new address pins afresh.
    LinkOidc {
        /// SQLite database path.
        #[arg(long)]
        db: Option<PathBuf>,
        /// The account's login name.
        #[arg(long)]
        login: String,
        /// The address the person signs in with at the provider.
        #[arg(long)]
        email: String,
        /// The provider the address belongs to — the same issuer URL
        /// `serve --oidc-issuer` is given.
        #[arg(long)]
        oidc_issuer: String,
    },
    /// Runs the zero-link sweep on demand (#95): deletes blob bytes no
    /// team links anymore. Links marked for purge still count as links
    /// — their bytes survive the grace window. Single-process
    /// assumption (#93): run this against a stopped server; a running
    /// server sweeps for itself after every reclaim. Never migrates
    /// the database — a schema behind this build is refused.
    Gc {
        /// SQLite database path.
        #[arg(long)]
        db: Option<PathBuf>,
        /// Blob store root.
        #[arg(long)]
        blobs: Option<PathBuf>,
    },
    /// Backs the instance up into ONE archive file: quiesce writes →
    /// SQLite snapshot via `VACUUM INTO` (never a live-file copy) →
    /// DB snapshot + blob dir into a plain tar at DESTINATION.
    #[command(long_about = "\
Backs the instance up into ONE archive file (#83 §4, gitea-dump shape):

  1. quiesce writes — the snapshot runs on the single writer, so no
     repository write interleaves with it
  2. SQLite snapshot via VACUUM INTO — never a copy of the live file
     (that is a documented corruption path); the snapshot lands in a
     local temp dir first, because live SQLite must never sit on
     network storage — the (possibly network-mounted) DESTINATION
     receives only the finished archive
  3. one plain uncompressed tar: db/teams.db FIRST, blobs/sha256/…
     after — so the worst inconsistency an archive can hold is an
     orphan blob (harmless; the restored instance's gc collects it),
     never a dangling DB reference

DESTINATION must be a fresh path; an existing file is refused rather
than overwritten. Run it against a stopped or idle server: a reclaim
landing mid-backup could remove bytes the snapshot still links. The
command never migrates the database — a schema that does not match
this build is refused, so a newer binary archives a pre-upgrade
instance as it stands or not at all.

RESTORE (documentation, not a command):

  1. tar -xf <archive> -C <dir>
  2. teams-server serve --db <dir>/db/teams.db --blobs <dir>/blobs

That is the whole procedure — the archive holds everything an instance
is (state, ledger, credentials, blob bytes). Members' clients keep
their own copies too, so anything promoted after the backup can simply
be promoted again (#83 §4's second recovery path).")]
    Backup {
        /// SQLite database path.
        #[arg(long)]
        db: Option<PathBuf>,
        /// Blob store root.
        #[arg(long)]
        blobs: Option<PathBuf>,
        /// Where the archive file is written (any mounted/rclone-
        /// reachable target). Must not already exist.
        destination: PathBuf,
    },
}

fn resolve_db_path(db: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    match db {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Ok(path)
        }
        None => Ok(teams_infra::paths::default_db_path()?),
    }
}

/// Like [`resolve_db_path`], but for commands that operate on an
/// existing instance (`gc`, `backup`): opening a database path that
/// holds nothing would silently manufacture an empty instance — and
/// then "back up" nothing, or sweep every blob as unlinked. Refusing
/// is the honest answer.
fn existing_db_path(db: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let path = match db {
        Some(path) => path,
        None => teams_infra::paths::default_db_path()?,
    };
    if !path.is_file() {
        anyhow::bail!(
            "no teams database at {}; this command operates on an existing instance \
             (create one with `teams-server init`)",
            path.display()
        );
    }
    Ok(path)
}

fn password_from_env(var: &str) -> anyhow::Result<String> {
    std::env::var(var).map_err(|_| {
        anyhow::anyhow!(
            "set ${var} to the account's password; \
             this instance has no default credentials (#83 §5)"
        )
    })
}

async fn create_account(
    db: Option<PathBuf>,
    login: &str,
    display_name: Option<String>,
    password_env: &str,
    admin: bool,
) -> anyhow::Result<()> {
    let password = password_from_env(password_env)?;
    let db_path = resolve_db_path(db)?;
    let (isle, driver) = teams_infra::sqlite::open_and_migrate(&db_path).await?;
    let auth = PasswordAuth::new(isle);
    let display_name = display_name.unwrap_or_else(|| login.to_string());
    let outcome = auth
        .create_account(login, &display_name, &password, admin, now_ms())
        .await;
    driver.shutdown().await.ok();
    let user_id = outcome?;
    let kind = if admin { "admin" } else { "user" };
    println!("teams-server: {kind} {login:?} created (user_id {user_id})");
    Ok(())
}

/// Provisions an account that signs in through the provider and holds
/// no password (#163): the locked row, and the binding beside it.
async fn create_provider_account(
    db: Option<PathBuf>,
    login: &str,
    display_name: Option<String>,
    issuer: &str,
    email: &str,
) -> anyhow::Result<()> {
    let db_path = resolve_db_path(db)?;
    let (isle, driver) = teams_infra::sqlite::open_and_migrate(&db_path).await?;
    let auth = PasswordAuth::new(isle.clone());
    let identities = OidcIdentities::new(isle);
    let display_name = display_name.unwrap_or_else(|| login.to_string());
    // Two writes, not one transaction: the account is created, then
    // bound. A binding refused after the account landed — an address
    // another account holds, say — leaves an account nobody can sign
    // in as, and the message says what completes it.
    let outcome = async {
        let user_id = auth
            .create_account_locked(login, &display_name, false, now_ms())
            .await?;
        identities
            .bind_email(user_id, issuer, email)
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "{err}; the account {login:?} exists and holds no password — \
                     `link-oidc --login {login} --email <address> --oidc-issuer {issuer}` \
                     completes it"
                )
            })?;
        Ok::<_, anyhow::Error>(user_id)
    }
    .await;
    driver.shutdown().await.ok();
    let user_id = outcome?;
    println!(
        "teams-server: user {login:?} created (user_id {user_id}), signs in as {email:?} at {issuer}"
    );
    Ok(())
}

/// Binds an existing account to its provider address (#163).
async fn link_provider_account(
    db: Option<PathBuf>,
    login: &str,
    issuer: &str,
    email: &str,
) -> anyhow::Result<()> {
    let db_path = existing_db_path(db)?;
    let (isle, driver) = teams_infra::sqlite::open_and_migrate(&db_path).await?;
    let auth = PasswordAuth::new(isle.clone());
    let identities = OidcIdentities::new(isle);
    let outcome = async {
        let account = auth
            .account_by_login(login)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no account is registered as {login:?}"))?;
        identities
            .bind_email(account.user_id, issuer, email)
            .await?;
        Ok::<_, anyhow::Error>(account.user_id)
    }
    .await;
    driver.shutdown().await.ok();
    let user_id = outcome?;
    println!("teams-server: user {login:?} (user_id {user_id}) signs in as {email:?} at {issuer}");
    Ok(())
}

/// The provider half of the context, from the `serve` arguments — or
/// `None` when no issuer was given, in which case nothing about the
/// instance changes.
fn provider_from_args(
    identities: OidcIdentities,
    issuer: Option<String>,
    client_id: Option<String>,
    name: Option<String>,
    public_url: Option<String>,
) -> anyhow::Result<Option<Arc<OidcSignIn>>> {
    let Some(issuer) = issuer else {
        return Ok(None);
    };
    // clap's `requires_all` has already refused the other two missing;
    // the unwraps below are that refusal restated as types.
    let client_id = client_id.expect("clap requires --oidc-client-id with --oidc-issuer");
    let public_url = public_url.expect("clap requires --public-url with --oidc-issuer");
    let public_url = public_url.trim_end_matches('/').to_string();
    let client_secret = std::env::var(OIDC_CLIENT_SECRET_ENV).map_err(|_| {
        anyhow::anyhow!(
            "set ${OIDC_CLIENT_SECRET_ENV} to the client secret the provider issued; \
             this instance has no default credentials (#83 §5)"
        )
    })?;
    // The issuer's host, for a button nobody named: everything after
    // the scheme and before the first slash, which is a host and a
    // port at most — an issuer URL carries no credentials or query.
    let display_name = name.unwrap_or_else(|| {
        let after_scheme = issuer
            .split_once("://")
            .map_or(issuer.as_str(), |(_, rest)| rest);
        after_scheme
            .split('/')
            .next()
            .filter(|host| !host.is_empty())
            .unwrap_or(issuer.as_str())
            .to_string()
    });
    let client = OidcClient::new(OidcConfig {
        issuer,
        client_id,
        client_secret,
        redirect_url: format!("{public_url}/teams/auth/oidc/callback"),
        display_name,
    });
    Ok(Some(Arc::new(OidcSignIn::new(
        client,
        identities,
        &public_url,
    ))))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Serve {
            db,
            blobs,
            port,
            bind,
            closed_registration,
            purge_grace_seconds,
            device_token_days,
            device_token_idle_days,
            auth_rate_limit,
            oidc_issuer,
            oidc_client_id,
            oidc_name,
            public_url,
        } => {
            let db_path = resolve_db_path(db)?;
            let (isle, _driver) = teams_infra::sqlite::open_and_migrate(&db_path).await?;
            let oidc = provider_from_args(
                OidcIdentities::new(isle.clone()),
                oidc_issuer,
                oidc_client_id,
                oidc_name,
                public_url,
            )?;
            let blob_root = match blobs {
                Some(path) => path,
                None => teams_infra::paths::default_blob_root()?,
            };
            // Opening the store runs the startup sweep of staging/
            // (#83 §3 — the one mechanical cleanup the blob layer owes).
            let blobs = teams_infra::blob::LocalFileStorageAdapter::open(blob_root).await?;
            let registration = if closed_registration {
                RegistrationPolicy::Closed
            } else {
                RegistrationPolicy::Open
            };
            let ctx = Arc::new(TeamsCtx {
                repo: SqliteTeamsRepository::new(isle.clone()),
                auth: PasswordAuth::new(isle.clone()),
                oidc,
                projections: teams_infra::sqlite::projection::SqliteProjectionStore::new(isle),
                blobs,
                registration,
                session_ttl_ms: DEFAULT_SESSION_TTL_MS,
                auth_limiter: RateLimiter::new(auth_rate_limit, AUTH_RATE_LIMIT_WINDOW),
                purge_grace_ms: i64::from(purge_grace_seconds) * 1000,
                device_token_ttl_ms: i64::from(device_token_days) * 24 * 60 * 60 * 1000,
                device_token_idle_ms: (device_token_idle_days > 0)
                    .then(|| i64::from(device_token_idle_days) * 24 * 60 * 60 * 1000),
                gc_guard: Arc::new(teams_infra::gc::GcGuard::new()),
            });
            let addr = SocketAddr::from((bind, port.unwrap_or(DEFAULT_PORT)));
            let listener = tokio::net::TcpListener::bind(addr).await?;
            eprintln!(
                "teams-server: http://{addr}/teams/* (db: {}, registration: {})",
                db_path.display(),
                if closed_registration {
                    "closed"
                } else {
                    "open"
                },
            );
            // Connect-info feeds the auth limiter its per-IP key.
            axum::serve(
                listener,
                http::router(ctx).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await?;
            Ok(())
        }
        Command::Init { db } => {
            let db_path = resolve_db_path(db)?;
            let (isle, driver) = teams_infra::sqlite::open_and_migrate(&db_path).await?;
            let version = teams_infra::sqlite::schema_version(&isle).await?;
            println!(
                "teams-server: db ready at {} (schema v{version})",
                db_path.display()
            );
            driver.shutdown().await.ok();
            Ok(())
        }
        Command::BootstrapAdmin {
            db,
            login,
            display_name,
        } => create_account(db, &login, display_name, ADMIN_PASSWORD_ENV, true).await,
        Command::CreateUser {
            db,
            login,
            display_name,
            oidc_email,
            oidc_issuer,
        } => match (oidc_email, oidc_issuer) {
            (Some(email), Some(issuer)) => {
                create_provider_account(db, &login, display_name, &issuer, &email).await
            }
            _ => create_account(db, &login, display_name, USER_PASSWORD_ENV, false).await,
        },
        Command::LinkOidc {
            db,
            login,
            email,
            oidc_issuer,
        } => link_provider_account(db, &login, &oidc_issuer, &email).await,
        Command::Gc { db, blobs } => {
            let db_path = existing_db_path(db)?;
            // No migration on a maintenance verb: the schema must
            // already be current, or the open refuses (#95).
            let (isle, driver) = teams_infra::sqlite::open_existing_at_latest(&db_path).await?;
            let blob_root = match blobs {
                Some(path) => path,
                None => teams_infra::paths::default_blob_root()?,
            };
            let adapter = teams_infra::blob::LocalFileStorageAdapter::open(blob_root).await?;
            let repo = SqliteTeamsRepository::new(isle);
            // A fresh guard: this process is the only one allowed to
            // be touching the instance (the subcommand's doc says so),
            // and within it the sweep is the sole CAS actor.
            let guard = Arc::new(teams_infra::gc::GcGuard::new());
            let outcome = teams_infra::gc::sweep_zero_link_blobs(&guard, &repo, &adapter).await;
            driver.shutdown().await.ok();
            let swept = outcome?;
            println!(
                "teams-server: gc swept {} blob(s){}",
                swept.len(),
                if swept.is_empty() { "" } else { ":" }
            );
            for digest in swept {
                println!("  {digest}");
            }
            Ok(())
        }
        Command::Backup {
            db,
            blobs,
            destination,
        } => {
            let db_path = existing_db_path(db)?;
            // No migration on a maintenance verb — the sharp corner is
            // backup-before-upgrade: a newer binary must archive the
            // instance as it stands, never migrate it first and
            // archive the migrated schema (#95).
            let (isle, driver) = teams_infra::sqlite::open_existing_at_latest(&db_path).await?;
            let blob_root = match blobs {
                Some(path) => path,
                None => teams_infra::paths::default_blob_root()?,
            };
            let outcome = teams_infra::backup::create_backup(&isle, &blob_root, &destination).await;
            driver.shutdown().await.ok();
            let report = outcome?;
            println!(
                "teams-server: backup written to {} (db snapshot {} bytes, {} blob file(s); \
                 restore: untar, then `teams-server serve --db <dir>/db/teams.db --blobs \
                 <dir>/blobs`)",
                report.archive.display(),
                report.db_snapshot_bytes,
                report.blob_files,
            );
            Ok(())
        }
    }
}
