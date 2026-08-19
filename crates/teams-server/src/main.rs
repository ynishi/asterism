//! `teams-server` — the hosted Team plane's binary (#83 §4/§5).
//!
//! ## Role
//!
//! Serves the `/teams/*` HTTP API (auth v0 + team/membership routes —
//! the #91 slice) over the teams-owned SQLite database. Fully separate
//! from the `asterism-server` binary: the two share `asterism-core`
//! (lib) only, so the license boundary sits at the bin edge (#83 §4).
//! The MCP surface and the `backup` command are later slices.
//!
//! ## Identity bootstrap
//!
//! No fixed default credentials exist (#83 §5). The initial admin —
//! the InstanceOperator of #83 §1, outside every membership table — is
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
        /// Closed registration (#83 §1): only the operator may create
        /// teams.
        #[arg(long)]
        closed_registration: bool,
    },
    /// Creates the database (if missing) and applies every pending
    /// migration. Idempotent — safe to re-run.
    Init {
        /// SQLite database path.
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Creates the initial admin — the InstanceOperator (#83 §1),
    /// outside every membership table. Password from
    /// `$ASTERISM_TEAMS_ADMIN_PASSWORD`; there is no default, and
    /// placeholder passwords are refused. Runs **once** per instance:
    /// the operator is an instance capacity with exactly one holder in
    /// v0, so re-running fails with "already bootstrapped" — minting
    /// further operators is a later deliberate feature, not a re-run.
    BootstrapAdmin {
        /// SQLite database path.
        #[arg(long)]
        db: Option<PathBuf>,
        /// The operator's login name.
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
    operator: bool,
) -> anyhow::Result<()> {
    let password = password_from_env(password_env)?;
    let db_path = resolve_db_path(db)?;
    let (isle, driver) = teams_infra::sqlite::open_and_migrate(&db_path).await?;
    let auth = PasswordAuth::new(isle);
    let display_name = display_name.unwrap_or_else(|| login.to_string());
    let outcome = auth
        .create_account(login, &display_name, &password, operator, now_ms())
        .await;
    driver.shutdown().await.ok();
    let user_id = outcome?;
    let kind = if operator { "operator" } else { "user" };
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
                auth: PasswordAuth::new(isle),
                blobs,
                registration,
                session_ttl_ms: DEFAULT_SESSION_TTL_MS,
                auth_limiter: RateLimiter::new(AUTH_RATE_LIMIT_MAX, AUTH_RATE_LIMIT_WINDOW),
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
    }
}
