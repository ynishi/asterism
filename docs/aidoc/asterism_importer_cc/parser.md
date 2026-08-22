# asterism-importer-cc::parser

Claude Code JSONL → `Footprint::ChatMessage` (+ `Footprint::Image`)
parser.

Each `*.jsonl` file under `~/.claude/projects/<slug>/` is one Claude
Code session. We emit one `Footprint::ChatMessage` per `user` /
`assistant` line with extractable text content. Non-text records
(mode, file-history-snapshot, tool_use / tool_result blocks with no
text) are skipped.

Images that entered the conversation (asset-model v4 P4): Claude
Code renders a pasted / referenced image into the message text as a
`[Image: source: <absolute path>]` marker — pasted screenshots get
a durable copy under `~/.claude/image-cache/<session>/`, file
references keep their original path. Each unique marker path emits
one `Footprint::Image` carrying the **same `external_session_key`**
as the messages, so the image lands as a member of the same Session
composite (membership is modality-agnostic). The base64 content
blocks in the same lines are ignored — they have no durable locator
of their own and the marker already points at the on-disk truth.

The `FootprintSource.locator` is `<file-path>#<line-uuid>` so
re-imports collapse to no-ops via the server-side
`asset.source_kind + source_locator` unique constraint. Image
footprints use the marker path itself as the locator — the same
constraint therefore assigns an image pasted into several sessions
to the first session imported (one asset per original file).

A line that carries no `uuid` is **not** given one made from its
line number: it has no address, so no `Footprint::ChatMessage` is
emitted for it and the drop is counted
([`asterism_importer_sdk::RecordAddresses`], which states the rule
and reports the count). An image marker on such a line still
becomes a `Footprint::Image` — its locator is the marker path, an
address the source really does state, and dropping the whole line
would take the picture down with the missing id.

## Types

- `CcSessionParser` — Parser for Claude Code session JSONL logs.

