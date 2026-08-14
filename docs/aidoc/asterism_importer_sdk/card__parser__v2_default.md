# asterism-importer-sdk::card::parser::v2_default

Canonical V2 slot logic, exposed as free functions so derivatives
can chain (call the V2 version, then append their own outputs)
rather than re-implement.

Every function is deterministic given the same envelope + context
and produces zero footprints when the corresponding slot is
absent, empty, or the wrong shape (so importers can call them
unconditionally without pre-checking).

## Functions

- `book` — V2 default for [`super::CharacterCardParser::parse_book`].
- `card_tags` — Card-level tag labels (`data.tags[]`) as owned strings. Empty
- `creator_notes` — V2 default for
- `extensions_bag` — Extract the extensions bag (`data.extensions`) as a JSON value
- `greetings` — V2 default for [`super::CharacterCardParser::parse_greetings`].
- `mes_example` — V2 default for
- `text_slots` — V2 default for [`super::CharacterCardParser::parse_text_slots`].

## Constants

- `TEXT_SLOT_NAMES` — The six V2 text-slot names, in the order [`text_slots`] emits

