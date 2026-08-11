//! Turning what a remote caller said into the attribution a write
//! records.
//!
//! Both remote surfaces this crate owns — the HTTP routes and the MCP
//! tools — are the [`Asserted`](AttributionChannel::Asserted) channel:
//! whatever `author_kind` / `author_subject` / `operator_ai` a command
//! carries is the caller's own statement about itself, believed and
//! labelled as such. Translating those fields into an
//! [`AttributionContext`] is the adapter's job; the
//! services below this layer never read them.
//!
//! It is one function rather than an inline `AttributionContext::asserted`
//! per route so that every remote write agrees on what the fields mean,
//! including the two answers the pair form makes possible: a corrupt
//! pair is refused here, and an owner claim is refused by the
//! constructor (a caller cannot state owner-ness — that follows from the
//! surface or from authentication, never from the claim).

use asterism_core::domain::attribution::{AttributionContext, Author, OperatorRef};
use asterism_core::error::DomainError;

/// Builds the context for a request that arrived over HTTP or MCP.
///
/// Passing all three as `None` is the ordinary case for the many
/// commands that carry no attribution fields at all: a caller that
/// stated nothing records nothing, which is the same value a system
/// write leaves behind (attribution rule 3).
pub fn asserted(
    author_kind: Option<&str>,
    author_subject: Option<&str>,
    operator_ai: Option<&str>,
) -> Result<AttributionContext, DomainError> {
    let author = Author::from_columns(author_kind, author_subject)?;
    let operator = operator_ai.map(OperatorRef::new).transpose()?;
    AttributionContext::asserted(author, operator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::attribution::AttributionChannel;

    #[test]
    fn a_stated_pair_becomes_an_asserted_context() {
        let ctx = asserted(Some("subject"), Some("alice"), Some("claude-code")).unwrap();
        assert_eq!(ctx.author(), Some(&Author::Subject("alice".into())));
        assert_eq!(
            ctx.operator_ai().map(OperatorRef::as_str),
            Some("claude-code")
        );
        assert_eq!(ctx.attributed_via(), Some(AttributionChannel::Asserted));
    }

    #[test]
    fn stating_nothing_records_nothing() {
        let ctx = asserted(None, None, None).unwrap();
        assert_eq!(ctx.author(), None);
        assert_eq!(ctx.operator_ai(), None);
        assert_eq!(
            ctx.attributed_via(),
            None,
            "a channel with nothing to attribute records no channel"
        );
    }

    #[test]
    fn a_remote_caller_cannot_call_itself_the_owner() {
        // If this passed, every HTTP client could write rows
        // indistinguishable from the ones the desktop app writes.
        assert!(asserted(Some("owner"), None, None).is_err());
        // And a half-written pair is refused before it reaches a write.
        assert!(asserted(Some("subject"), None, None).is_err());
    }
}
