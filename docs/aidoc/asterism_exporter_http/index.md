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

### Templates and JSONPath

Both grammars are the shared adapter machinery, documented where
they are defined: [`asterism_exporter_common::template`] for the
`{{...}}` roots, the optional-`?` suffix and which of them resolve in
which phase, and [`asterism_exporter_common::jsonpath`] for the path
subset. They are not restated here — a grammar with two write-ups
grows two meanings, and a profile author cannot tell which one their
adapter implements.

This exporter reaches them through the
[`TemplateAdapter`] / [`ResponsePath`] traits and is parameterised on
the implementation, defaulting to
[`CommonExportAdapter`][asterism_exporter_common::CommonExportAdapter].
`HttpExporter::new()` therefore keeps meaning what it meant, and an
adapter that needs a placeholder root this one must not have — a
credential resolved outside the params blob, say — supplies its own
grammar rather than widening this one.

One consequence of the params blob being a template namespace is
worth repeating at the point of use, because it decides what may go
in the example above. Params are persisted unedited: the blob handed
to `CreateDispatchCommand` is stored whole as
`dispatch_job.params_json` and handed back out as
`DispatchDto.params_json` on every read of the dispatch, and nothing
on that path filters, redacts, or drops a field. A credential reached
by `{{params.…}}` — the `extras.api_key` above — is therefore
readable by anything that can list dispatches. Put one there only
where that visibility is acceptable.

