# asterism-exporter-comfy 0.0.0

# asterism-exporter-comfy

First-slice Asterism `Exporter` — dispatches jobs to a running
ComfyUI HTTP backend and harvests generated images.

Scope of this crate at first-slice:

- Implements the [`asterism_dispatch_sdk::Exporter`] trait against
  the ComfyUI HTTP prompt-queue API (`POST /prompt`, `GET
  /history/{prompt_id}`, output files served under `/view`).
- Supports the single action `"img2img"`. The Comfy workflow JSON
  is passed through `params.workflow` verbatim; the exporter's
  only job is to substitute the input image references and read
  the produced files back out.
- Output images land under a caller-configurable dir (`params.output_dir`
  defaults to `$ASTERISM_HOME/dispatch/<dispatch_id>/`) so the reified
  `Asset` locator points at a stable on-disk path.

Everything more ambitious (workflow template registry, txt2img /
upscale actions, streaming previews, WebSocket progress) is
deferred — the `Exporter` trait leaves room for those without
touching this crate's surface.

## Params contract

`CreateDispatchCommand.params_json` for this exporter deserialises
into [`ComfyDispatchParams`]:

```json
{
  "endpoint": "http://127.0.0.1:8188",
  "workflow": { /* ComfyUI prompt graph JSON */ },
  "output_dir": "/optional/absolute/path",
  "input_slot": "load_image_node_id",
  "poll_interval_ms": 2000
}
```

- `endpoint` — Comfy base URL (no trailing slash).
- `workflow` — the exact prompt graph the Comfy UI would submit.
  The exporter walks it looking for `input_slot` and rewrites that
  node's `image` field to the Selection's first input.
- `output_dir` — absolute directory to write the harvested files
  to. `None` = fall back to `$ASTERISM_HOME/dispatch/<id>/` on the
  caller side.
- `input_slot` — the id of the workflow node whose `image` input
  should be substituted with the Selection's first asset locator.
- `poll_interval_ms` — how often the runner will poll; the value
  is echoed back into the progress hint so the UI can display a
  correct spinner cadence.

