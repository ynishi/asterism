//! # asterism-server — Asterism local API server (HTTP + MCP dual transport)
//!
//! ## Role
//!
//! Serves the domain and application layers of `asterism-core` as a local
//! API over two transports, which is why the binary is named
//! `asterism-server` rather than `asterism-mcp`.
//!
//! - **HTTP transport** (`serve` subcommand): axum, bound to loopback.
//!   Consumers include Lua scripts using `agent-block-core`, external
//!   persona tooling, and future bridge receivers. Route conventions live
//!   in the `http` module and mirror the Tauri command surface. The same
//!   router serves MCP over streamable-http at `/mcp` (see the `mcp`
//!   module for the curated tool set).
//! - **MCP transport** (`mcp` subcommand): a stdio **proxy** that
//!   forwards tools/resources to the running app's `/mcp` and owns the
//!   app's lifecycle (launch on access, `app_status` / `app_restart`).
//!   It starts with no backend and connects lazily, so the MCP client's
//!   session never depends on the app's start order (see `mcp_proxy`).
//! - **Shared database**: the UI process and the server process point at
//!   the same SQLite file under WAL, using `busy_timeout = 5000`. The
//!   default path is selected by the local data profile (override with
//!   `$ASTERISM_HOME`); resolution is shared with `asterism-ui` via
//!   `asterism_infra::paths`.
//! - **Authentication**: none in v1 — access is gated by binding to
//!   loopback. Tokens will follow in a later phase.
//! - **LLM path**: any future LLM-backed tool (for example `asset_ask`)
//!   will invoke `agent-block-core` indirectly through the plugin surface,
//!   keeping it out of the direct dependency graph.

#![warn(missing_docs)]

