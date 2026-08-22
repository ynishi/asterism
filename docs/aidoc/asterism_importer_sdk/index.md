# asterism-importer-sdk 0.0.0

# asterism-importer-sdk

Building blocks for Asterism importers.

The unified `asterism-import` binary walks an external source,
parses it into typed [`Footprint`]s, and pushes them to a running
`asterism-server` through the HTTP API. This crate provides the
reusable pipeline — subcommands only need to plug in a
[`SourceScanner`] (usually one of the ones bundled here) and a
source-specific [`SourceParser`].

## Pipeline

```text
Scanner  ─→ RawItem  ─→ Parser  ─→ Footprint  ─→ AssetSpec  ─→ AddAssetCommand
                                                                      │
                                                                      ▼
                                                    HTTP POST /asterism/assets/add[-batch]
```

- [`SourceScanner`] enumerates or watches an external source and
  emits [`RawItem`]s. Implementations bundled here: [`FsScanner`],
  [`SqliteScanner`].
- [`SourceParser`] turns a `RawItem` into zero or more
  [`Footprint`]s; it is the only source-specific piece an importer
  author has to write.
- [`Footprint`] is a typed enum with one variant per well-known
  modality (`ChatMessage`, `Doc`, `Note`, `Image`); the compiler
  guides the plugin author to fill in the right fields.
- [`AssetSpec`] is the flat intermediate the SDK converts each
  footprint to before batching; plugin authors do not touch it
  directly (see [`Footprint::into_asset_spec`]).
- [`ApiClient`] performs the HTTP POSTs (single or batch).
- [`Progress`] keeps a running success / failure tally.
- [`run_import`] owns the shared scan / parse / batch / progress
  loop after the outer CLI has resolved arguments and environment.
  It also fills in [`AssetSpec::declared_content_hash`] for the
  records where it is a true statement — the scanner read a whole
  artefact ([`SourceScanner::payload_is_whole_artefact`]) and the
  spec still carries that artefact's own address — which lets the
  server propose an exact-copy duplicate without opening the file.

## Writing a new importer

Four conventions every importer author needs to know. Each has its
canonical rule co-located with the API item that owns it; this
section is a jump table.

1. **`source_kind` ownership** — the *importer* decides the slug,
   not the scanner. Scanners provide a sensible default (`"fs"`,
   `"sqlite"`) so a one-off tool works out of the box, but a
   published importer overrides it via
   [`FsScanner::with_source_kind`] (or the equivalent on other
   scanners) to a slug that names the importer's source
   (`"cc"`, `"persona-journal"`, `"apple-notes"`). The slug flows
   through [`RawItem::source_kind`] into
   [`FootprintSource::kind`] and eventually into the DB unique
   index `(source_kind, source_locator)`, so it must be stable
   across releases of the same importer. See
   [`scanner::RawItem::source_kind`] for the full rule.

2. **`occurred_at` fallback ladder** — parsers pick the timestamp
   with the highest fidelity available: (a) a timestamp *inside*
   the payload (message header, DB column), (b)
   [`RawItem::occurred_at`] (scanner-derived: file `mtime`, row
   column), (c) `Utc::now()` as a last resort. Never invert the
   order — an `mtime` for a JSONL session log is the *file's* last
   write, not the individual message's. See [`parser::SourceParser`]
   for the canonical form.

3. **Idempotent `locator` under `Watch` mode** — [`ScanMode::Watch`]
   re-emits whole files as they change (append-heavy sources like
   `.jsonl` session logs are the common case). The parser is
   responsible for producing a **record-level** locator so
   unchanged records collapse via the server-side unique index and
   only new records land. The `<file-path>#<record-uuid>` pattern
   is the canonical shape; see
   [`footprint::FootprintSource::locator`] for the full patterns
   and constraints.

4. **Adding a new modality** — new modalities are added as
   new [`Footprint`] variants, not through a stringly-typed
   escape hatch. Every modality gets a typed struct so the
   compiler shows plugin authors the fields it needs, and the
   [`Footprint::into_asset_spec`] arm centralises truncation /
   label / modality-slug rules. `JournalKind::Other` /
   `ChatRole::Other` / `DocFormat::Other` are per-variant escape
   hatches for sub-kind slugs, not for whole new modalities.

## Import target catalogue

Field-mapping reference for schemas Asterism plans to import
(Character Card V2/V3, PNG tEXt embed, CharacterHub, RisuAI,
AgnAI, KoboldAI, SillyTavern chat JSONL / World Info,
NovelAI Lorebook, ChatGPT export, Claude data export, Letta,
MemoryPlugin, SillyTavern backup zip). See [`catalogue`] for
per-target split rules, locator patterns, and unverified fields.

## Modules

- [`bundle`](bundle.md): Deriving the grouping key that ties the footprints of one container
- [`card`](card.md): # Character-card parser subsystem
- [`card::envelope`](card__envelope.md): Envelope + context types shared by every character-card parser.
- [`card::parser`](card__parser.md): [`CharacterCardParser`] trait + canonical V2 slot logic.
- [`card::parser::v2_default`](card__parser__v2_default.md): Canonical V2 slot logic, exposed as free functions so derivatives
- [`card::png_chunk`](card__png_chunk.md): Character-card PNG `tEXt` chunk decoders.
- [`card::registry`](card__registry.md): Registry that dispatches a [`CardEnvelope`] to the right
- [`card::source_parser`](card__source_parser.md): [`SourceParser`] adapter that turns any character-card [`RawItem`]
- [`card::v2`](card__v2.md): Canonical Character Card V2 parser (see [`crate::catalogue`] section 1).
- [`card::v3`](card__v3.md): Character Card V3 parser (see [`crate::catalogue`] section 2).
- [`catalogue`](catalogue.md): # Import target catalogue
- [`client`](client.md): Thin HTTP client for the asterism-server API.
- [`footprint`](footprint.md): `Footprint` — the typed, plugin-facing shape a parser hands back.
- [`harvest`](harvest.md): # Agent-harvest intake subsystem
- [`harvest::envelope`](harvest__envelope.md): Typed schema for `asterism_agent_harvest` v1 JSON dumps.
- [`harvest::parser`](harvest__parser.md): [`SourceParser`] impl that decodes `asterism_agent_harvest` JSON
- [`mapper`](mapper.md): Convenience shape for parser output plus the mapping to the wire
- [`parser`](parser.md): `SourceParser` — turn a scanned [`RawItem`] into one or more
- [`progress`](progress.md): Tiny progress reporter — running success / failure counters, plus
- [`runner`](runner.md): Shared importer execution pipeline.
- [`scanner`](scanner.md): `SourceScanner` trait and shared item type.
- [`scanner::fs`](scanner__fs.md): `FsScanner` — filesystem source scanner.
- [`scanner::sqlite`](scanner__sqlite.md): `SqliteScanner` — SQLite source scanner.

