//! AlbumMeta — what a person or an agent *says about* an asset, in
//! Album's own words.
//!
//! # The gap this fills
//!
//! An asset's `extra` bag already has two populations. Its top level is
//! what an importer read out of the source — EXIF fields, a camera
//! model, the raw text of a PNG chunk — facts about the artefact,
//! reported by whatever was holding it. Underscore-prefixed keys are
//! Album's own bookkeeping, and `_trace` in particular is where this
//! library keeps assertions rather than facts (see
//! [`content_hash::DECLARED_HASH_NOTE_KEY`](crate::domain::content_hash::DECLARED_HASH_NOTE_KEY)
//! on why a claim does not get a column of its own).
//!
//! What neither zone had is a place for a statement **Album's user
//! makes**, about anything, under a name they chose. `_trace` holds
//! four kinds of statement and Album knows what all four mean; there
//! was no way to add a fifth without teaching the application about it
//! first.
//!
//! # Why not reuse what is already there
//!
//! - **Tags / labels** are facets: words that put a row in a bucket.
//!   They carry no record of who said them or through what, so a tag
//!   cannot say "an agent asserted this at import" as distinct from "a
//!   person typed it".
//! - **`extra` top level** is the importer's, and putting a person's
//!   assertion beside a camera's readings is how a later reader comes
//!   to treat one as the other.
//! - **First-class columns** (`title`, `rating`, `register_note`) are
//!   single slots the application understands. Adding one per thing
//!   somebody might want to say is not a design.
//!
//! # Why external identifiers land here rather than becoming keys
//!
//! The case this was designed against: an artefact arrives carrying
//! something that *looks* like an identifier — a workflow id, a
//! generator's own reference, an id minted by hand. It is tempting to
//! use it as a natural key, and that is the mistake. An identifier is
//! only an identity if its issuer maintains it as one; most do not.
//! ComfyUI is the measured example — its embedded graph carries no
//! identifier for the artefact at all, its node ids are unique only
//! within one file, and the reference to an input image is a bare
//! filename. Borrowing uniqueness from a system that is not keeping any
//! produces a key that silently stops being unique.
//!
//! So an external identifier is recorded here as **what it is**: a
//! statement somebody made, with a name they gave it. Asset identity
//! stays [`AssetId`](crate::domain::value::AssetId), internal and
//! minted by Album, exactly as it was. Looking rows up by a recorded
//! value is a *filter* over a secondary index — a different layer, and
//! deliberately not this one.
//!
//! # Why there is no verdict
//!
//! [`declared_hash`](crate::domain::content_hash::DECLARED_HASH_NOTE_KEY)
//! records a claim and later a verdict, because a job reads the bytes
//! and can say whether the claim held. Nothing can do that here. A
//! statement under a name its author invented has no checker, so a
//! `verified` field would either stay absent forever or be filled in by
//! something inventing an answer.

use crate::error::DomainError;

/// Field inside `_trace` that holds every AlbumMeta entry.
///
/// Nested rather than spread across `_trace` itself, and the reason is
/// the one `DECLARED_HASH_NOTE_KEY` gives for staying out of the
/// `material` table: two things that look alike in one place means
/// every reader picks between them, and a wrong pick reads a person's
/// statement as one of the four `_trace` fields the application acts
/// on. Under `meta` a reader that wants "what somebody said" reads one
/// object and gets exactly that, and a key somebody invents can never
/// collide with a name the application owns.
pub const META_KEY: &str = "meta";

/// Longest accepted key.
const MAX_KEY_LEN: usize = 64;

/// Longest accepted value.
///
/// A statement, not a document. The bag is loaded with every read of
/// the row, and the place for something long is the material or a note
/// asset — both of which the library already streams on demand. The
/// limit is generous enough for any identifier, URL or sentence and
/// small enough that a thousand of them is still a small row.
const MAX_VALUE_LEN: usize = 2048;

/// Checks a key.
///
/// The same shape `SourceKind` accepts — lowercase, digits, `_`, `-` —
/// because these names end up in the same places: a filter expression,
/// a URL, a log line. Rejecting `.` matters most: the bag is JSON and a
/// dotted key reads as a path, so `a.b` would look addressable and not
/// be.
///
/// Uppercase is refused rather than folded. Folding would make
/// `Workflow` and `workflow` the same key while the caller's own
/// records still say two different things, and the caller finds out
/// when one overwrites the other.
pub fn parse_key(raw: &str) -> Result<String, DomainError> {
    let key = raw.trim();
    if key.is_empty() {
        return Err(DomainError::Validation(
            "an album meta key cannot be empty".into(),
        ));
    }
    if key.len() > MAX_KEY_LEN {
        return Err(DomainError::Validation(format!(
            "album meta key {key:?} is longer than {MAX_KEY_LEN} characters"
        )));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(DomainError::Validation(format!(
            "album meta key {key:?} may only hold lowercase letters, digits, \
             underscore and hyphen"
        )));
    }
    Ok(key.to_string())
}

