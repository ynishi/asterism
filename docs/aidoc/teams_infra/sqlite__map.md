# teams-infra::sqlite::map

Row ↔ domain conversion helpers for the teams tables.

The reading convention is `asterism-infra`'s: inside an isle closure
only `rusqlite` primitives are handled, and promoting rows into
domain types (including validation) happens **outside** the closure.
Writes are the deliberate exception, documented in
[`repo`](crate::sqlite::repo): the same-tx rule requires the domain
invariants to be evaluated *inside* the transaction, on the state it
is about to change.

## Functions

- `actor_from_json` — Parses the `actor` TEXT column back into a [`LedgerActor`].
- `actor_to_json` — Serialises a [`LedgerActor`] into the `actor` TEXT column — the
- `datetime_to_ms` — An instant as the epoch-ms column that carries it.
- `infra_err` — Wraps an infrastructure error (typically `IsleError`) into
- `ms_to_datetime` — An epoch-ms column as an instant, refusing a value no clock
- `subject_from_ref` — Rebuilds a [`SubjectRef`] from its `(ref_type, ref_value)` columns —
- `subject_to_ref` — Splits a [`SubjectRef`] into the `(ref_type, ref_value)` pair the

