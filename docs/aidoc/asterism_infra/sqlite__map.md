# asterism-infra::sqlite::map

Row ↔ domain conversion helpers.

Convention: inside an isle closure we handle only `rusqlite` primitives
(`Result<_, rusqlite::Error>`). Promoting rows into domain types
(including validation) happens **outside** the closure. Corrupted
rows and unknown slugs surface as `DomainError::Infra` /
`DomainError::Validation` for the caller.

## Functions

- `datetime_to_ms` — Converts `DateTime<Utc>` into unix epoch milliseconds for storage.
- `escape_like` — Escapes `%`, `_`, and `\` for a SQL `LIKE` pattern
- `infra_err` — Wraps an infrastructure error (typically `IsleError`) into
- `json_to_strings` — Parses a JSON array `TEXT` column into `Vec<String>`
- `ms_to_datetime` — Converts unix epoch milliseconds (as stored in `INTEGER` columns) into
- `opt_u32` — Converts `Option<i64>` (from a nullable `INTEGER` column) into
- `opt_u64` — Converts `Option<i64>` (from a nullable `INTEGER` column) into
- `strings_to_json` — Serialises a string slice as a JSON array `TEXT` value.

