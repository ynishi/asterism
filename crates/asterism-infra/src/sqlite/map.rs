//! Row ↔ domain conversion helpers.
//!
//! Convention: inside an isle closure we handle only `rusqlite` primitives
//! (`Result<_, rusqlite::Error>`). Promoting rows into domain types
//! (including validation) happens **outside** the closure. Corrupted
//! rows and unknown slugs surface as `DomainError::Infra` /
//! `DomainError::Validation` for the caller.

use asterism_core::error::DomainError;
use chrono::{DateTime, Utc};

/// Converts unix epoch milliseconds (as stored in `INTEGER` columns) into
/// `DateTime<Utc>`.
pub fn ms_to_datetime(ms: i64) -> Result<DateTime<Utc>, DomainError> {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .ok_or_else(|| DomainError::Infra(anyhow::anyhow!("timestamp out of range: {ms}")))
}

/// Converts `DateTime<Utc>` into unix epoch milliseconds for storage.
pub fn datetime_to_ms(dt: &DateTime<Utc>) -> i64 {
    dt.timestamp_millis()
}

/// Parses a JSON array `TEXT` column into `Vec<String>`
/// (`labels`, `keywords`, `vis_sharing`).
pub fn json_to_strings(json: &str) -> Result<Vec<String>, DomainError> {
    serde_json::from_str(json)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("corrupt JSON array column: {e}")))
}

/// Serialises a string slice as a JSON array `TEXT` value.
pub fn strings_to_json<S: AsRef<str>>(items: &[S]) -> String {
    serde_json::Value::Array(
        items
            .iter()
            .map(|s| serde_json::Value::String(s.as_ref().to_string()))
            .collect(),
    )
    .to_string()
}

/// Wraps an infrastructure error (typically `IsleError`) into
/// `DomainError::Infra`.
pub fn infra_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> DomainError {
    DomainError::Infra(anyhow::Error::new(e))
}

/// Converts `Option<i64>` (from a nullable `INTEGER` column) into
/// `Option<u64>`. A negative value is treated as data corruption.
pub fn opt_u64(value: Option<i64>, column: &str) -> Result<Option<u64>, DomainError> {
    value
        .map(|v| {
            u64::try_from(v).map_err(|_| {
                DomainError::Infra(anyhow::anyhow!("negative value in column {column}: {v}"))
            })
        })
        .transpose()
}

/// Converts `Option<i64>` (from a nullable `INTEGER` column) into
/// `Option<u32>`, refusing both ends.
///
/// Distinct from [`opt_u64`] one function up, which only has a floor to
/// check: a 32-bit target has a ceiling as well, and `INTEGER` columns
/// hold values above it. A `as u32` cast would take `4294967296` down to
/// `0` — a value that reads as a measurement and sorts ahead of every
/// real one — and `-1` up to `4294967295`. Both are silent, so both are
/// errors instead: `u32::try_from` covers the two ends in one question
/// (the shape `repo/thread.rs` uses for `message_count`).
///
/// Deliberately unlike the truncating conversions elsewhere in the same
/// read path (`rating` narrows with `as u8`). Those are bounded by a
/// domain rule the writer holds to; a pixel dimension is bounded by
/// nothing but the column, so out-of-range here means the row is wrong
/// and saying so is the only honest answer.
pub fn opt_u32(value: Option<i64>, column: &str) -> Result<Option<u32>, DomainError> {
    value
        .map(|v| {
            u32::try_from(v).map_err(|_| {
                DomainError::Infra(anyhow::anyhow!(
                    "value out of u32 range in column {column}: {v}"
                ))
            })
        })
        .transpose()
}

/// Escapes `%`, `_`, and `\` for a SQL `LIKE` pattern
/// (paired with `ESCAPE '\'`).
pub fn escape_like(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both ends, and the one in the middle that has to keep working.
    ///
    /// `4294967296` is `u32::MAX + 1`, which is the case a cast would
    /// turn into `0` — the value that reads as a real measurement. The
    /// boundary itself (`u32::MAX`) is asserted as *accepted*, so the
    /// check cannot be off by one in the direction that rejects good
    /// rows.
    #[test]
    fn a_dimension_column_refuses_both_ends_and_keeps_the_boundary() {
        assert_eq!(opt_u32(None, "width_px").unwrap(), None);
        assert_eq!(opt_u32(Some(0), "width_px").unwrap(), Some(0));
        assert_eq!(opt_u32(Some(1920), "width_px").unwrap(), Some(1920));
        assert_eq!(
            opt_u32(Some(i64::from(u32::MAX)), "width_px").unwrap(),
            Some(u32::MAX),
            "the top of the range is in it"
        );

        let over = opt_u32(Some(4_294_967_296), "width_px")
            .expect_err("one past the top is not zero pixels");
        assert!(
            matches!(&over, DomainError::Infra(e) if e.to_string().contains("width_px")
                && e.to_string().contains("4294967296")),
            "the message names the column and the value: {over}"
        );

        let under =
            opt_u32(Some(-1), "height_px").expect_err("and a negative is not 4294967295 pixels");
        assert!(
            matches!(&under, DomainError::Infra(e) if e.to_string().contains("height_px")),
            "{under}"
        );
    }
}
