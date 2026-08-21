//! Write-side guard for the attribution columns.
//!
//! `Author::from_columns` is the read-side check SQLite cannot express
//! as a CHECK on an `ALTER TABLE` column. This is its write-side twin
//! for the rule the channel column adds: **a row that records an author
//! or an operator records the channel that answer arrived through.**
//!
//! Without it, "author set, channel NULL" would keep being produced by
//! new writes, and that shape already means something else — it is how
//! a V47 / V48 row looks, which is precisely the set of rows an
//! authenticated deployment cannot resolve. A new row landing in the
//! legacy bucket would be indistinguishable from one that predates the
//! column.
//!
//! Called by the row builders in [`super::asset`] and
//! [`super::dispatch`], at the point where the values about to be
//! bound are visible as the columns themselves.
//!
//! [`attribution_columns`] lives here for the same reason: it is the
//! encoding half of the same concern, wanted by every table that
//! carries the triple, and a home in any one adapter would make the
//! next one reach sideways into a sibling.

use asterism_core::domain::attribution::PersistedAttribution;
use asterism_core::error::DomainError;

/// The four attribution column values in write order:
/// `(author_kind, author_subject, operator_ai, attributed_via)`.
pub type AttributionColumns = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Encodes an entity's attribution triple into the column values,
/// running [`assert_channel_recorded`] on the result — a row that
/// records somebody records the channel the answer arrived through.
pub fn attribution_columns(
    table: &'static str,
    attribution: &PersistedAttribution,
) -> Result<AttributionColumns, DomainError> {
    let (author_kind, author_subject) = match attribution.author() {
        Some(author) => {
            let (kind, subject) = author.encode();
            (Some(kind.to_string()), subject.map(str::to_string))
        }
        None => (None, None),
    };
    let operator_ai = attribution.operator_ai().map(|o| o.as_str().to_string());
    let attributed_via = attribution.attributed_via().map(|c| c.slug().to_string());
    assert_channel_recorded(
        table,
        author_kind.as_deref(),
        operator_ai.as_deref(),
        attributed_via.as_deref(),
    )?;
    Ok((author_kind, author_subject, operator_ai, attributed_via))
}

/// Rejects a row that records somebody without recording how that
/// answer arrived.
///
/// Takes the encoded column values rather than domain types so it sits
/// at the same boundary the `params!` list does — the last place where
/// what is about to be written is visible as itself.
///
/// - author / operator absent → nothing to attribute, no channel
///   expected (an ordinary unrecorded write).
/// - author or operator present, channel present → recorded.
/// - author or operator present, channel absent → rejected here.
pub fn assert_channel_recorded(
    table: &'static str,
    author_kind: Option<&str>,
    operator_ai: Option<&str>,
    attributed_via: Option<&str>,
) -> Result<(), DomainError> {
    let records_somebody = author_kind.is_some() || operator_ai.is_some();
    if records_somebody && attributed_via.is_none() {
        return Err(DomainError::Validation(format!(
            "{table}: attribution without a channel — an author or operator is recorded \
             but `attributed_via` is absent, which is the shape rows written before the \
             column carry and cannot be minted anew"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unrecorded_row_needs_no_channel() {
        assert!(assert_channel_recorded("asset", None, None, None).is_ok());
    }

    #[test]
    fn a_recorded_row_carries_its_channel() {
        assert!(
            assert_channel_recorded("asset", Some("owner"), None, Some("owner-surface")).is_ok()
        );
        assert!(
            assert_channel_recorded("asset", None, Some("claude-code"), Some("asserted")).is_ok()
        );
        assert!(
            assert_channel_recorded(
                "dispatch_job",
                Some("subject"),
                Some("codex"),
                Some("asserted")
            )
            .is_ok()
        );
    }

    #[test]
    fn recording_somebody_without_a_channel_is_refused() {
        // Both halves trip it independently: an operator alone is still
        // an attribution, and a new row of the legacy shape would be
        // unresolvable in exactly the way legacy rows are.
        assert!(assert_channel_recorded("asset", Some("owner"), None, None).is_err());
        assert!(assert_channel_recorded("asset", None, Some("claude-code"), None).is_err());
        assert!(
            assert_channel_recorded("dispatch_job", Some("subject"), Some("codex"), None).is_err()
        );
    }
}
