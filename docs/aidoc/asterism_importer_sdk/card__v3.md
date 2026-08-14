# asterism-importer-sdk::card::v3

Character Card V3 parser (see [`crate::catalogue`] section 2).

Extends the V2 default composition with three V3-only slot groups:

- `creator_notes_multilingual{lang}` → one extra
  [`Doc`] (Markdown) per language (appended to the V2
  creator_notes doc via
  [`super::parser::v2_default::creator_notes`]).
- `group_only_greetings[]` → one [`ChatMessage`]
  per entry, at high thread positions so they sort after normal
  alternates without colliding.
- `assets[]` → one [`Image`] per entry, keyed by
  `(type, name)` so the grid can group emotions / backgrounds under
  the card's shared `session_id`.

## Types

- `V3Parser` — V3 character-card parser. Inherits V2 slot logic and adds the V3

