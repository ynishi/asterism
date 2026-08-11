//! The one contradiction the service layer still has to check about the
//! attribution fields a command carries.
//!
//! Services do not *read* those fields any more — the adapter that
//! received the request translated them into an
//! [`AttributionContext`](crate::domain::attribution::AttributionContext)
//! and the context is the only write source. What is left
//! is a contradiction that translation cannot express: a request that
//! arrived through the owner's own surface, carrying fields whose only
//! purpose is to state an attribution a remote caller could not
//! otherwise supply.
//!
//! The check lives here, in one function, rather than in each adapter.
//! An adapter-side check is opt-in by shape — the next Tauri command
//! someone writes simply would not have it, and the assertion would be
//! silently dropped instead of refused, which is the failure mode this
//! whole wave exists to remove.

use crate::domain::attribution::{AttributionChannel, AttributionContext};
use crate::error::DomainError;

/// Refuses a command that carries attribution fields on a request that
/// came in through the owner's own operation surface.
///
/// `fields` names the command's attribution fields and says whether each
/// one was present, so the error can quote the ones that were set rather
/// than the whole command.
///
/// Every other channel is fine: on `Asserted` the fields *are* the
/// assertion (the adapter already turned them into the context), and on
/// `Authenticated` the auth layer's own reject is the one that applies,
/// which is a wave away.
pub(crate) fn refuse_assertion_from_owner_surface(
    attribution: &AttributionContext,
    fields: &[(&'static str, bool)],
) -> Result<(), DomainError> {
    if attribution.attributed_via() != Some(AttributionChannel::OwnerSurface) {
        return Ok(());
    }
    let stated: Vec<&str> = fields
        .iter()
        .filter(|(_, present)| *present)
        .map(|(name, _)| *name)
        .collect();
    if stated.is_empty() {
        return Ok(());
    }
    Err(DomainError::Validation(format!(
        "the owner's own surface answers who is writing; it cannot also be told — \
         drop {} from the command",
        stated.join(", ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::attribution::{Author, OperatorRef};

    #[test]
    fn the_owner_surface_refuses_a_command_that_states_an_attribution() {
        let err = refuse_assertion_from_owner_surface(
            &AttributionContext::owner_surface(),
            &[("author_kind", true), ("operator_ai", false)],
        )
        .expect_err("a field stated on the owner's surface is a contradiction");
        let msg = err.to_string();
        assert!(
            msg.contains("author_kind"),
            "names the field that was set: {msg}"
        );
        assert!(
            !msg.contains("operator_ai"),
            "and only that one, so the message points at the fix: {msg}"
        );
    }

    #[test]
    fn a_bare_command_on_the_owner_surface_is_the_normal_case() {
        assert!(
            refuse_assertion_from_owner_surface(
                &AttributionContext::owner_surface(),
                &[("author_kind", false), ("operator_ai", false)],
            )
            .is_ok()
        );
    }

    #[test]
    fn on_every_other_channel_the_fields_are_the_assertion() {
        // Asserted: the adapter turned these very fields into the
        // context, so seeing them set is not a contradiction — it is the
        // request.
        let asserted = AttributionContext::asserted(
            Some(Author::Subject("alice".into())),
            Some(OperatorRef::new("claude-code").unwrap()),
        )
        .unwrap();
        assert!(
            refuse_assertion_from_owner_surface(
                &asserted,
                &[("author_kind", true), ("operator_ai", true)],
            )
            .is_ok()
        );
        // Unrecorded (a system write) carries no channel at all.
        assert!(
            refuse_assertion_from_owner_surface(
                &AttributionContext::unrecorded(),
                &[("author_kind", true)],
            )
            .is_ok()
        );
    }
}
