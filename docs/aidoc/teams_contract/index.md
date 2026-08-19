# teams-contract 0.0.0

# teams-contract — wire contract for the teams plane (shell)

Near-empty by design in this slice: #83 §4 gives this crate the
schema-bridge / schemars flow over `teams-core`'s types, mirroring
how `asterism-contract` serves `asterism-core` — and that lands
with the API slices. What exists now is the crate boundary, so the
layering (`teams-server` reaches domain types through a contract
crate, not by re-deriving schemas inline) is fixed before the first
route needs one.

