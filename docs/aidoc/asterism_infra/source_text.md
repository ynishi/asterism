# asterism-infra::source_text

Filesystem adapter for `SourceTextReader` — resolves asset
locators back to the full text of the original artefact.

The DB stores only the 200-char cover snippet; the source of
truth stays on disk (Asterism never writes back to a locator).
Four locator forms are handled, chosen by the extension of the
container path:

- `<path>` — plain text file; the whole content, capped.
- `<path>.jsonl#<uuid>` — one record inside a JSON-lines
  container (Claude Code session logs): the line whose `uuid`
  field equals the fragment. The message body is extracted from
  the common chat-log shapes (`message.content` as a string, or
  as an array of `{type: "text", text}` blocks joined by blank
  lines).
- `<path>.db#<uuid>` — one row in a persona-journal SQLite
  EventLog dump (`entries` + `versions` schema). The current
  version's `body` column is returned. Opened read-only so it
  never collides with the journal-mcp writer.
- `<path>.json#conversation=<conv_id>/message=<msg_id>` — one
  nested message inside a single-envelope harvest JSON
  (`asterism_agent_harvest` v1). The envelope is parsed once,
  then the walk finds `conversations[].id == conv_id` and
  `messages[].id == msg_id`.

Batching: locators are grouped by container path first, so a
200-message session / db / envelope costs exactly one open.
Extraction failures degrade to `None` per locator — the caller
falls back to the stored cover instead of failing the whole
batch.

## Types

- `FsSourceTextReader` — Filesystem-backed reader. Stateless; safe to share.

