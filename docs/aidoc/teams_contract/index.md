# teams-contract 0.0.0

# teams-contract — wire contract for the teams plane

The request/response shapes of the `/teams/*` HTTP surface
(#83 §5, the #91 slice), through the same schema-bridge flow as
`asterism-contract`: plain serde structs with
`#[derive(SchemaBridge)]`, so a TypeScript consumer can be
generated from the one source of truth when a UI arrives, and the
shapes stay hand-off-able between the HTTP body and any later MCP
tool schema.

## Wire representation

Same conventions as `asterism-contract`:

- Ids: UUID hyphenated `String`.
- Timestamps: unix epoch milliseconds as `i64` (matches the SQLite
  schema on disk).
- Opaque JSON (a ledger event's kind-versioned payload): serialised
  into a `String` — schema-bridge does not render
  `serde_json::Value`, and the payload is per-kind data the
  envelope deliberately does not model (#83 §2).

Validation is not here: role words go through
`teams-core`'s `Role::parse` on the server side, ids through
`Uuid::parse_str` — this crate only defines the shapes.

## Modules

- [`command`](command.md): Command DTOs — inputs of the state-changing `/teams/*` routes.
- [`dto`](dto.md): Response DTOs of the `/teams/*` surface.

