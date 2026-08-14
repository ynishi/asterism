# asterism-importer-sdk::footprint

`Footprint` — the typed, plugin-facing shape a parser hands back.

A footprint is *one* thing the user collected: one chat message, one
image, one doc, one note. This matches the domain intent in
`asterism-core::domain::asset::Asset` — an aggregate root for a single
footprint. The SDK exposes a typed enum here so plugin authors do not
have to guess which fields belong on which modality; the compiler
shows them the shape of every variant.

# Design notes

- **One `Footprint` = one `Asset` on the server.** A parser that
  receives one `RawItem` (say, one `.jsonl` file) usually emits many
  footprints (one per chat message inside the file).
- **Variants are aligned with `Modality` well-known slugs.** New
  modalities are added as new variants; the crate's API is still
  pre-stability, so breaking additions
  are cheap. Aspirationally-open extension via a `Custom` variant is
  deferred until an external plugin author actually needs it.
- **`extra` is a per-variant escape hatch.** Fields promoted to
  variant fields have a `Some`/typed shape enforced by the compiler;
  everything else lives in `extra` as raw JSON. When several sources
  grow the same key in `extra`, promote it.
- **The mapping to `AssetSpec` lives here, not in each parser.** The
  SDK owns the cover-hint truncation, the modality slug, the label
  normalisation — plugin authors only choose the variant and fill in
  the typed fields.

## Types

- `Audio` — One audio clip (voice memo, VoiceLoid / VoiceVox synthesis
- `ChatMessage` — One chat / dialogue message (Claude Code turn, Slack message,
- `ChatRole` — Role a chat participant played on a specific message.
- `Doc` — One doc / written work product (Markdown note, PDF, spec, code
- `DocFormat` — Format of a `Doc` footprint.
- `Footprint` — The typed shape a parser hands back.
- `FootprintSource` — Reference to the raw source of a footprint.
- `Image` — One image (photo, screenshot, drawing).
- `JournalEntry` — One journal-style entry — short self-authored text with a
- `JournalKind` — Kind of journal-style entry the plugin is emitting.
- `Note` — One short note (mood, idea, quick capture).
- `Tape` — One terminal-session transcript / Persona Tape (`.tape`, `.cast`, `.log`).
- `Video` — One video (recording, screen capture, AI-generated clip).

## Constants

- `COVER_MAX_CHARS` — Maximum cover hint length in Unicode scalar values.
- `REGISTER_MAX_CHARS` — Maximum register-note preview length in Unicode scalar values.

