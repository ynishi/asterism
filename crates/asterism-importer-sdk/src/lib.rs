//! # asterism-importer-sdk
//!
//! Building blocks for Asterism importers.
//!
//! The unified `asterism-import` binary walks an external source,
//! parses it into typed [`Footprint`]s, and pushes them to a running
//! `asterism-server` through the HTTP API. This crate provides the
//! reusable pipeline — subcommands only need to plug in a
//! [`SourceScanner`] (usually one of the ones bundled here) and a
//! source-specific [`SourceParser`].
//!
//! ## Pipeline
//!
//! ```text
//! Scanner  ─→ RawItem  ─→ Parser  ─→ Footprint  ─→ AssetSpec  ─→ AddAssetCommand
//!                                                                       │
//!                                                                       ▼
//!                                                     HTTP POST /asterism/assets/add[-batch]
//! ```
//!
//! - [`SourceScanner`] enumerates or watches an external source and
//!   emits [`RawItem`]s. Implementations bundled here: [`FsScanner`],
//!   [`SqliteScanner`].
//! - [`SourceParser`] turns a `RawItem` into zero or more
//!   [`Footprint`]s; it is the only source-specific piece an importer
//!   author has to write.
//! - [`Footprint`] is a typed enum with one variant per well-known
//!   modality (`ChatMessage`, `Doc`, `Note`, `Image`); the compiler
//!   guides the plugin author to fill in the right fields.
//! - [`AssetSpec`] is the flat intermediate the SDK converts each
//!   footprint to before batching; plugin authors do not touch it
//!   directly (see [`Footprint::into_asset_spec`]).
//! - [`ApiClient`] performs the HTTP POSTs (single or batch).
//! - [`Progress`] keeps a running success / failure tally.
//! - [`run_import`] owns the shared scan / parse / batch / progress
//!   loop after the outer CLI has resolved arguments and environment.
//!   It also fills in [`AssetSpec::declared_content_hash`] for the
//!   records where it is a true statement — the scanner read a whole
//!   artefact ([`SourceScanner::payload_is_whole_artefact`]) and the
//!   spec still carries that artefact's own address — which lets the
//!   server propose an exact-copy duplicate without opening the file.
//!
//! ## Writing a new importer
//!
//! Four conventions every importer author needs to know. Each has its
//! canonical rule co-located with the API item that owns it; this
//! section is a jump table.
//!
//! 1. **`source_kind` ownership** — the *importer* decides the slug,
//!    not the scanner. Scanners provide a sensible default (`"fs"`,
//!    `"sqlite"`) so a one-off tool works out of the box, but a
//!    published importer overrides it via
//!    [`FsScanner::with_source_kind`] (or the equivalent on other
//!    scanners) to a slug that names the importer's source
//!    (`"cc"`, `"persona-journal"`, `"apple-notes"`). The slug flows
//!    through [`RawItem::source_kind`] into
//!    [`FootprintSource::kind`] and eventually into the DB unique
//!    index `(source_kind, source_locator)`, so it must be stable
//!    across releases of the same importer. See
//!    [`scanner::RawItem::source_kind`] for the full rule.
//!
//! 2. **`occurred_at` fallback ladder** — parsers pick the timestamp
//!    with the highest fidelity available: (a) a timestamp *inside*
//!    the payload (message header, DB column), (b)
//!    [`RawItem::occurred_at`] (scanner-derived: file `mtime`, row
//!    column), (c) `Utc::now()` as a last resort. Never invert the
//!    order — an `mtime` for a JSONL session log is the *file's* last
//!    write, not the individual message's. See [`parser::SourceParser`]
//!    for the canonical form.
//!
//! 3. **Idempotent `locator` under `Watch` mode** — [`ScanMode::Watch`]
//!    re-emits whole files as they change (append-heavy sources like
//!    `.jsonl` session logs are the common case). The parser is
//!    responsible for producing a **record-level** locator so
//!    unchanged records collapse via the server-side unique index and
//!    only new records land. The `<file-path>#<record-uuid>` pattern
//!    is the canonical shape; see
//!    [`footprint::FootprintSource::locator`] for the full patterns
//!    and constraints.
//!
//! 4. **Adding a new modality** — new modalities are added as
//!    new [`Footprint`] variants, not through a stringly-typed
//!    escape hatch. Every modality gets a typed struct so the
//!    compiler shows plugin authors the fields it needs, and the
//!    [`Footprint::into_asset_spec`] arm centralises truncation /
//!    label / modality-slug rules. `JournalKind::Other` /
//!    `ChatRole::Other` / `DocFormat::Other` are per-variant escape
//!    hatches for sub-kind slugs, not for whole new modalities.
//!
//! ## Import target catalogue
//!
//! Field-mapping reference for schemas Asterism plans to import
//! (Character Card V2/V3, PNG tEXt embed, CharacterHub, RisuAI,
//! AgnAI, KoboldAI, SillyTavern chat JSONL / World Info,
//! NovelAI Lorebook, ChatGPT export, Claude data export, Letta,
//! MemoryPlugin, SillyTavern backup zip). See [`catalogue`] for
//! per-target split rules, locator patterns, and unverified fields.

pub mod bundle;
pub mod card;
pub mod catalogue;
pub mod client;
pub mod footprint;
pub mod harvest;
pub mod mapper;
pub mod parser;
pub mod progress;
pub mod runner;
pub mod scanner;

/// The digest notation, re-exported on the same terms: an importer that
/// builds its own [`AssetSpec`] outside [`run_import`] can spell a
/// [`AssetSpec::declared_content_hash`] with
/// [`digest::of_bytes`](asterism_contract::digest::of_bytes) without
/// reaching past this crate.
///
/// Only the notation is here. What a digest *means* — whether two of
/// them are a duplicate, which axis a value belongs to, what the
/// markers stand for — is the server's, and an importer that could
/// spell those rules would be a second place they are decided.
pub use asterism_contract::digest;
/// Sidecar vocabulary, re-exported so a parser can look for
/// `<locator>.meta.json` without taking a direct dependency on the
/// contract crate (importers depend on this SDK and nothing else of
/// Asterism's).
pub use asterism_contract::sidecar::{SIDECAR_IDENTITY_KEY, SIDECAR_SCHEMA, SIDECAR_SUFFIX};
pub use client::ApiClient;
pub use footprint::{
    Audio, COVER_MAX_CHARS, ChatMessage, ChatRole, Doc, DocFormat, Footprint, FootprintSource,
    Image, JournalEntry, JournalKind, Note, REGISTER_MAX_CHARS, Tape, Video,
};
pub use mapper::{AssetSpec, spec_to_command};
pub use parser::{ParseError, RecordAddresses, SourceParser};
pub use progress::Progress;
pub use runner::{ImportOptions, ImportSummary, run_import};
pub use scanner::{
    RawItem, ScanError, ScanMode, SourceScanner, fs::FsScanner, sqlite::SqliteScanner,
};
