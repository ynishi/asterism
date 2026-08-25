# Asterism

**A local-first grid UI for personal footprints.** Keep your assets — dialogues,
work artefacts, tick logs, handoffs, memories, mood notes, dreams — on your own
disk, and let a single hover surface everything that happened around the same
moment as a small constellation. The name is a literal reference to the
interaction: an _asterism_ is a small pattern of stars, exactly what a
hover-burst looks like.

## Status

- **v0.0.0** — the initial cut is in place. Domain, application, SQLite schema,
  job pipeline (`cover_gen` / `auto_tag` / `edge_rebuild`), the HTTP API server,
  and the Tauri grid UI with hover-burst rendering all run end-to-end.
- Nothing is published to crates.io (every crate has `publish = false`).
- Data is isolated by local profile: release builds default to
  `~/.asterism/profiles/dogfood/`, debug builds to `~/.asterism/profiles/dev/`,
  and stress runs may select `bench` with `$ASTERISM_PROFILE` (`$ASTERISM_HOME`
  moves a named profile's home, and is not a profile selector on its own).

## Local data profiles

Asterism keeps development, daily Dogfooding, and large performance fixtures
physically separate:

| Profile   | Default home                    | Default HTTP port | Durability       |
| --------- | ------------------------------- | ----------------: | ---------------- |
| `dev`     | `~/.asterism/profiles/dev/`     |             18989 | disposable       |
| `dogfood` | `~/.asterism/profiles/dogfood/` |              8989 | durable; back up |
| `bench`   | `~/.asterism/profiles/bench/`   |             28989 | reproducible     |

Debug builds default to `dev`; release/bundled builds default to `dogfood`.
`ASTERISM_PROFILE` selects a profile and may be used alone; `ASTERISM_HOME`
moves that profile's home somewhere other than the default path, and an explicit
home with no profile named is refused rather than guessed at.

A named home contains a `.asterism-profile` marker and Asterism refuses to open
it under a different named profile. `ASTERISM_PROFILE=custom` opens an explicit
home without that guard, for scratch work — it requires `ASTERISM_HOME`, since a
custom home is the thing being named, and it shares dogfood's default port, so
pass `--port` when a dogfood instance may also be running. It is spelled out
because opting out of the guard should be something you say rather than
something you get by leaving a variable unset.

Deleting is two steps: **trash** hides an item and keeps everything about it
(rating, comments, group filing, body text), and **purge** is the irreversible
half — reachable only for something already in the trash. A retention sweep
purges what has aged out; `ASTERISM_TRASH_RETENTION_DAYS` sets that window and
defaults to 14. A malformed or non-positive value is refused at startup rather
than silently replaced, because this number decides when data stops being
recoverable.

## Signing exported disclosures

Every export carries the IPTC/XMP disclosure, which needs no key material. The
C2PA manifest beside it is signed, and this repository ships no certificate — so
out of the box the manifest half is reported as skipped, which is a supported
state rather than a degraded one. An untrusted manifest makes a provenance claim
a validator rejects, and that is worse than making none.

A deployment supplies its own certificate through the environment:

| Variable                             | Meaning                                                                                                             |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------- |
| `ASTERISM_DISCLOSURE_CERT_CHAIN`     | Path to the PEM certificate chain, end-entity first                                                                 |
| `ASTERISM_DISCLOSURE_PRIVATE_KEY`    | Path to the PEM private key                                                                                         |
| `ASTERISM_DISCLOSURE_KEYCHAIN_KEY`   | macOS: the label of a private key in the Keychain, instead of a key file                                            |
| `ASTERISM_DISCLOSURE_SIGNING_ALG`    | COSE algorithm name; defaults to `es256`                                                                            |
| `ASTERISM_DISCLOSURE_TSA_URL`        | Timestamp authority (`http://` or `https://`); without one, a manifest stops verifying when the certificate expires |
| `ASTERISM_DISCLOSURE_SIGNING_STRICT` | `true` also refuses a certificate a trust list would not carry, and requires the issuer's chain                     |

Not settings-screen keys, deliberately. These are the operator's arrangement
with an issuer rather than a user preference, and a settings key is both
writable through `PUT /asterism/settings/{key}` and readable back with the value
of every layer it has — which would publish the location of the private key and
let that route choose which file this process opens. Keep the key readable by
its owner alone (`0600`); Asterism warns at startup when it is not.

The certificate variable is needed with exactly one of the two key variables. On
macOS the Keychain form is the stronger custody: the key — including one held in
the Secure Enclave, which the same label search finds — never enters the
process, signing goes through the Security framework, and there is no key file
whose permissions anyone has to audit. It signs ECDSA (`es256`, `es384`,
`es512`); an RSA or Ed25519 arrangement uses the file form. A certificate that
cannot be loaded does not stop the application — one issued under the C2PA
conformance profile is valid for at most 366 days, so every signing deployment
eventually meets an expired one — but it does not pass quietly either: the
reason is logged at startup and recorded against every export as a failed
manifest half, which is a different statement from the skip that means no
certificate is configured.

Nothing here makes a manifest _trusted_. That needs a certificate authority on
the C2PA trust list, and a self-issued one validates as
`signingCredential.untrusted` while C2PA's own validation state stays `Valid`.

From the repository root, use `just dev` for the isolated Dev app,
`just dogfood` to build and launch the production-shaped Dogfood app, and
`just bench` for the large fixture. Run `just --list` for profile init,
headless, and check recipes. The Dev flavor has a distinct application
identifier, window title, and in-app badge so it can coexist with the bundled
app.

All built-in import adapters share one CLI. Run `just import --help` for the
source list, then select a subcommand such as `just import tape --help`,
`just import journal --help`, or `just import image --help`. CLI and environment
resolution stay in this outer command; scanner/parser libraries receive resolved
values through the shared importer runner.

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

- **Domain** (`asterism-core`) — separate `Persona` and `Asset` aggregates,
  open-slug `Modality`, an explicit `Visibility` model, and a
  `ConstellationEdge` planner (`plan_edges`, pure). Design lives in the rustdoc.
- **SQLite** (`asterism-infra`) — `rusqlite-isle` (0.3 line, matching the
  `libsqlite3-sys 0.30` cluster used by `apalis-sql`), append-only migrations
  gated by `PRAGMA user_version`, `STRICT` tables, UUID primary keys stored as
  BLOBs, timestamps as unix epoch milliseconds.
- **Job pipeline** (apalis + `apalis-sql`) — asset ingest fans out to
  `cover_gen` (modality-specific heuristic), `auto_tag` (keywords → channel
  tags), and finally `edge_rebuild` (windowed incremental) once the keywords are
  committed.
- **API** (`asterism-server`) — `asterism-server serve` binds
  `http://127.0.0.1:8989/asterism/*`. Route conventions mirror the Tauri command
  surface. The same router serves MCP (streamable-http) at `/mcp` — a curated
  tool set (search / list / get / add / lineage / comments / catalog / dispatch)
  over the same application services, with input schemas generated from
  `asterism-contract`. `asterism-server mcp` serves the identical tools over
  stdio.
- **UI** (`asterism-ui`) — Svelte 5 on top of Tauri v2: persona sidebar,
  modality tabs, a dense grid, and a hover-burst side panel. TypeScript bindings
  are regenerated from `asterism-contract` at build time via `schema-bridge`.

## Development environment

`just check` is the gate. Beyond a stable Rust toolchain and Node for the UI,
three things are installed per machine rather than per checkout:

| Needed by                                                                                                               | Install                                                                                                                   | Without it                                                                                                    |
| ----------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `aidoc-guard` (inside `just check`)                                                                                     | `cargo install cargo-aidoc` (0.2.2 or newer), then `rustup toolchain install "$(cargo aidoc --print-required-toolchain)"` | the step prints `WARNING: docs/aidoc/ NOT CHECKED` and the gate continues                                     |
| `prose-shape`, recommended — see [CONTRIBUTING.md](CONTRIBUTING.md#working-with-coding-agents--the-recommended-pattern) | in Claude Code: `/plugin marketplace add ynishi/asterism`, then `/plugin install prose-shape@asterism`                    | nothing catches a pull request body hard-wrapped into a renderer that folds paragraphs itself                 |
| `doc-review`, recommended — same section                                                                                | same marketplace: `/plugin install doc-review@asterism`                                                                   | nothing reads a comment for whether it is still true; the change reviewer says so rather than covering for it |

The toolchain is a dated nightly rather than the channel, and the tool names it
rather than this file: rustdoc's JSON carries a `format_version`, every nightly
emits exactly one, and it moves whenever rustdoc's types do — so `cargo-aidoc`
pins the one it can read and `--print-required-toolchain` prints it. A date
copied into a script here would go stale the next time the tool is upgraded.

An older `cargo-aidoc` is worse than none: it is on `PATH`, so the guard runs
it, and it fails on an argument it does not have rather than on anything about
this repository.

`docs/aidoc/` is the committed module inventory — generated, because the
hand-written one shipped 15 modules stale. Regenerate it with `just aidoc` after
changing any public API or doc comment, and commit the diff; `aidoc-guard` fails
on drift.

The warning exists because the alternative is worse than the inconvenience. The
step needs a nightly toolchain, this workspace pins none, and a machine without
one must still be able to run the full gate — but a gate that is silently
skipped is not a gate. A change that deleted a crate once left the committed
docs describing it while `just check` went green.

## Public development

Asterism keeps public product and implementation work self-contained while
allowing plain internal task identifiers and links as supplemental provenance.
The disclosure boundary and the `ALLOW / WARN / BLOCK` publication check are
defined in [PUBLIC_DEVELOPMENT.md](PUBLIC_DEVELOPMENT.md).

## Licence

Licensing is declared **per crate** (the `license` field in each crate's
`Cargo.toml`). The local-first core is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option, and it stays that way.

The crates implementing the hosted teams plane — `teams-core`, `teams-infra`,
`teams-contract`, `teams-server` — are licensed under the GNU Affero General
Public License, version 3 or later ([LICENSE-AGPL](LICENSE-AGPL)). The boundary
between the two regimes sits at the binary's edge: `teams-server` owns its own
binary, the direction the licence boundary guards — an `asterism-*` crate
depending on a `teams-*` crate — stays empty, and the two sides share only what
the MIT/Apache side declares.

One crate sits on neither side. `asterism-teams-wire` carries the wire
vocabulary a member's client and a team server both speak, depends on neither
plane, and is MIT/Apache-2.0 — which is what lets the local-first core link it
at all, and what makes the server implementable by something that is not this
codebase. Its manifest says so at the field.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions. A
contribution to the four `teams-*` crates is licensed under AGPL-3.0-or-later,
the licence those crates carry.

## Author

Yutaka Nishimura <ytk.nishimura@gmail.com>
