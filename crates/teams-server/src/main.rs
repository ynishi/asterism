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
use teams_infra::auth::password::PasswordAuth;
use teams_infra::sqlite::SqliteTeamsRepository;
use teams_server::http;
use teams_server::rate_limit::RateLimiter;
use teams_server::state::{
    AUTH_RATE_LIMIT_MAX, AUTH_RATE_LIMIT_WINDOW, DEFAULT_SESSION_TTL_MS, TeamsCtx, now_ms,
};

/// Default HTTP port. Its own number, near the local app's profile
/// ports (8989 / 18989 / 28989) but colliding with none of them — the
/// two binaries are expected to coexist on one host.
const DEFAULT_PORT: u16 = 9989;

/// Where `bootstrap-admin` reads its password from.
const ADMIN_PASSWORD_ENV: &str = "ASTERISM_TEAMS_ADMIN_PASSWORD";
/// Where `create-user` reads its password from.
const USER_PASSWORD_ENV: &str = "ASTERISM_TEAMS_USER_PASSWORD";

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
        } => {
            let db_path = resolve_db_path(db)?;
            let (isle, _driver) = teams_infra::sqlite::open_and_migrate(&db_path).await?;
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
                projections: teams_infra::sqlite::projection::SqliteProjectionStore::new(isle),
                blobs,
                registration,
                session_ttl_ms: DEFAULT_SESSION_TTL_MS,
                auth_limiter: RateLimiter::new(AUTH_RATE_LIMIT_MAX, AUTH_RATE_LIMIT_WINDOW),
                purge_grace_ms: i64::from(purge_grace_seconds) * 1000,
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
        } => create_account(db, &login, display_name, USER_PASSWORD_ENV, false).await,
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
