# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **The series key no longer borrows its canonical form from a
  dependency** (#14) — `series::render` hashes a `serde_json::Value`
  parsed out of a container, and was taking its nested key order from
  whichever map type `serde_json`'s feature flags selected. A new
  `series::canonical_value` sorts every object's keys recursively before
  the bytes are rendered, so the digest is a function of the document
  rather than of how it was typed: a JSON object is an unordered
  collection (RFC 8259), and two containers carrying the same fields in a
  different order carry the same document. Arrays keep their order, since
  a JSON array *is* ordered. Byte output is unchanged and no stored key
  moves.

  `serde_json`'s `preserve_order` is now declared in the workspace
  `Cargo.toml` with its reasoning rather than arriving as a side effect of
  `c2pa` (which requires it). The old test that asserted sorted output and
  warned in prose that this rested on a default has a sibling asserting
  the property itself, plus the negative case; both fail by name if the
  sort is removed, which is the point — the function reads like a no-op
  and will invite deletion.

### Added

- **AI-disclosure provenance: the vocabulary, the emitters and the signer**
  (#14) — a new `asterism-provenance` crate holds what an exported file
  says about where it came from, as values: the IPTC digital source type
  (five terms, closed, refusing anything the vocabulary does not define),
  the XMP packet carrying `Iptc4xmpExt:DigitalSourceType` and the four AI
  properties IPTC added in Photo Metadata Standard 2025.1, that packet
  written into a PNG `iTXt` chunk or a JPEG `APP1` segment as a byte
  transform, and the C2PA manifest definition built from the same record
  so the two cannot disagree. `asterism-infra::provenance` is the adapter
  that puts them into a file and signs the manifest through `c2pa`,
  covering MP4 and MOV as well as stills — signing after the encode,
  which is the only point at which it is possible.

  Two decisions are worth stating. **XMP is written before the manifest is
  signed**: the hard binding covers the packet, so the reverse order
  invalidates the signature, and a test signs a file, edits its packet and
  asserts the binding then fails. **A signing identity is configuration**:
  the IPTC/XMP disclosure is written with or without one, a manifest only
  with, and the C2PA test certificates are refused by name rather than
  used as a fallback — a manifest signed by them validates as untrusted,
  which claims a provenance a reader rejects.

  Not yet wired to the export path, and no re-apply verb; both are the
  rest of #14. Unsigned video carries no disclosure at all, because the
  XMP half has no BMFF spelling here, and the writer reports that rather
  than a success it did not have.

- **Material layers, and the chapters an import brings in** (#1) — a
  material now carries layers: an origin (`imported` / `user` / `machine`),
  a role (`structure` / `annotation`), a default flag and an order. Chapters
  declared by a container are read by a `ChapterScan` job (the bundled
  ffmpeg's `ffmetadata` output, one parser for every format instead of one
  per container) into an imported structure layer, which re-probing replaces
  wholesale. A user keeps their own chapter set in a separate layer beside
  the file's and switches between them; editing one never alters the other,
  and the server refuses writes into an imported layer. Existing time-based
  comments become the asset's annotation layer via a total backfill
  (migration V78). The UI's untyped `extra.chapters` reader — dead code
  whose producer never existed — is deleted in favour of a typed chapter
  panel on both the video and audio branches, and an empty imported layer
  ("the file declares no chapters") renders distinctly from no layer at all
  ("never scanned"). MCP gains a read-only `material_layers` tool.

- **CI** (`.github/workflows/check.yml`) — `just check` runs on every pull
  request and on push to `main`, so whether the gates pass is something the
  repository states rather than a claim about whoever last ran the recipe.
  The workflow invokes the recipe instead of restating its six gates, so the
  local gate and CI cannot drift apart. One macOS job for now, which is the
  simple and expensive answer; splitting the portable crates onto Linux is a
  decision left to a measurement. `ui-e2e` (needs a real window) and
  `collation-jsc` (needs macOS's `jsc`) stay out, and the workflow says so
  rather than leaving it to be inferred.

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

- **A refused operation says so on screen** (`asterism-ui`) — asking
  Asterism to do something it then refused could leave no trace: the
  failure went to the browser console and the interface carried on,
  including for operations that move or destroy data. The read path had
  no equivalent gap (`Resource` exposes load failures); the write path
  had no owner for them at all. A new `lib/mutate.ts` wraps the write
  calls, puts the refusal and the backend's reason in a sticky toast
  beside the Undo one, and re-throws so that existing rollbacks are
  unaffected. Routed through it: the grid, group and trash paths —
  `trash_asset` (including the duplicate panel's bulk trash),
  `purge_asset`, `restore_asset`, `empty_trash`, `trash_group`,
  `delete_dir`, `delete_asset_comment`, `add_asset_to_group` and
  `remove_asset_from_group`, `unlink_group`. **Not yet routed**, and
  still console-only: tag detach, persona themes, material marks,
  threads, modalities, sessions and setting resets — along with the
  non-destructive half of the write path (metadata edits, reordering,
  the create and rename family). Bulk loops that could partly fail
  now report what actually happened ("moved 3 of 5 to trash — the rest
  was refused") instead of counting a refusal as a success. The path
  is exercised end-to-end: `e2e/refusal.spec.ts` seeds its own dir
  pair over the app's loopback HTTP, provokes a real `delete_dir`
  refusal in the WebView, asserts the toast carries the backend's own
  reason, then deletes the emptied pair with the same gesture,
  asserting that success stays silent.

- **The committed TypeScript bindings are checked against the contract**
  — `asterism-ui/src/bindings.ts` is generated by `src-tauri/build.rs`
  and tracked in git, and nothing compared the two. A contract change
  whose regenerated bindings were never committed would have left a
  stale copy that every gate passed, and passed invisibly: everyone
  builds from a copy regenerated on their own machine, so only a
  consumer reading the file without compiling Rust would have met it.
  `just bindings-check` forces the build script to run, then diffs the
  result against `HEAD`; it runs inside `just check`. The forcing is not
  incidental — `tauri_build` registers `rerun-if-changed` directives,
  which means a warm tree can otherwise skip the script entirely and
  compare the committed file against itself.

- **`rust-test` no longer depends on the caller's colour setting** — the
  recipe counts cargo's `Running` / `Doc-tests` lines against the
  `test result:` lines to prove that every launched binary reported a
  result, and both patterns are anchored at the start of the line.
  Coloured output puts an escape sequence there, so the count came back
  0 launched against 81 reported and the check failed over a suite that
  was 1191 passed / 0 failed. It fixes `CARGO_TERM_COLOR=never` for
  itself now, rather than parsing a shape its caller's terminal can
  change.

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
