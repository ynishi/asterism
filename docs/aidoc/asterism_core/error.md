# asterism-core::error

`DomainError` — the innermost error type shared across every layer of
`asterism-core`.

Outer layers (the Tauri `UiError`, the HTTP `ApiError`, MCP tool errors)
convert from this enum. Infrastructure-level failures are collapsed into
`Infra` so that domain vocabulary (`NotFound` / `Duplicate` / `Validation`
/ `Conflict`) stays clean.

## Types

- `ConflictKind` — What a caller can do about a [`Conflict`](DomainError::Conflict).
- `DomainError` — Errors raised by the domain layer.

