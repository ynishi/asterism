# asterism-core::error

`DomainError` — the innermost error type shared across every layer of
`asterism-core`.

Outer layers (the Tauri `UiError`, the HTTP `ApiError`, MCP tool errors)
convert from this enum. Infrastructure-level failures are collapsed into
`Infra` so that domain vocabulary (`NotFound` / `Duplicate` / `Validation`
/ `Conflict`) stays clean.

# Which variant a refusal belongs to

Stated here because it was not stated anywhere, and the cost of that
was fifty-eight call sites each deciding for themselves — thirty-nine
of them inside a SQLite repository, which is not a layer that can
answer an API question. The four definitions below are the rule.

A repository does not apply them. `asterism-infra` has a
`StoreFault` type of its own — seven cases naming what storage did,
with one hand-written conversion into this enum whose doc is the
table those seven land on. That crate is not published, so there is
nothing to link; `cargo doc` and the source both have it under
`asterism_infra::fault`.

- **[`Validation`](DomainError::Validation)** — the request cannot be
  satisfied as written, and that is decidable from the request plus
  the identity of what it addressed. Nothing changes on its own. A
  blank name; an outcome that is neither word; a directory moved
  into itself; a reply naming a message of another conversation.
- **[`NotFound`](DomainError::NotFound)** and its two named
  siblings — what was addressed is not there.
- **[`Conflict`](DomainError::Conflict)** — the request is
  well-formed and *would* be satisfiable. What refuses it is the
  current state, and that state is a thing that changes. A name
  already taken; a lost optimistic lock; work somebody else already
  ended; a precondition held by another row.
- **[`Infra`](DomainError::Infra)** — the store handed back
  something that could not have been written, or the machine
  underneath failed. The caller is not involved.

The line between the first and the third is the one that gets drawn
wrongly, and "would a different request work" is not it — a blank
name passes that test and is plainly a `Validation`. Ask instead
whether *the state* is what refuses, and whether that state is
something that changes.

## Types

- `ConflictKind` — What a caller can do about a [`Conflict`](DomainError::Conflict).
- `DomainError` — Errors raised by the domain layer.

