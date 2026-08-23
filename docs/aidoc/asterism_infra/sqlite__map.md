# asterism-infra::sqlite::map

Row ↔ domain conversion helpers.

Convention: inside an isle closure we handle only `rusqlite` primitives
(`Result<_, rusqlite::Error>`). Promoting rows into domain types
(including validation) happens **outside** the closure.

A corrupted row — including a slug this build has no name for —
surfaces as `DomainError::Infra`. It reads as the caller's fault
nowhere: the request was fine and the row was not, and no different
request avoids it. This said "`Infra` / `Validation`" and left the
choice open, which is how three decode failures came to answer
`400`. [`crate::fault`] is where that is decided now, and
`StoreFault::CorruptRow` is the case these reach for.

## Functions

- `datetime_to_ms` — Converts `DateTime<Utc>` into unix epoch milliseconds for storage.
- `escape_like` — Escapes `%`, `_`, and `\` for a SQL `LIKE` pattern
- `infra_err` — Wraps an infrastructure error (typically `IsleError`) into
- `json_to_strings` — Parses a JSON array `TEXT` column into `Vec<String>`
- `ms_to_datetime` — Converts unix epoch milliseconds (as stored in `INTEGER` columns) into
- `opt_u32` — Converts `Option<i64>` (from a nullable `INTEGER` column) into
- `opt_u64` — Converts `Option<i64>` (from a nullable `INTEGER` column) into
- `strings_to_json` — Serialises a string slice as a JSON array `TEXT` value.