use asterism_server::{http, state};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Asterism local API server (HTTP + MCP dual transport).
#[derive(Parser)]
#[command(name = "asterism-server", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serves the HTTP API on loopback for agent-block scripts, personas,
    /// bridge receivers, and other local clients.
    Serve {
        /// SQLite database path (default: active profile;
        /// override with `$ASTERISM_HOME`).
        #[arg(long)]
        db: Option<PathBuf>,
        /// Listen port (bind address is fixed to `127.0.0.1`).
        #[arg(long)]
        port: Option<u16>,
    },
    /// Creates the database (if missing) and applies every pending
    /// migration up to the latest schema version. Idempotent — safe to
    /// re-run.
    Init {
        /// SQLite database path (default: active profile;
        /// override with `$ASTERISM_HOME`).
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Alias of [`Init`]; kept because "migrate" is the more familiar
    /// verb for existing databases.
    Migrate {
        /// SQLite database path.
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Prints the current schema version, asset count, and persona
    /// count. Read-only — never migrates.
    Status {
        /// SQLite database path.
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Serves the MCP surface over stdio as a lifecycle-aware proxy:
    /// requests are forwarded to the running app's `/mcp` on loopback,
    /// and if the app is down it is launched on first access. Local
    /// tools `app_status` / `app_restart` manage the serving process
    /// from the side that survives its death. Starts fine with no
    /// backend — the connection is made lazily per access.
    Mcp {
        /// Loopback port the app serves on (default: the active
        /// profile's port — dogfood 8989, dev 18989, bench 28989; set
        /// `$ASTERISM_PROFILE` accordingly in the client registration).
        #[arg(long)]
        port: Option<u16>,
        /// App to launch when the backend is down: a `.app` bundle
        /// (opened via LaunchServices) or a plain binary (spawned
        /// directly). Falls back to `$ASTERISM_APP`, then to
        /// `open -a Asterism`.
        #[arg(long)]
        app: Option<PathBuf>,
    },
    /// Prints the canonical wire-shape schemas an external backend
    /// author is expected to consume. Mirrors the harvest importer's
    /// `--print-schema` flag but covers every SDK-owned schema plus
    /// each registered exporter's params shape. Read-only — never
    /// opens the database.
    Schema {
        #[command(subcommand)]
        action: SchemaAction,
    },
}

#[derive(Subcommand)]
enum SchemaAction {
    /// Enumerates every published schema name. Grouped as
    /// SDK-owned first, then `exporter:<slug>:params` entries.
    List,
    /// Streams one schema example JSON to stdout.
    Print {
        /// Schema name (see `schema list`).
        name: String,
    },
}

/// Every schema `schema list` / `schema print` can serve.
///
/// Statically composed at compile time from the SDK's
/// [`asterism_dispatch_sdk::SDK_SCHEMAS`] plus the four exporters
/// currently in the workspace. Phase 2 will replace the exporter
/// entries with a runtime walk over the `ExporterRegistry` so new
/// adapters register themselves; Phase 1 keeps the composition
/// static because the registry currently exposes no schema hook.
fn all_schemas() -> Vec<asterism_dispatch_sdk::SdkSchemaEntry> {
    let mut out: Vec<asterism_dispatch_sdk::SdkSchemaEntry> =
        asterism_dispatch_sdk::SDK_SCHEMAS.to_vec();
    out.push((
        asterism_exporter_file::SCHEMA_NAME,
        asterism_exporter_file::params_example_json,
    ));
    out.push((
        asterism_exporter_comfy::SCHEMA_NAME,
        asterism_exporter_comfy::params_example_json,
    ));
    out.push((
        asterism_exporter_http::SCHEMA_NAME,
        asterism_exporter_http::params_example_json,
    ));
    out.push((
        asterism_exporter_cloud::SCHEMA_NAME,
        asterism_exporter_cloud::params_example_json,
    ));
    out
}

fn resolve_db_path(db: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    match db {
        Some(path) => Ok(path),
        None => state::default_db_path(),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Before anything that can log. Records emitted until the database
    // opens are queued and flushed by `init_core`; `RUST_LOG` controls
    // what reaches stderr in the meantime.
    asterism_infra::observe::install();
    match Cli::parse().command {
        Command::Serve { db, port } => {
            let db_path = resolve_db_path(db)?;
            let ctx = state::init(&db_path).await?;
            let port = port.unwrap_or(asterism_infra::paths::active_profile()?.default_http_port());
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
            let listener = tokio::net::TcpListener::bind(addr).await?;
            eprintln!(
                "asterism-server: http://{addr}/asterism/health (db: {})",
                db_path.display()
            );
            axum::serve(listener, http::router(ctx)).await?;
            Ok(())
        }
        Command::Init { db } | Command::Migrate { db } => {
            let db_path = resolve_db_path(db)?;
            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let (isle, driver) = asterism_infra::sqlite::open_and_migrate(&db_path).await?;
            let version = asterism_infra::sqlite::schema_version(&isle).await?;
            println!(
                "asterism-server: db ready at {} (schema v{version})",
                db_path.display()
            );
            driver.shutdown().await.ok();
            Ok(())
        }
        Command::Status { db } => {
            let db_path = resolve_db_path(db)?;
            if !db_path.exists() {
                anyhow::bail!(
                    "db does not exist at {} — run `asterism-server init` first",
                    db_path.display()
                );
            }
            let (isle, driver) = asterism_infra::sqlite::open_expecting_latest(&db_path)
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "db at {} is not on the latest schema — run `asterism-server migrate`",
                        db_path.display()
                    )
                })?;
            let version = asterism_infra::sqlite::schema_version(&isle).await?;
            let personas: i64 = isle
                .call(|conn| conn.query_row("SELECT count(*) FROM persona", [], |row| row.get(0)))
                .await?;
            let assets: i64 = isle
                .call(|conn| conn.query_row("SELECT count(*) FROM asset", [], |row| row.get(0)))
                .await?;
            // Reported separately rather than folded into `assets`: this
            // is a diagnostic surface, so "how much is recoverable from
            // the trash" is exactly the kind of thing an operator wants
            // to see, and hiding it inside the total would make the
            // figure disagree with the grid for no stated reason.
            let trashed: i64 = isle
                .call(|conn| {
                    conn.query_row(
                        "SELECT count(*) FROM asset WHERE trashed_at IS NOT NULL",
                        [],
                        |row| row.get(0),
                    )
                })
                .await?;
            println!("db_path:        {}", db_path.display());
            println!("schema_version: v{version} (latest)");
            println!("personas:       {personas}");
            println!("assets:         {assets} (trashed: {trashed})");
            driver.shutdown().await.ok();
            Ok(())
        }
        Command::Mcp { port, app } => {
            let port = port.unwrap_or(asterism_infra::paths::active_profile()?.default_http_port());
            let app = app.or_else(|| std::env::var_os("ASTERISM_APP").map(PathBuf::from));
            let launch = match app {
                Some(path) => asterism_server::mcp_proxy::AppLaunch::Path(path),
                None => asterism_server::mcp_proxy::AppLaunch::Default,
            };
            // stdout is the MCP wire; anything human-facing goes to
            // stderr (tracing already does — see `observe::install`).
            eprintln!(
                "asterism-server: mcp stdio proxy → http://127.0.0.1:{port}/mcp (lazy connect)"
            );
            let service = rmcp::serve_server(
                asterism_server::mcp_proxy::McpProxy::new(port, launch),
                rmcp::transport::io::stdio(),
            )
            .await?;
            service.waiting().await?;
            Ok(())
        }
        Command::Schema { action } => match action {
            SchemaAction::List => {
                let schemas = all_schemas();
                println!("Available schemas:");
                for (name, _) in &schemas {
                    println!("  {name}");
                }
                Ok(())
            }
            SchemaAction::Print { name } => {
                let schemas = all_schemas();
                match schemas.iter().find(|(n, _)| *n == name.as_str()) {
                    Some((_, get)) => {
                        // Trailing newline strip so `| jq` and other
                        // pipe consumers see a clean single JSON doc.
                        print!("{}", get());
                        Ok(())
                    }
                    None => {
                        anyhow::bail!(
                            "unknown schema {:?} — run `asterism-server schema list`",
                            name
                        )
                    }
                }
            }
        },
    }
}