/// Checks a value.
///
/// A string rather than arbitrary JSON. What goes here is a sentence
/// somebody said — an identifier, a URL, a name — and a nested document
/// is the importer's kind of thing, which has its own zone. It also
/// keeps the value directly comparable, which is what a filter over
/// these will need without having to decide what equality means for two
/// objects.
pub fn parse_value(raw: &str) -> Result<String, DomainError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(DomainError::Validation(
            "an album meta value cannot be empty — to take a statement back, \
             remove the key instead"
                .into(),
        ));
    }
    if value.len() > MAX_VALUE_LEN {
        return Err(DomainError::Validation(format!(
            "album meta value is longer than {MAX_VALUE_LEN} characters \
             ({} given)",
            value.len()
        )));
    }
    Ok(value.to_string())
}

/// The recorded statement.
///
/// `source` is the channel it arrived on, from the same vocabulary a
/// provenance claim uses ([`source`](crate::domain::provenance::source))
/// — the two are the same question about two different statements, and
/// a second word for it would be a second thing to keep in step.
///
/// `operator` is the agent it came through, absent when nobody stated
/// one. Same rule as everywhere else in this library: an unrecorded
/// operator is not a claim that a person was at the keyboard.
pub fn entry(value: &str, source: &str, operator: Option<&str>, at_ms: i64) -> serde_json::Value {
    let mut note = serde_json::json!({
        "value": value,
        "source": source,
        "declared_at_ms": at_ms,
    });
    if let Some(operator) = operator {
        note["operator"] = serde_json::json!(operator);
    }
    note
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_keeps_the_shape_a_filter_can_carry() {
        assert_eq!(parse_key(" workflow-id ").unwrap(), "workflow-id");
        assert_eq!(parse_key("plate_no_2").unwrap(), "plate_no_2");
    }

    #[test]
    fn a_dotted_key_is_refused_because_it_would_read_as_a_path() {
        // The bag is JSON. `a.b` looks addressable and is not, and the
        // caller finds out when their reader returns nothing.
        let err = parse_key("a.b").unwrap_err().to_string();
        assert!(err.contains("lowercase letters"), "{err}");
    }

    #[test]
    fn case_is_refused_rather_than_folded() {
        // Folding would silently make two of the caller's own keys one,
        // and the second write would eat the first.
        assert!(parse_key("Workflow").is_err());
    }

    #[test]
    fn an_empty_key_or_value_says_what_to_do_instead() {
        assert!(parse_key("   ").is_err());
        let err = parse_value("  ").unwrap_err().to_string();
        assert!(
            err.contains("remove the key instead"),
            "an empty value is a retraction attempt, and the message says \
             where the retraction lives: {err}"
        );
    }

    #[test]
    fn lengths_are_bounded_on_both_sides() {
        assert!(parse_key(&"k".repeat(MAX_KEY_LEN)).is_ok());
        assert!(parse_key(&"k".repeat(MAX_KEY_LEN + 1)).is_err());
        assert!(parse_value(&"v".repeat(MAX_VALUE_LEN)).is_ok());
        assert!(parse_value(&"v".repeat(MAX_VALUE_LEN + 1)).is_err());
    }

    #[test]
    fn an_entry_carries_its_channel_and_omits_an_operator_nobody_stated() {
        let note = entry(
            "xmp.did:1234",
            crate::domain::provenance::source::MANUAL,
            None,
            42,
        );
        assert_eq!(note["value"], serde_json::json!("xmp.did:1234"));
        assert_eq!(note["source"], serde_json::json!("manual"));
        assert_eq!(note["declared_at_ms"], serde_json::json!(42));
        assert!(
            note.get("operator").is_none(),
            "an absent operator stays absent rather than becoming null"
        );
    }

    #[test]
    fn an_entry_names_the_agent_when_there_was_one() {
        let note = entry(
            "v",
            crate::domain::provenance::source::MANUAL,
            Some("claude-code"),
            1,
        );
        assert_eq!(note["operator"], serde_json::json!("claude-code"));
    }
}
