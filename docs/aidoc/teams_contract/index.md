# teams-contract 0.0.0

# teams-contract — wire contract for the teams plane

What is left here after #148 decision 15: the `/teams/*` shapes a
member's client does not speak. The roster verbs, which are an
owner's; the substrate's own blob upload and its purge two-step;
the instance head registry. Everything a client says or reads moved
to `asterism-teams-wire`, the MIT/Apache leaf both planes may link — this
crate cannot be that leaf, for the two reasons #148 gives: its
licence is AGPL-3.0-or-later (#162), which the local plane may not
link, and it declares a `teams-core` dependency.

Nothing was copied. `LedgerEventDto` is named from the leaf where
two shapes here embed it, because one type with two homes is the
failure a leaf exists to prevent.

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

- [`command`](command.md): Command DTOs — inputs of the state-changing `/teams/*` routes an
- [`dto`](dto.md): Response DTOs of the `/teams/*` surfaces a member's client does not

