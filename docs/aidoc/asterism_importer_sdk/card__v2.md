# asterism-importer-sdk::card::v2

Canonical Character Card V2 parser (see [`crate::catalogue`] section 1).

All six [`super::parser::CharacterCardParser`] hooks inherit their
default implementations, which encode the V2 spec verbatim. This
type is a marker whose sole job is to advertise `spec() =
"chara_card_v2"` so [`super::CardParserRegistry`] can route to it.

## Types

- `V2Parser` — V2 character-card parser.

