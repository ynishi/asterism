# asterism-importer-sdk::catalogue

# Import target catalogue

Field-mapping reference for schemas Asterism plans to import. Every
target is decomposed into per-record [`crate::Footprint`]s sharing
one `session_id`, so [`asterism-core`]'s `edge_rebuild` clusters the
siblings via `time_proximity=1.0` (see
[`crate::footprint::Note::session_id`] and its neighbours). Targets
land here as they are surveyed; unverified fields are marked
**(unverified)** and require a real export sample before an importer
is written against them.

[`asterism-core`]: https://github.com/ynishi/asterism/tree/main/crates/asterism-core

## Design axioms

1. **One character card ≠ one footprint.** A card is decomposed into
   many per-slot footprints — name / description / personality →
   `Note`, first_mes / alternate_greetings → `ChatMessage`, avatar /
   emotions → `Image`, character_book entries → `Note`. This keeps
   the grid UI's "dense, browsable, per-record" character intact.

2. **No persona bootstrap from the importer.** Persona identity
   stays with the existing `Persona` aggregate (created out of
   band); every footprint attaches to a persona id passed via CLI.
   Character metadata (name / description / avatar / expressions) is
   treated as *assets* under that persona, not as a persona seed.

3. **`session_id` groups per-container siblings.** All footprints
   from one card / one chat / one lorebook / one conversation share
   one `session_id` (typically the container's locator). The domain
   then draws `time_proximity=1.0` edges automatically.

4. **Extension namespaces pass through `extra`.** Fields under
   `extensions.chub.*` / `extensions.risuai.*` / `extensions.agnai.*` /
   … are stored verbatim in the footprint's `extra` JSON — the SDK
   does not interpret them; downstream tooling can.

## Locator taxonomy

Common suffixes used across importers. All idempotent, record-level,
stable across re-imports (see
[`crate::footprint::FootprintSource::locator`]).

| suffix | meaning | example target |
|---|---|---|
| `<file>` | container = one footprint | standalone image, single doc |
| `<file>#field=<slot>` | one text slot inside a container | V2 `name` / `description` / `personality` |
| `<file>#greeting=<i>` | array-indexed greeting | `first_mes` / `alternate_greetings[i]` |
| `<file>#book_entry=<uid>` | lorebook entry (id if present, else content hash) | `character_book.entries` / World Info |
| `<file>#L<n>` | line-indexed record (append-heavy source) | SillyTavern chat JSONL |
| `<file>#tree/<msg-id>` | tree-node message | ChatGPT / Claude conversation |
| `<file>!<zip-inner-path>` | ZIP archive inner entry | `.charx` / SillyTavern backup |
| `<file>#chunk=<name>` | PNG tEXt chunk | `chara` / `ccv3` |
| `<file>#hash=<sha1>` | content-hash fallback when no stable id | MemoryPlugin, legacy books |

---

## Character cards

### 1. SillyTavern Character Card V2

**Implemented** — [`crate::card::V2Parser`] covers this section,
with a live integration test against SillyTavern's default
character (`default_Seraphina.png`) in
`tests/seraphina.rs`.

- Spec: [malfoyslastname/character-card-spec-v2](https://github.com/malfoyslastname/character-card-spec-v2/blob/main/spec_v2.md)
- Container: PNG (tEXt `chara`, see section 3) or standalone `.json`.
- Envelope: `{spec: "chara_card_v2", spec_version: "2.0", data: {…}}`.
- `data.*`: `name` / `description` / `personality` / `scenario` /
  `first_mes` / `mes_example` / `creator_notes` / `system_prompt` /
  `post_history_instructions` / `alternate_greetings[]` /
  `character_book` (section 9) / `tags[]` / `creator` /
  `character_version` / `extensions{}`.
- Split:
  - `name` / `description` / `personality` / `scenario` /
    `system_prompt` / `post_history_instructions` → `Note` × 6
    (label = slot name)
  - `first_mes` → `ChatMessage` (`role = Assistant`,
    `thread_position = 0`)
  - `alternate_greetings[i]` → `ChatMessage` × N
    (`thread_position = 1 + i`, `extra.greeting_kind = "alternate"`)
  - `mes_example` → `Doc(Plain)` (or `<START>`-split → many
    `ChatMessage` when the parser recognises the delimiter)
  - `creator_notes` → `Doc(Markdown)`
  - `character_book.entries[]` → `Note` × N (see section 9)
  - `tags[]` → labels (not a footprint)
  - `extensions.*` → verbatim in every footprint's `extra`
- Locator: `<container>#field=<slot>` / `#greeting=<i>` /
  `#book_entry=<id>`.

### 2. Character Card V3 (superset of V2)

**Implemented** — [`crate::card::V3Parser`] extends [`crate::card::V2Parser`]
with `creator_notes_multilingual`, `group_only_greetings`, and
`assets[]`. Unit-tested via schema-based synthetic envelopes in
`src/card/v3.rs`.

- Spec: [kwaroran/character-card-spec-v3](https://github.com/kwaroran/character-card-spec-v3/blob/main/SPEC_V3.md)
- Delta vs V2: `assets[]`, `creator_notes_multilingual{lang}`,
  `group_only_greetings[]` (required), `nickname`, `source[]`,
  `creation_date` / `modification_date` (Unix seconds),
  `character_book.use_regex`, character_book entry `content` may
  embed `@@`-decorators (`@@depth`, `@@role`,
  `@@activate_only_after`, …).
- Container: PNG (tEXt `ccv3`, `chara` fallback) or `.charx` ZIP.
- `.charx` layout:
  ```text
  card.json
  assets/<type>/<category>/<file>
      type     ∈ icon | background | emotion | user_icon | x_<custom>
      category ∈ images | audio | video | l2d | 3d | ai | fonts | code | other
  ```
- Additional split (on top of section 1):
  - `assets[type ∈ icon|background]` → `Image` (`label =
    "asset:<type>"`, `extra.ext = "png"|"webp"|…`)
  - `assets[type=emotion]` → `Image` × N (`label = "emotion:<name>"`)
  - `creator_notes_multilingual{lang}` → `Doc(Markdown)` × N
    (`extra.lang = "<iso-639-1>"`)
  - `group_only_greetings[i]` → `ChatMessage` (`label = "group_only"`)
  - character_book decorators → carried on entry `Note.extra.decorators`
    (parse deferred to downstream)
- Locator: `<charx>!card.json#field=<slot>` /
  `<charx>!assets/<type>/<category>/<file>`; PNG same as V2
  (section 1) but preferring `#chunk=ccv3`.

### 3. PNG tEXt embed

**Implemented** — chunk decoding lives in
[`crate::card::png_chunk`] ([`crate::card::envelope_from_chunk`]
for base64 → envelope, [`crate::card::CHARA_KEYWORD`] /
[`crate::card::CCV3_KEYWORD`] for the canonical keyword strings).

- Chunks: `chara` (V2 base64 UTF-8 JSON), `ccv3` (V3 base64 UTF-8
  JSON). tEXt is Latin-1 → base64 → ASCII wrap is spec-mandatory.
- Read priority: `ccv3` wins, `chara` falls back. Both may coexist
  on one file (V3 emitters often write both for compatibility).
- The PNG bitmap itself becomes an `Image` (label = `avatar`);
  the embed then decomposes per sections 1 / 2.
- Locator: image = `<png>`; embed container = `<png>#chunk=<name>`;
  per-record = `<png>#chunk=<name>/field=<slot>`.

### 4. CharacterHub (chub.ai)

- Base: V2 (section 1); chub is a web host, not a spec author.
- Delta: `data.extensions.chub.*` namespace (observed keys:
  `alt_expressions`, `related_lorebooks`, `expressions`,
  `background_image`, `tags`, `stats` — **literal field list
  unverified**, needs a real card sample to enumerate).
- Container: PNG (default) or `.json`.
- Split: identical to V2; `chub.*` fields pass through in `extra`.
- Locator: identical to V2.

### 5. RisuAI

- Author of V3 spec ([kwaroran/RisuAI](https://github.com/kwaroran/RisuAI)).
- Containers: `.png` (chara or ccv3), `.json`, `.charx` (ZIP), plus
  `.risup` (LLM preset — **not** a card) and `.risum` (regex /
  trigger module — **not** a card, skip on generic import).
- Delta: `data.extensions.risuai.*` namespace: `emotions`,
  `additionalAssets`, `customScripts`, `bias`, `viewScreen`,
  `lowLevelAccess` (**literal shape unverified** — source
  `src/ts/*` read required).
- Split: V3 (section 2). Emotions under `assets/emotion/*` → `Image` × N.
- Locator: V3 (section 2).

### 6. AgnAI

- Schema (verbatim TypeScript): [agnaistic/agnai `common/types/library.ts`](https://github.com/agnaistic/agnai/blob/dev/common/types/library.ts)
- Container: `.json` (also imports/exports V2 PNG chunks).
- Delta vs V2: `persona: Persona` object with `kind ∈
  wpp/sbf/boostyle/text/attributes` replaces flat `personality`;
  `sprite: FullSprite` (2D avatar composition); `culture`,
  `insert{depth,prompt}`, `voice`, `imageSettings`, `json:
  ResponseSchema` are agnai-only.
- Split:
  - V2 core (section 1)
  - `persona.attributes[k]` (when `kind=attributes`) → `Note` × N
    (`label = "persona:<k>"`)
  - `sprite` → `Note` (`label = "sprite-composition"`, JSON in
    `body`); referenced part images → `Image` × N
  - `avatar` (base64 or URL) → `Image`
- Locator: `<json>#field=persona.attribute.<k>` /
  `#field=sprite` / `#greeting=<i>` / `#book_entry=<i>`.

### 7. KoboldAI Lite preset / story save

- Source (no formal spec): [LostRuins/lite.koboldai.net](https://github.com/LostRuins/lite.koboldai.net)
- Character preset (V1 subset): `name`, `description`,
  `personality`, `scenario`, `first_message` (aliases: `first_mes` /
  `greeting` — **which alias is canonical is unverified**, parse
  tolerantly).
- Story save embeds `memory` / `authorsnote` / `worldinfo[]` /
  `actions[]` / `savedsettings`.
- Split:
  - Character preset: `Note` × 4 (name → title label) + `ChatMessage`
    × 1 (first_message)
  - `memory` / `authorsnote` → `Note` × 2 (labels = `memory`,
    `authors-note`)
  - `worldinfo[]` → `Note` × N (see section 9)
  - `actions[]` → `ChatMessage` × N (`thread_position = index`,
    `role` via `is_user`) — **the heaviest path in this crate; one
    story = N chat messages**
- Locator: `<json>#field=<slot>` / `#wi=<uid>` / `#action=<i>`.

---

## Chat logs

### 8. SillyTavern chat JSONL

- Container: `.jsonl` at
  `~/SillyTavern/data/default-user/chats/<char>/<YYYY-MM-DD-HHmmss>.jsonl`
  (host-side layout; group chats live under `.../group chats/`).
  Append-heavy — use [`crate::ScanMode::Watch`] with per-record
  locators.
- Line 1 = header (`user_name`, `character_name`, `create_date`,
  `chat_metadata`).
- Line N (N ≥ 2) = message (`name`, `is_user`, `send_date`, `mes`,
  `swipes[]`, `swipe_id`, `swipes_info[]`, `extra{model, api,
  token_count}`).
- **Date parsing quirk**: `send_date` is typically a human-readable
  English string (e.g. `"June 22, 2024 3:14pm"`), not ISO 8601 or
  epoch. Some builds emit epoch ms. Parse tolerantly; on failure
  fall back to [`crate::RawItem::occurred_at`].
- Split:
  - Header → drop; use as `session_id` seed + baseline
    `occurred_at`.
  - Message → `ChatMessage` (`role` via `is_user` / `is_system` →
    `User` / `System` / `Assistant`; `extra` carries model / api /
    token_count).
  - `swipes[]` alternatives: default = fold into `extra`; per-record
    option = emit each alt as its own `ChatMessage` with `label =
    "swipe:<i>"` (aligns with the "dense, browsable" axiom).
- Locator: `<file>#L<n>` (line-indexed, safe under append). No
  intrinsic message id in the format.

### 9. SillyTavern World Info / lorebook

- Container: `.json` at
  `~/SillyTavern/data/default-user/worlds/<name>.json`, or embedded
  in V2/V3 `data.character_book`.
- **Asymmetric shape**: standalone = `entries: { "<uid>": {…} }`
  (dict, keyed by uid); embedded (V2) = `entries: [ {…} ]` (array).
  Parsers must handle both.
- Entry field (standalone superset): `uid`, `key[]`,
  `keysecondary[]`, `comment`, `content`, `constant`, `selective`,
  `selectiveLogic`, `order`, `position`, `disable`, `probability`,
  `depth`, `group`, `role` (0=system / 1=user / 2=assistant),
  `scanDepth`, `caseSensitive`, `matchWholeWords`, `automationId`,
  `vectorized`, `sticky` / `cooldown` / `delay`, `displayIndex`,
  `characterFilter`, `excludeRecursion` / `preventRecursion` /
  `delayUntilRecursion`.
- Split: each entry → `Note` (`body = content`, `labels = ["lore"]
  + key[]`, `session_id` = book locator, remaining fields → `extra`).
  Long `content` (> ~3 paragraphs) may escalate to `Doc(Markdown)`
    at parser discretion.
- Locator: standalone = `<file>#entry=<uid>`; embedded =
  `<container>#book_entry=<uid-or-content-sha1[:8]>` (V2 array shape
  often lacks `uid`, fall back to a content hash).

### 10. NovelAI Lorebook

- Doc: [NovelAI docs — Lorebook](https://docs.novelai.net/en/text/lorebook/)
  (no formal JSON schema — field list is doc + community reverse).
- Container: `.lorebook` or `.json` (identical body).
- Fields: `lorebookVersion` (**current value unverified**),
  `entries[]` (`text`, `keys[]`, `contextConfig{suffix, tokenBudget,
  budgetPriority, trimDirection, insertionType, insertionPosition,
  maximumTrimType, reservedTokens}`, `searchRange`, `enabled`,
  `forceActivation`, `keyRelative`, `categories[]`, `displayName`,
  `id`, `lastUpdatedAt`, `phraseBiasGroups[]`), `categories[]`,
  `settings{}`, top-level `phraseBiasGroups[]`.
- **Not V2-compatible** — needs its own parser. Community
  converters emit V2 `character_book` shape when needed.
- Split: each entry → `Note` (`body = text`, `labels =
  ["lorebook", "novelai"] + keys`, `session_id = <file>`, `keys /
  categories / contextConfig` → `extra`).
- Locator: `<file>#entry=<entry.id>` (id is stable).

### 11. ChatGPT export

- Path: Settings → Data controls → Export data → emailed zip.
- Zip contents: `conversations.json`, `chat.html`, `user.json`,
  `message_feedback.json`, `model_comparisons.json`, (recent —
  unverified) `shared_conversations.json`,
  `dalle-generations/<uuid>.png`.
- `conversations.json` = array of `{id, title, create_time,
  update_time, mapping: {<node-id>: Node}}` where each `Node =
  {id, message, parent, children[]}` and each `Message = {id,
  author.role (user/assistant/system/tool), content.{content_type,
  parts[]}, create_time, metadata.model_slug, status, recipient}`.
- Split:
  - Message → `ChatMessage` (`session_id = <conversation.id>`,
    `role` via `author.role`, `body = parts.join("\n\n")`,
    `parent_message_id = node.parent`, `thread_position` = BFS
    index over `mapping`)
  - Multimodal `parts[]` dict entries with `asset_pointer` →
    sibling `Image` (same `session_id`; `extra.asset_pointer =
    "…"`)
  - `dalle-generations/*.png` → `Image` (cross-link via
    `asset_pointer` when resolvable)
  - `user.json` / feedback / comparisons → not emitted; feedback
    merges into corresponding message `extra`
- Locator: `<zip>!conversations.json#tree/<conversation.id>/<message.id>`
  / `<zip>!dalle-generations/<filename>`.

### 12. Claude data export

- Path: Settings → Privacy → Data controls → Export data.
- Zip contents: `conversations.json` (often 100 MB+), `users.json`,
  `projects.json`, `memories.json` (README / index varies).
- `Conversation`: `{uuid, name, summary, created_at, updated_at,
  account.uuid, chat_messages[]}`.
- `chat_messages[]`: `{uuid, sender ∈ "human"|"assistant", text,
  created_at, updated_at, content (structured — tool_use blocks,
  **shape unverified**), attachments[], files[]}` (attachments and
  files are **distinct** arrays).
- `projects.json`: `{uuid, name, description, prompt_template,
  docs[], creator}`.
- `memories.json`: `{conversations_memory[], project_memories[]}`
  (element shape **unverified**).
- Split:
  - `chat_messages[]` → `ChatMessage` (`session_id =
    conversation.uuid`, role: `sender == "human"` → `User`,
    `"assistant"` → `Assistant`; `body = text` — or reassemble from
    `content` when `text` is empty; `thread_position = index`)
  - Conversation summary → `Doc(Other("claude_conversation"))`
    (same `session_id`, `title = name`, `excerpt = summary`)
  - Each project → `Doc(Other("claude_project"))`; `docs[]` inside
    → `Doc` × N (session_id shared with project uuid)
  - `memories.json.conversations_memory[]` /
    `project_memories[]` → `JournalEntry{kind = Memory}` × N
    (Anthropic's "memory" surface aligns directly with this
    variant's semantics)
  - `attachments[]` / `files[]`: images → `Image`; docs → `Doc`;
    `session_id` shared with parent conversation so
    `edge_rebuild` links siblings
  - `users.json` → **not emitted** (persona bootstrap is out of
    scope; see axiom 2)
- Locator:
  `<zip>!conversations.json#tree/<conv.uuid>/<msg.uuid>` /
  `<zip>!projects.json#project=<uuid>[/docs/<id-or-index>]` /
  `<zip>!memories.json#conversations_memory/<id-or-content-sha1[:8]>`.

---

## AI-native memory

### 13. Letta (formerly MemGPT)

- Repo: [letta-ai/letta](https://github.com/letta-ai/letta). Schema
  SoT: `letta/schemas/{memory,agent}.py` + `letta/orm/message.py`.
- Native storage: SQLAlchemy over SQLite (dev) / Postgres (prod).
  **No first-party zip export** — practical export paths: (a)
  Python SDK / CLI per-agent JSON dump, (b) SQLite file copy, (c)
  `pg_dump`.
- Core memory: `Memory = {agent_type, git_enabled, blocks[Block],
  file_blocks[FileBlock], prompt_template}`. `ChatMemory`
  (BasicBlockMemory) preloads `persona` and `human` blocks with
  `limit = CORE_MEMORY_BLOCK_CHAR_LIMIT`.
- Message ORM row: `{id, role, text, content[MessageContent],
  model, name, tool_calls, tool_call_id, step_id, run_id, otid,
  tool_returns, group_id, sender_id, conversation_id, is_err,
  sequence_id, approval_request_id, batch_item_id}`. Linear (no
  `parent` column).
- Archival: `{id, timestamp, content, tags[]}` on read;
  `{text, tags, created_at}` on create.
- Split:
  - Block (persona / human / custom) → `Note` (`body = value`,
    `label = <block.label>`); use
    `Doc` (`format = Markdown`, `labels = ["handoff"]`) when the semantic is
    carry-across-agents rather than static persona text
  - Archival passage → `JournalEntry{kind = Memory}`
  - Message row → `ChatMessage` (`session_id = conversation_id`,
    `thread_position = sequence_id`; `parent_message_id = None`
    because Letta's chat is linear)
  - Sources / file_blocks → `Doc`
- Locator: `<export>#agents/<agent_id>/blocks/<label>` /
  `#agents/<agent_id>/archival/<passage_id>` /
  `#agents/<agent_id>/messages/<message.id>`. When `id` is absent
  the message has no address and is dropped — do **not** fall back
  to `messages/seq/<sequence_id>`. A sequence number describes the
  export's contents at one moment, so an export that gains a
  message ahead of it re-addresses everything behind onto its
  neighbour (see [`crate::parser`]).

### 14. MemoryPlugin

- API: [help.memoryplugin.com/api-reference](https://help.memoryplugin.com/api-reference/introduction);
  OpenAPI at `https://www.memoryplugin.com/openapi.json`.
- Native storage: SaaS, server-side, not exposed.
- Memory shape (OpenAPI verbatim): `{text, score,
  metadata.bucketId}`. Create input: `{text, bucketId, source}`.
  **Record `id` / `created_at` are not exposed in the OpenAPI**
  response schemas — may exist server-side (unverified).
- Export: no dedicated `/export` endpoint; the browser extension
  emits a single JSON blob (shape **unverified**, typical:
  `{memories[], buckets[]}`).
- Split:
  - Memory item → `JournalEntry{kind = Memory}` (`body = text`,
    `session_id = str(bucketId)`, `labels = [source]`)
  - Bucket → not emitted; used only as `session_id` grouping
    (bucket name / description → `extra`)
  - `POST /api/chat-history/inject` payloads → out of scope
    (write path, not user data)
- Locator:
  `<export.json>#memory/hash=<sha256(text || bucketId || source)>`
  until a real record id is confirmed.

---

## Whole-account backups

### 15. SillyTavern backup zip

- Container: 7z (default from SillyTavern-Launcher) or zip of the
  whole `data/<user>/` tree. Inner layout:
  `characters/` (sections 1 / 2 / 3), `chats/` (section 8),
  `worldinfo/` (section 9), `groups/`, `group chats/`,
  `backgrounds/`, `user_avatars/`,
  `themes/`, `presets/`, `settings.json`, `backups/`.
- Split: each inner file walks its own catalogue entry
  (`characters/*` → sections 1 / 2 / 3; `chats/*.jsonl` → section 8;
  `worldinfo/*.json` → section 9). `backgrounds/` / `user_avatars/` /
  `themes/` / `presets/` / `settings.json` are config assets —
  skip (importing them would be noise).
- Locator prefix: `<backup.7z>!<inner-path>`; the inner path then
  uses the per-format suffix from the target's own section.

---

## Media (container-scanned)

### 17. Video (MP4 / MOV / WebM)

**Implemented** — [`crate::Video`] variant + `asterism-import video`
subcommand. Two probe layers: `mp4parse` (Mozilla, pure Rust) for
ISOBMFF containers (MP4 / MOV), then `matroska` (pure Rust) for
EBML containers (WebM / MKV).

- Extraction: dims (from `tkhd` / Matroska pixel dims), duration_ms
  (from track duration × timescale / segment info), codec (H.264 /
  H.265 / AV1 / VP9 / MP4V / …), from the first video track when
  present.
- Extensions: `.mp4` / `.mov` / `.webm` / `.m4v` / `.mkv` / `.avi`
  accepted by default (`.avi` lands container-only — neither probe
  reads RIFF).
- Fixtures verified: `minimal.mp4` (Mozilla), `animation.mov`
  (openpreserve, 320×240 25fps 1s), `bbb_360_10s.webm` (Big Buck
  Bunny, CC-BY).
- Modality slug: `"video"`. Codec surfaces as `codec:<slug>` label.

### 18. Audio (MP3 / M4A / WAV / FLAC / OGG)

**Implemented** — [`crate::Audio`] variant + `asterism-import audio`
subcommand. Metadata via `lofty` (pure Rust,
header-only). Covers voice memos, VoiceLoid / VoiceVox synthesis
output, podcasts, music, dictation.

- Extraction: duration_ms, sample_rate, channels, codec (mp3 /
  aac / pcm / flac / vorbis / opus / speex / …).
- Extensions: `.mp3` / `.m4a` / `.wav` / `.flac` / `.ogg` /
  `.opus` / `.oga` / `.aac` / `.aiff` accepted by default.
- Modality slug: `"audio"`. `voice` / `voice-synth` / `music` /
  `podcast` etc. flow as free-form labels for grid facet.

---

## Agent-harvest intake

### 16. `asterism_agent_harvest` canonical schema

**Implemented** — see [`crate::harvest`] for the schema types
([`crate::harvest::HarvestEnvelope`]), the parser
([`crate::harvest::HarvestSourceParser`]), and the prompt-ready
example ([`crate::harvest::schema_example_json`]). Exposed through
the `asterism-import harvest` subcommand.

- Covers: closed cloud services with **no official export** —
  Character.AI, Kindroid, Replika, Grok, x.com bot chats — plus
  any future intake whose native format is not worth carving a
  dedicated Rust parser for.
- Container: `.json` files in a landing dir. Watch-mode friendly.
- Envelope: `{spec: "asterism_agent_harvest", spec_version, service,
  service_user_id?, harvested_at?, characters[]?, conversations[],
  extra}`.
- Split: `conversations[].messages[]` → `ChatMessage` × N
  (`session_id = uuid5(<file>#<conv.id>)`; `role` via
  `ChatRole::User/Assistant/System/Tool/Other`; `thread_position`
  = message index; `parent_message_id = message.parent_id`;
  labels = `["agent-harvest", service, role, "chat:<title>"?]`).
  `characters[]` is preserved in each message's `extra.character`
  but not emitted as its own footprint in v1 (character metadata
  should route through sections 1 / 2 when possible).
- Intake pipeline:
  ```text
  raw service dump  →  Claude Code / Codex writes converter
  (browser scrape,      (prompted with the schema example above)
   GDPR export,     →  canonical JSON in ~/.asterism/inbox/harvest/
   DevTools grab)   →  asterism-importer-harvest picks it up
                    →  Footprint::ChatMessage × N in DB
  ```
- Locator: `<file>#conversation=<conv.id>/message=<msg.id-or-idx>`.

---

## Cross-cutting conventions

- **`session_id` sharing.** Every footprint decomposed from the
  same container (card, chat, book, conversation, agent) shares
  one `session_id` string — typically the container's own locator.
  `edge_rebuild` then draws `time_proximity=1.0` edges across the
  sibling set (see [`crate::footprint::Note::session_id`],
  [`crate::footprint::Image::session_id`], and
  [`crate::footprint::JournalEntry::session_id`]).
- **Extension namespaces pass through.** Any
  `data.extensions.<vendor>.*` sub-tree is stored verbatim in the
  footprint's `extra` JSON; the SDK does not interpret it, so
  third-party tools can query the raw shape later.
- **Tolerant date parsing.** Chat exports do not agree on
  `send_date` / `create_time` / `timestamp` format — ISO 8601,
  Unix seconds, Unix milliseconds, and English human strings show
  up side by side. Try each in fidelity order (payload → RawItem
  `mtime` → `Utc::now()`), per the
  [`crate::parser::SourceParser`] ladder.
- **Unverified fields.** Anything marked **(unverified)** above
  needs a real export sample before an importer commits to a
  struct field for it. Until confirmed, prefer `extra`-bag
  passthrough over hard-coded fields — cheap now, cheap to promote
  later once the shape is nailed down.

---

## Experimental / future intake

Designs not yet implemented. Sketched here so the SDK contract
shape is nailed down before the first sample arrives — the moment
a real intake source lands, the variant / adapter can go in
without redesigning the surrounding schema.

### E1. LoRA / adapter weights (`Footprint::Weights`, planned)

LoRA fine-tunes, ControlNet adapters, DreamBooth checkpoints,
IPAdapter models — the "distilled identity" surface for a
persona (my-face LoRA, my-writing-style adapter, this-character's
voice model, …). Typical files: `.safetensors` / `.gguf` /
`.pt` / `.onnx`, 10–500 MB per file.

- New variant: `Footprint::Weights` with typed fields
  `base_model: Option<String>` (e.g. `"sdxl-1.0"` / `"llama-3-8b"`),
  `format: WeightsFormat` (Safetensors / Gguf / Pt / Onnx / Other),
  `parameter_count: Option<u64>`, `training_config: Value` (extra
  bag for LR / rank / trigger words / etc.).
- Modality slug: `"weights"`. Labels: `["lora"]` / `["adapter"]` /
  `["dreambooth"]` / `["controlnet"]` for grid facet.
- Container is one file = one footprint (weights are indivisible).
- Adapters intake: dedicated `asterism-importer-weights` binary
  walking `~/Downloads/` / `~/models/` / civitai export dirs, or
  folded into an existing `fs`-scanner path with extension filter.
- Cover: LoRA preview image when present (many `.safetensors`
  have a sibling `.png`) → sibling `Footprint::Image` with same
  `session_id`.

### E2. RAG embeddings (server-side companion table, planned)

High-dim float embeddings over the corpus (semantic search
backbone). Typical scale: 384–1536-dim float32 × millions of
rows — one embedding per existing asset, per embedding model.

**Not a footprint variant.** Embeddings are derived data over
existing assets, so the natural shape is a **server-side
companion table** (`asset_embedding` in `asterism-core`) with
`(asset_id, embedding_model, vector BLOB)` rows and an ANN index
(HNSW / IVF). One asset gets N rows (one per model). The SDK
contract stays modality-agnostic; embeddings are an infra layer
the domain owns.

Exception: if a user wants to *import* a pre-computed embeddings
dump (research project export, historical FAISS index), it lands
via `Footprint::Weights { format: WeightsFormat::Other("faiss"), … }`
and a downstream server-side job re-indexes into the companion
table.

### E3. Interactive artifacts (Claude Artifacts / ChatGPT Canvas / GitHub Gist)

**Already covered by [`crate::DocFormat`]** — no new variant needed.
Artifacts map onto `Doc(Html)` / `Doc(Code(lang))` /
`Doc(Markdown)` depending on payload shape; origin service
(`"claude-artifact"` / `"chatgpt-canvas"` / `"gist"` /
`"observable"`) flows via `Doc.labels` for facet. Pending: a
dedicated harvester Skill (browser automation + share-URL grab)
that dumps into the section 16 agent-harvest schema for pickup.

### E4. Terminal captures — Persona Tape lands, generic adapter pending

Persona Tape transcripts are covered by `asterism-import tape`: one
`.txt` terminal session becomes one `Tape` footprint (`Modality::TAPE`) with
the original file retained as the searchable source. The broader
terminal adapter (raw `.log`, VHS `.tape`, and asciinema `.cast`,
including ANSI stripping) is still pending.

### E5. "PersonaVaultApp" completion

With Weights (E1) + Embeddings (E2) landing, one persona's grid
holds every trace the AI needs to *exist* as that character:
character card definition, chat history, voice / video / photo
identity, LoRA / adapter distillation, and a searchable
embedding backbone. The current 7-variant SDK contract
(`ChatMessage` / `Doc` / `Note` / `JournalEntry` / `Image` /
`Video` / `Audio`) covers the observable side; Weights closes
the model-side gap.

### E6. Screenshot pipeline (universal fallback)

Screenshots are the ultimate "any source works" capture path —
when a service has no official export (Char.AI, Kindroid,
Sora, Kling, Grok, private chats, closed apps), the user can
always screenshot the surface and preserve it. Digital
Alma-style persona reconstruction ("gachi de Persona-chan no
footprint") leans heavily on this fallback.

- **Already covered by `asterism-import image`** — no new
  variant needed. Screenshots are `Footprint::Image` with EXIF
  / dims / thumb the same as any photo.
- **Watch-mode integration is the missing UX piece.** Point
  `asterism-import image --watch --dir ~/Desktop` (macOS
  default screenshot dir) or `~/Pictures/Screenshots` (Windows) /
  iCloud Photos Screenshots album at the FS scanner and every
  `Cmd+Shift+4` shot lands in the grid within seconds. A tiny
  launchd plist / systemd unit / shortcut wrapper is enough —
  no code change required.
- **Screenshot-specific labels.** iOS screenshots carry
  `mediasubtype = 4` (SPSS) in EXIF; macOS shots write
  `Software = "Screenshot"`. The image parser can auto-add a
  `"screenshot"` label from these hints so the grid facet is
  free.
- **Follow-up cover_gen jobs** (server side, not SDK): OCR
  the screenshot for text search + VLM caption for semantic
  search. Recovers the "what was on this screen" query even
  when the source service is gone.

### E7. External asset IDs / canonical service references

AI companion services (Kindroid persona id, Character.AI
character UUID, Suno song id, HeyGen avatar id, HuggingFace
model handle, civitai LoRA id, …) all expose a canonical
reference the user wants to preserve alongside the footprint.

- **Already covered by existing fields** — no new variant:
  - `FootprintSource.locator` = canonical URL / id
    (`https://beta.character.ai/character/<uuid>` etc.)
  - `FootprintSource.platform` = origin service slug
    (`"character.ai"` / `"kindroid"` / `"suno"` / …)
  - `extra.external_ids` = free-form JSON bag for cross-service
    joins (`{"character_ai_uuid": "...", "suno_song_id": "..."}`)
- **Future promotion**: if cross-service dedup / join becomes a
  hot query, promote to a first-class
  `FootprintSource.external_id: Option<String>` field.

