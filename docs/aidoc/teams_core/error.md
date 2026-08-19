# teams-core::error

`DomainError` — the innermost error type of the teams plane.

Same convention as `asterism-core`'s enum of the same name: outer
layers (`teams-server`'s HTTP / MCP errors, once they exist) convert
from this, infrastructure failures are collapsed into `Infra`, and
the domain vocabulary stays clean. The two enums are deliberately
separate types — the teams plane and the local app do not share an
error surface, only `asterism-core`'s domain vocabulary (#83 §4).

## Types

- `DomainError` — Errors raised by the teams domain layer.

