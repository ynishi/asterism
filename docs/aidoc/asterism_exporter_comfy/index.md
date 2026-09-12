# asterism-exporter-comfy 0.1.1

# asterism-exporter-comfy

Asterism `Exporter` against a running ComfyUI — one dispatch goes
upload → prompt → poll → fetch → reify without anybody touching
ComfyUI's filesystem by hand.

- Implements [`asterism_dispatch_sdk::Exporter`] over the ComfyUI
  HTTP API: `POST /upload/image` for the inputs, `POST /prompt` to
  queue the graph, `GET /history/{prompt_id}` to wait, `GET /view`
  to read the produced files back out.
- The Comfy workflow JSON is passed through `params.workflow` in
  the API format the frontend's *Export (API)* writes. Before it is
  submitted it is rendered through the shared `{{...}}` template
  ([`asterism_exporter_common::template`]), so a prompt, a seed or a
  batch count is a placeholder in the graph and a value in the
  params rather than a literal to edit in the node.
- Every input the graph names lands in ComfyUI's `input/` through
  the upload route, under a subfolder of our own, and the name the
  backend answers with is what the `LoadImage` node is given. Stock
  ComfyUI refuses a path outside `input/`, so this is the only way
  an image gets in.
- Every produced file is fetched and written under the profile's
  custody root ([`asterism_exporter_common::CustodyPaths`]), and
  that path is the reified asset's locator. ComfyUI's own `output/`
  is not a place a locator can point: `temp/` is wiped on restart,
  file names are a counter that is reused once a file is deleted,
  and where the directory is at all is a flag on ComfyUI's command
  line that this process cannot see.

Deferred: WebSocket progress, a saved-workflow registry, fanning a
snapshot's members through one graph each. The `Exporter` trait
leaves room for all three without touching this crate's surface.

## Params contract

`CreateDispatchCommand.params_json` for this exporter deserialises
into [`ComfyDispatchParams`]:

```json
{
  "endpoint": "http://127.0.0.1:8188",
  "workflow": { /* ComfyUI prompt graph, API format */ },
  "input_slot": { "10": 0 },
  "prompt": "golden hour, same person",
  "count": 4,
  "poll_interval_ms": 2000
}
```

- `endpoint` — Comfy base URL (no trailing slash).
- `workflow` — the graph as the Comfy frontend would submit it. Any
  string leaf may be a template: `{{params.prompt}}`,
  `{{params.count}}`, `{{input[0].cover}}`, `{{dispatch_id}}`. A
  leaf that is *one* placeholder and nothing else keeps the type of
  the value it names, so `"amount": "{{params.count}}"` reaches
  the backend as the integer `4`.
- `input_slot` — which nodes take an image from the snapshot, as
  `{ node_id: input_index }`; the index is the position in the
  snapshot's member list. A bare `"10"` still means `{ "10": 0 }`,
  which is what every stored params blob says. Optional: a txt2img
  graph names no node and uploads nothing.
- `seed` — read by `{{params.seed?}}`; left out, a random one is
  drawn per dispatch and written into the params the template sees,
  so a re-dispatch is a new sample and the attempt record shows
  which one.
- `poll_interval_ms` — how often the runner will poll; the value is
  echoed back into the progress hint so the UI can display a
  correct spinner cadence.

Anything else in the blob is the caller's — `{{params.<key>}}`
reaches it. That is the same rule the http adapter has.

## What the produced PNG carries

The submit sets `extra_data.extra_pnginfo.asterism` to the dispatch
id and the prompt id, and ComfyUI writes every key of
`extra_pnginfo` as a tEXt chunk of that name beside its own
`prompt`. A file that reaches the library by some other route —
dragged out of ComfyUI's output directory — still says which
dispatch made it.

