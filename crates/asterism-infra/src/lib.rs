//! # asterism-infra — outbound adapters (SQLite, filesystem, job engine)
//!
//! ## Architecture
//!
//! Concrete implementations of the ports declared in `asterism-core`
//! (repository traits, `JobQueue`, `ProgressEmitter`, and so on). The
//! dependency direction is one-way — `asterism-core` never depends on
//! this crate.
//!
//! - **SQLite backend** — built on `rusqlite-isle`, which confines the
//!   `Connection` to a dedicated thread and gives us `sqlite3_interrupt`-
//!   based cancellation, a WAL reader pool, and panic containment. The
//!   `SqlitePersonaRepository`, `SqliteAssetRepository`, `SqliteTagRepository`,
//!   and `SqliteEdgeRepository` all live here.
//! - **Filesystem / media pipeline** (planned) — originals stay on the
//!   filesystem and are served through Tauri's asset protocol, while
//!   thumbnails will be cached as SQLite BLOBs. `walkdir`, `notify`,
//!   `image`, `rayon`, and `kamadak-exif` are wired for future use.
//! - **Job engine** — apalis + apalis-sql (SQLite backend) coexists with
//!   `rusqlite-isle` on the same SQLite file. The engine owns the job
//!   lifecycle; adapters here expose it to the domain through the
//!   `JobQueue` port.
//! - **Artefact probes** — one implementation of
//!   `asterism_core::domain::probe::ArtefactProbe` per container format,
//!   plus the registry `fingerprint` asks. Each holds a format's
//!   judgement about its own bytes (which of them are the picture, which
//!   are notes about it); the byte-level parsing underneath is
//!   `asterism-media-probe`, which has no such judgement.
//!
//! ## Design intent
//!
//! - **Ports live in `asterism-core`.** This crate defines adapters only,
//!   never traits.
//! - **`libsqlite3-sys` cluster alignment.** `rusqlite-isle` publishes
//!   separate release lines per `libsqlite3-sys` cluster; we pin to the
//!   line that lines up with `apalis-sql` (see the workspace `Cargo.toml`).

#![warn(missing_docs)]

pub mod disclosure;
pub mod dispatch;
pub(crate) mod fingerprint;
pub mod forge;
pub mod generator_params;
pub mod jobs;
pub mod memory;
pub mod observe;
pub mod paths;
pub mod probes;
pub mod search;
pub mod source_text;
pub mod sqlite;
pub mod telemetry;
