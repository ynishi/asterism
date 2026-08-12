# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **MCP transport** (`asterism-server`) — the third adapter over the
  same application services. A curated nine-tool vocabulary
  (`asset_search` / `asset_list` / `asset_get` / `asset_add` /
  `asset_lineage` / `asset_comments` / `asset_comment_add` /
  `catalog_overview` / `dispatch_get`) served over streamable-http at
  `/mcp` on the loopback router (present in both the Tauri-embedded
  server and the standalone binary) and over stdio via
  `asterism-server mcp` (previously a stub). Tool input schemas are
  generated from the `asterism-contract` types that already back HTTP
  and Tauri IPC (new contract feature `json-schema`); domain failures
  surface as tool-level errors carrying the HTTP boundary's
  `{kind, message}` shape.

- **Local data profiles** — `dev` / `dogfood` / `bench` homes under
  `~/.asterism/profiles/`, each with its own default HTTP port, selected
  by build flavour or `$ASTERISM_PROFILE`. A `.asterism-profile` marker
  in the home prevents opening one profile's data under another.
- **Trash and purge** — trashing is reversible and preserves rating,
  comments, group filing and body text; purge is separate, irreversible,
  and only reachable from the trash. A retention sweep purges what has
  aged past `ASTERISM_TRASH_RETENTION_DAYS`.
- **Full-text search** (`asterism-infra/search`) — a BM25 body index on
  Tantivy with Lindera Japanese morphological analysis and an English
  Porter stemmer on one tokenizer chain, persisted outside the SQLite
  transaction and reconstructed by the `index_rebuild` job after a crash.
- **Import adapters** — Claude Code session logs, tapes, persona
  journals, images, video and audio, all behind one CLI whose
  environment resolution happens in the outer command; media inspection
  is shared through `asterism-media-probe`, and video/audio bundling
  uses an LGPL-clean ffmpeg sidecar built by
  `scripts/build-ffmpeg-sidecar.sh`.
- **Export adapters** (`asterism-dispatch-sdk` + `asterism-exporter-*`)
  — outbound dispatch to ComfyUI, the filesystem, and arbitrary HTTP
  endpoints, with per-backend parameter schemas.
- **Two-sided sort contract** — the grid comparator (`Intl.Collator`)
  and its Rust port (`icu_collator`) are checked against shared
  collation fixtures, because Query Groups freeze the backend order into
  `asset_bucket.position` and the two halves must agree.
- **Benchmark corpus generator** (`asterism-benchgen`) — a seeded
  synthetic corpus (ChaCha20) where the seed, not the emitted files, is
  the identity of the corpus.
- **Domain layer** (`asterism-core/domain`) — `Persona` and `Asset`
  aggregates, an open-slug `Modality` and `SourceKind`, a `Visibility`
  model, `ConstellationEdge` with a pure `plan_edges` planner, and every
  repository port.
- **Application layer** (`asterism-core/application`) —
  `PersonaService` and `AssetService` with DTO-in / DTO-out APIs, plus
  the domain ↔ DTO mapping in one place.
- **SQLite backend** (`asterism-infra`) — `rusqlite-isle` on the 0.3
  release line (aligned with `apalis-sql`'s `libsqlite3-sys` cluster);
  append-only migrations gated by `PRAGMA user_version`; schema v1
  covering six `STRICT` tables (`persona`, `asset`, `tag`, `asset_tag`,
  `edge`, `thumb_cache`) with UUID BLOB keys and unix-epoch-ms
  timestamps.
- **Job pipeline** (apalis + `apalis-sql`) — `cover_gen` (modality-
  specific heuristic), `auto_tag` (keywords → channel tags),
  `edge_rebuild` (windowed incremental). Column-level partial updates
  avoid a read-modify-write race; `auto_tag` chain-enqueues
  `edge_rebuild` once the keywords are committed.
- **HTTP API** (`asterism-server`) — axum router bound to loopback, with
  RPC-style routes under `/asterism/*` that mirror the Tauri command
  surface. Clap CLI with `serve` and a placeholder `mcp` subcommand.
- **Contract crate** (`asterism-contract`) — Command / Query / Response
  DTOs derived with `schema-bridge`; TypeScript bindings are regenerated
  from the same source at build time and land in
  `asterism-ui/src/bindings.ts`.
- **Desktop UI** (`asterism-ui`) — Svelte 5 on Tauri v2: persona
  sidebar, modality tabs, dense grid, hover-burst side panel.
- Workspace scaffolding — `Cargo.toml` metadata, README, and this
  changelog.

### Fixed

- **The e2e suite is now type-checked** (`asterism-ui`) — the specs and
  both WebdriverIO configs sat outside every tsconfig, so `just ui-check`
  reported zero errors over ~4200 lines it never read, and the test
  runner erased their types rather than checking them. A second config
  (`tsconfig.e2e.json`, run as `check:e2e`) covers them without putting
  `describe` / `it` / `browser` in scope for application code. The seven
  diagnostics it surfaced on its first run are fixed: `await $$(…)` now
  goes through `getElements()`, and both configs take `tauri:options`
  and `browser.tauri` from the service's own `TauriCapabilities` instead
  of a local cast.

### Boundaries

- **Data layout**: user data is isolated per local profile rather than
  living at one fixed path. Release builds default to
  `~/.asterism/profiles/dogfood/`, debug builds to
  `~/.asterism/profiles/dev/`, and stress runs select `bench`;
  `$ASTERISM_PROFILE` names a profile and `$ASTERISM_HOME` overrides the
  location outright. A named home carries a `.asterism-profile` marker
  and is refused when opened under a different profile. The UI and the
  standalone server share whichever home is selected.
- **Deletion is two steps**: trash hides an item and keeps everything
  about it; purge is the irreversible half and is reachable only for
  something already trashed. The retention sweep window is
  `ASTERISM_TRASH_RETENTION_DAYS` (default 14); a malformed or
  non-positive value is refused at startup rather than silently
  replaced.

[Unreleased]: https://github.com/ynishi/asterism/commits/main
