//! Canonical Character Card V2 parser (see [`crate::catalogue`] section 1).
//!
//! All six [`super::parser::CharacterCardParser`] hooks inherit their
//! default implementations, which encode the V2 spec verbatim. This
//! type is a marker whose sole job is to advertise `spec() =
//! "chara_card_v2"` so [`super::CardParserRegistry`] can route to it.

use super::parser::CharacterCardParser;

/// V2 character-card parser.
///
/// Inherits the default composition of [`CharacterCardParser`] —
/// [`super::parser::v2_default`] contains the concrete slot logic.
pub struct V2Parser;

impl CharacterCardParser for V2Parser {
    fn spec(&self) -> &'static str {
        "chara_card_v2"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Footprint;
    use crate::card::envelope::{CardContext, CardEnvelope};
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn v2_parser_covers_v2_spec_end_to_end() {
        let env = CardEnvelope::from_json(json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Alice",
                "description": "curious",
                "first_mes": "hello",
                "alternate_greetings": ["hi"],
                "creator_notes": "notes",
                "character_book": {
                    "entries": [{"keys": ["fire"], "content": "burn", "id": "e1"}]
                }
            }
        }))
        .unwrap();
        let ctx = CardContext {
            source_kind: "chara",
            locator: "/tmp/a.png",
            session_id: "s",
            occurred_at: Utc::now(),
            platform: Some("SillyTavern"),
        };
        let out = V2Parser.parse(&env, &ctx);
        // 2 notes (name/description) + 2 greetings + 1 creator_notes doc +
        // 1 book entry = 6 footprints.
        assert_eq!(out.len(), 6, "V2 default composition yields 6 footprints");
        let notes = out
            .iter()
            .filter(|f| matches!(f, Footprint::Note(_)))
            .count();
        let msgs = out
            .iter()
            .filter(|f| matches!(f, Footprint::ChatMessage(_)))
            .count();
        let docs = out
            .iter()
            .filter(|f| matches!(f, Footprint::Doc(_)))
            .count();
        assert_eq!((notes, msgs, docs), (3, 2, 1));
    }
}
