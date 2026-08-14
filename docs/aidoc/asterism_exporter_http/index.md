# asterism-exporter-http 0.0.0

# asterism-exporter-http

Schema-driven HTTP exporter. Where [`asterism_exporter_comfy`] hard-
codes the ComfyUI protocol, this crate stays backend-agnostic:
**the caller supplies the request / poll / harvest shapes as JSON
schema in the dispatch params**. One deployable adapter, N backends.

Position in the workspace: mirror of `asterism-importer-sqlite`
("query + column map = whole importer") on the OUT side. Where the
SQLite importer's caller writes SQL + a column mapping, this
exporter's caller writes an HTTP shape + a JSON-path mapping.

## Params schema

`CreateDispatchCommand.params_json` deserialises into
[`HttpDispatchParams`]. All three phases (`dispatch` /
`poll` / `harvest`) are configured up front so the runner can
drive the state machine without re-reading params on every tick.

```json
{
  "endpoint": "http://backend.example.com",

  "dispatch": {
    "method": "POST",
    "path": "/generate",
    "headers": { "authorization": "Bearer {{params.extras.api_key}}" },
    "body_template": {
      "input_url": "{{input[0].source_locator}}",
      "prompt":    "{{params.extras.prompt}}",
      "client_id": "{{dispatch_id}}"
    },
    "handle_from": "$.job_id"
  },

  "poll": {
    "method": "GET",
    "path":   "/status/{{handle}}",
    "done_when":   { "path": "$.status", "equals": "done" },
    "failed_when": { "path": "$.status", "equals": "failed",
                      "message_path": "$.error" },
    "progress_message_path": "$.status_message"
  },

  "harvest": {
    "method": "GET",
    "path": "/result/{{handle}}",
    "items_path": "$.outputs[*]",
    "map": {
      "modality":      "image",
      "locator":       "{{item.url}}",
      "cover_hint":    "{{item.caption?}}",
      "labels_static": ["batch:{{dispatch_id}}"]
    }
  },

  "extras": {
    "api_key": "put-your-token-here",
    "prompt":  "photo studio portrait"
  }
}
```

`extras` is not a field the exporter knows about — the params blob
is its own template namespace, so a caller nests its per-backend
values anywhere it likes and reaches them with
`{{params.<dot.path>}}`. `schema/http_params.example.json` is the
runnable version of this same shape (it is what `asterism-server
schema print exporter:http:params` streams), and the tests at the
bottom of this file are what keep it honest.

Note that `handle_from` decides what `{{handle}}` resolves to.
With `"$.job_id"` the handle *is* the id string, so the poll path
interpolates `{{handle}}`; a `handle_from` of `"$"` keeps the whole
response body and the path would read `{{handle.job_id}}` instead.

### Template placeholders

Simple `{{...}}` substitution, no arithmetic. Supported roots:

- `{{selection_id}}`, `{{dispatch_id}}`, `{{persona_id}}`,
  `{{action}}` — dispatch-context ids.
- `{{input[N].<field>}}` — indexed input asset field. Supported
  fields: `id`, `persona_id`, `source_locator`, `source_kind`,
  `modality`, `cover`.
- `{{params.<dot.path>}}` — deep dot-access into the params JSON
  itself (so the caller can define its own "extra fields" section
  in params and reference it from templates).
- `{{handle.<dot.path>}}` — deep dot-access into the handle JSON.
  Only available in `poll` / `harvest` templates (the exporter
  panics on this in `dispatch`, when no handle exists yet).
- `{{item.<dot.path>}}` — dot-access into the current
  `harvest.items_path` element. Only available inside
  `harvest.map`.

A trailing `?` on a placeholder (`{{item.caption?}}`) means
"resolve to empty string when the path is missing" instead of
failing with `BackendRejected`.

Params are persisted unedited. The blob handed to
`CreateDispatchCommand` is stored whole as
`dispatch_job.params_json` and handed back out as
`DispatchDto.params_json` on every read of the dispatch — nothing
on that path filters, redacts, or drops a field. A credential
reached by `{{params.…}}` (the `extras.api_key` above) is
therefore readable by anything that can list dispatches; put one
there only where that visibility is acceptable.

### JSONPath

Minimal subset — enough to steer the state machine and pluck out
items:

- `$.foo`             — object field.
- `$.foo.bar`         — dot chain.
- `$.arr[0]`          — array index.
- `$.arr[*]`          — array wildcard (only the last segment can
  be a wildcard; matches the shape of every documented example).

Anything outside this grammar is rejected up front with
`BackendRejected`.

