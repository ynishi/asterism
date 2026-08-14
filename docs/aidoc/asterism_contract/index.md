# asterism-contract 0.0.0

# asterism-contract — Command / Query / Response DTOs

## Role

Leaf crate of the Asterism workspace: pure serde structs that every
caller (`asterism-core`, `asterism-server`, `asterism-ui/src-tauri`,
the importer / exporter crates) uses to pass data across boundaries.
Depends on nothing else inside the workspace so there is no risk of a
dependency cycle.

- **CommandSchemaLayer.** Uses `schema-bridge` for
  `#[derive(SchemaBridge)]` so a single source generates TypeScript
  bindings (via `asterism-ui/build.rs`), and the opt-in `json-schema`
  feature adds `schemars::JsonSchema` derives on the shapes the MCP
  transport (`asterism-server::mcp`) publishes as tool input schemas.
- **Boundary reuse.** The same DTOs feed the application services,
  Tauri IPC, the HTTP API, and the per-source importers / per-backend
  exporters.

## Design intent

- **Leaf purity.** No other Asterism crate is imported here; domain
  types (`Repository` traits, `DomainError`) live in `asterism-core`.
- **A field's grammar travels with the field.** Mostly these are
  plain serde structs, and [`digest`] is the one exception that
  proves the rule: `AddAssetCommand::declared_content_hash` is a
  field here, so the notation its value is written in
  (`sha256:<hex>`, and the hasher that produces one) is here too.
  What a digest *means* — duplicate axes, reserved markers, the
  versioned container tags — is domain and stays in `asterism-core`,
  which re-exports the notation rather than restating it.
- **Codegen scope.** `schema-bridge` handles primitives, `Vec`,
  `Option`, structs, unit enums, newtypes, tuples, and `serde`
  `rename_all`. Tagged unions, generics, and `Result` fall outside its
  scope and require hand-written TypeScript.

## Wire representation

- Ids: UUID hyphenated `String`.
- Timestamps: unix epoch milliseconds as `i64` (matches the SQLite
  schema on disk).
- Extension bags: JSON serialised into a `String` (schema-bridge does
  not know how to render `serde_json::Value`).

## Modules

- [`command`](command.md): Command DTOs — inputs for state-changing operations.
- [`digest`](digest.md): The notation a digest is written in — the `sha256:` tag, and the
- [`dto`](dto.md): Response DTOs — outputs shared between the application services, the
- [`query`](query.md): Query DTOs — inputs for read-side operations.
- [`query_group`](query_group.md): Query Group `query_json` — the persisted rule of a query-backed Group.
- [`sidecar`](sidecar.md): The shape of an exported artefact's `<file>.meta.json` sidecar.
- [`sort`](sort.md): Sort specification DTO — the wire form of the grid's sort axis.

