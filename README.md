# Asterism

**A local-first grid UI for personal footprints.** Keep your assets —
dialogues, work artefacts, tick logs, handoffs, memories, mood notes,
dreams — on your own disk, and let a single hover surface everything that
happened around the same moment as a small constellation. The name is a
literal reference to the interaction: an *asterism* is a small pattern of
stars, exactly what a hover-burst looks like.

## Status

- **v0.0.0** — the initial cut is in place. Domain, application, SQLite
  schema, job pipeline (`cover_gen` / `auto_tag` / `edge_rebuild`), the
  HTTP API server, and the Tauri grid UI with hover-burst rendering all
  run end-to-end.
- Nothing is published to crates.io (every crate has `publish = false`).
- Data is isolated by local profile: release builds default to
  `~/.asterism/profiles/dogfood/`, debug builds to
  `~/.asterism/profiles/dev/`, and stress runs may select `bench` with
  `$ASTERISM_PROFILE` (`$ASTERISM_HOME` remains the explicit override).

## Local data profiles

Asterism keeps development, daily Dogfooding, and large performance
fixtures physically separate:

| Profile | Default home | Default HTTP port | Durability |
|---|---|---:|---|
| `dev` | `~/.asterism/profiles/dev/` | 18989 | disposable |
| `dogfood` | `~/.asterism/profiles/dogfood/` | 8989 | durable; back up |
| `bench` | `~/.asterism/profiles/bench/` | 28989 | reproducible |

Debug builds default to `dev`; release/bundled builds default to
`dogfood`. Set `ASTERISM_PROFILE` to select a named profile or
`ASTERISM_HOME` for an explicit scratch location. A named home contains
a `.asterism-profile` marker and Asterism refuses to open it under a
different named profile.

Deleting is two steps: **trash** hides an item and keeps everything about
it (rating, comments, group filing, body text), and **purge** is the
irreversible half — reachable only for something already in the trash.
A retention sweep purges what has aged out; `ASTERISM_TRASH_RETENTION_DAYS`
sets that window and defaults to 14. A malformed or non-positive value is
refused at startup rather than silently replaced, because this number
decides when data stops being recoverable.

From the repository root, use `just dev` for the isolated Dev app,
`just dogfood` to build and launch the production-shaped Dogfood app, and
`just bench` for the large fixture. Run `just --list` for profile init,
headless, and check recipes. The Dev flavor has a distinct application
identifier, window title, and in-app badge so it can coexist with the
bundled app.

All built-in import adapters share one CLI. Run `just import --help` for
the source list, then select a subcommand such as `just import tape --help`,
`just import journal --help`, or `just import image --help`. CLI and
environment resolution stay in this outer command; scanner/parser libraries
receive resolved values through the shared importer runner.

## Crate layout

```
asterism/
├── Cargo.toml                     # workspace
├── crates/
│   ├── asterism-core/             # Domain + Application (hexagonal core)
│   ├── asterism-infra/            # rusqlite-isle + apalis + FS adapters
│   ├── asterism-contract/         # Command / Query / DTO types (leaf)
│   ├── asterism-server/           # local HTTP API + MCP transport
│   ├── asterism-importer-sdk/     # importer pipeline (Scanner + Parser plug-ins)
│   ├── asterism-importer-*/       # per-source importers (cc / tape / journal / image / …)
│   ├── asterism-dispatch-sdk/     # exporter (outbound) SDK
│   ├── asterism-exporter-*/       # per-backend exporters (comfy / file / http)
│   └── asterism-ui/               # Tauri v2 app (frontend + backend host)
├── workspace/                     # local scratch (gitignored)
└── README.md
```

Every crate has `publish = false`; nothing is distributed via crates.io.

## Architecture

- **Domain** (`asterism-core`) — separate `Persona` and `Asset`
  aggregates, open-slug `Modality`, an explicit `Visibility` model, and a
  `ConstellationEdge` planner (`plan_edges`, pure). Design lives in the
  rustdoc.
- **SQLite** (`asterism-infra`) — `rusqlite-isle` (0.3 line, matching the
  `libsqlite3-sys 0.30` cluster used by `apalis-sql`), append-only
  migrations gated by `PRAGMA user_version`, `STRICT` tables, UUID
  primary keys stored as BLOBs, timestamps as unix epoch milliseconds.
- **Job pipeline** (apalis + `apalis-sql`) — asset ingest fans out to
  `cover_gen` (modality-specific heuristic), `auto_tag` (keywords →
  channel tags), and finally `edge_rebuild` (windowed incremental) once
  the keywords are committed.
- **API** (`asterism-server`) — `asterism-server serve` binds
  `http://127.0.0.1:8989/asterism/*`. Route conventions mirror the Tauri
  command surface. The same router serves MCP (streamable-http) at
  `/mcp` — a curated tool set (search / list / get / add / lineage /
  comments / catalog / dispatch) over the same application services,
  with input schemas generated from `asterism-contract`. `asterism-server
  mcp` serves the identical tools over stdio.
- **UI** (`asterism-ui`) — Svelte 5 on top of Tauri v2: persona sidebar,
  modality tabs, a dense grid, and a hover-burst side panel. TypeScript
  bindings are regenerated from `asterism-contract` at build time via
  `schema-bridge`.

## Development environment

`just check` is the gate. Beyond a stable Rust toolchain and Node for the
UI, one step has prerequisites it cannot install for you:

| Needed by | Install | Without it |
|---|---|---|
| `aidoc-guard` (inside `just check`) | `cargo install cargo-aidoc` (0.2.1 or newer) and `rustup toolchain install nightly` | the step prints `WARNING: docs/aidoc/ NOT CHECKED` and the gate continues |

An older `cargo-aidoc` is worse than none: it is on `PATH`, so the guard
runs it, and it fails on an argument it does not have rather than on
anything about this repository.

`docs/aidoc/` is the committed module inventory — generated, because the
hand-written one shipped 15 modules stale. Regenerate it with `just
aidoc` after changing any public API or doc comment, and commit the
diff; `aidoc-guard` fails on drift.

The warning exists because the alternative is worse than the
inconvenience. The step needs a nightly toolchain, this workspace pins
none, and a machine without one must still be able to run the full
gate — but a gate that is silently skipped is not a gate. A change that
deleted a crate once left the committed docs describing it while `just
check` went green.

## Public development

Asterism keeps public product and implementation work self-contained while
allowing plain internal task identifiers and links as supplemental provenance.
The disclosure boundary and the `ALLOW / WARN / BLOCK` publication check are
defined in [PUBLIC_DEVELOPMENT.md](PUBLIC_DEVELOPMENT.md).

## Licence

Licensing is declared **per crate** (the `license` field in each crate's
`Cargo.toml`). Every crate currently in this repository — the local-first
core — is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Future crates implementing a hosted collaboration plane (shared spaces,
sync, team features) may be released under a different license; the
local-first core stays MIT/Apache-2.0.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms
or conditions.

## Author

Yutaka Nishimura <ytk.nishimura@gmail.com>
