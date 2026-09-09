# asterism-importer-sdk::card::charx

`.charx` — a V3 card and its assets inside one ZIP.

The third container for the same card, beside the PNG tEXt chunk and
the standalone `.json`. What it adds is that the assets travel with
it: `card.json` at the root, and the pictures the card refers to
under `assets/<type>/<category>/<file>` (the layout is written down
in [`crate::catalogue`], V3 section).

**The entries are addressed, not extracted.** This module reads
`card.json` out of the archive and lists what else is in there; the
footprints it feeds carry `<container>#<entry>` locators, which the
domain reads back as a
`SourceLocator::Record` — a container plus an address its reader
resolves. Nothing is written to disk, so there is no second copy to
keep in step with the first.

Only the card route reaches this. An archive is not opened because
it is an archive: a plain `.zip` is one asset that states its
format, and nothing looks inside it.

## Functions

- `entry_for_uri` — The entry an `assets[]` URI names, when it names one inside this
- `is_zip` — Whether a payload opens like a ZIP.
- `read` — Read a `.charx`: the card at [`CARD_JSON`], and the names of the

## Types

- `Charx` — What one `.charx` holds: the card, and the names of everything else

## Constants

- `CARD_JSON` — Where a `.charx` keeps the card itself. Fixed by the V3 spec.
- `EMBEDDED_SCHEME` — The scheme a V3 `assets[].uri` uses for something inside the

