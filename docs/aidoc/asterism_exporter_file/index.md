# asterism-exporter-file 0.0.0

# asterism-exporter-file

Generic filesystem exporter. Takes a Selection of already-ingested
Assets, copies (or symlinks, or just references) each input into a
caller-supplied output directory, and emits one [`Derived`] per
written entry.

Position in the workspace:

- Mirrors the `asterism-importer-image` / `-video` / `-audio`
  position on the OUT side — a concrete, name-shaped exporter
  registered next to `comfy` in the server's [`ExporterRegistry`].
- Unlike the importers (subprocess binaries that POST to the
  server) this crate is a **library** because exporters run
  in-process inside the apalis `DispatchRun` worker.

## When to use

- Physically duplicate every asset in a Selection into a
  pre-configured drop folder — e.g. the Comfy `input/` mount, a
  Photoshop watch folder, a portable archive dir.
- Materialise a **read-only reference** to each Selection member
  as a new derived Asset (`mode = "reference"`), so a Selection
  can be turned into a persistent, linkable subset without moving
  any bytes.

## Params schema

`CreateDispatchCommand.params_json` deserialises into
[`FileDispatchParams`]:

```json
{
  "output_dir": "/absolute/path",
  "mode": "copy" | "symlink" | "reference" | "instruction",
  "filename_template": "{{basename}}",
  "modality": "image",
  "labels": ["archive"],
  "instruction": { "workflow": "wf-1", "prompt": "..." }
}
```

- `output_dir` — the directory the exporter writes into. Created
  recursively when it does not exist.
- `mode` — how the input is materialised on disk:
  - `copy` — physical copy (owns the bytes, safe against source
    deletion).
  - `symlink` — creates a symlink pointing at the source. Cheaper
    than copy; breaks if the source moves.
  - `reference` — nothing is written; the Derived's `locator`
    points at the original source. Useful for "make a subset
    Selection into a listable dispatch history" without touching
    the filesystem.
  - `instruction` — writes a single dispatch-scoped JSON file to
    `output_dir` that embeds the caller-supplied `instruction`
    blob plus the input locator list. Intended for fs-mediated
    handoff to an external receiver (e.g. a Comfy watch-folder
    plugin) that runs the workflow and drops results into a
    directory an Importer is watching. Emits exactly one Derived
    (`modality = "instruction"` by default) pointing at the
    written file. `filename_template` defaults to
    `"{{dispatch_id}}.json"` in this mode.
- `filename_template` — filename shape under `output_dir`.
  Supports these placeholders (simple text substitution, no
  arithmetic):
  - `{{basename}}` — original filename (base name including
    extension). Default when the field is omitted.
  - `{{stem}}` — original filename without extension.
  - `{{ext}}` — original extension (without the dot).
  - `{{index}}` — 0-indexed position within the Selection.
  - `{{selection_id}}` / `{{dispatch_id}}` / `{{persona_id}}` —
    ids from the dispatch context.
  - `{{action}}` — the exporter action slug.
    Collisions (two input assets that would produce the same output
    path) are broken by appending `-<index>` to the stem before the
    extension.
- `modality` — modality slug written on each Derived. Optional; if
  omitted the exporter passes the input's modality through
  verbatim so the derived Asset lands in the same grid lane as
  its source.
- `labels` — extra labels appended to each Derived (in addition
  to the exporter/action labels the core prepends).

## Lifecycle

Filesystem writes are synchronous; the exporter does all the work
inside [`dispatch`](FileExporter::dispatch) and stashes the
produced [`Derived`] list on the returned [`Handle`]'s payload.
`poll` always returns [`DispatchState::Done`] immediately;
`harvest` just deserialises the cached list. This keeps the
runner's state machine identical to the network-backed exporters
without introducing a fake "waiting" phase.

