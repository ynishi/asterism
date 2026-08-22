# asterism-exporter-http 0.0.0

# asterism-exporter-http

Schema-driven exporter for **HTTP job APIs**. Where
[`asterism_exporter_comfy`] hard-codes the ComfyUI protocol, this
crate stays backend-agnostic: **the caller supplies the submit / poll
/ harvest shapes as JSON schema in the dispatch params**. One
deployable adapter, N backends.

Position in the workspace: mirror of `asterism-importer-sqlite`
("query + column map = whole importer") on the OUT side. Where the
SQLite importer's caller writes SQL + a column mapping, this
exporter's caller writes an HTTP shape + a JSON-path mapping.

## Why there is no second adapter for hosted platforms

There was one, for a while: `asterism-exporter-cloud`, carrying its
own copy of this schema — the same poll predicates, the same harvest
item map, the same handle — because a hosted platform needed three
things this adapter had not grown yet. It was the wrong axis. A
hosted platform and a self-hosted backend speak the same job API:
submit, keep a handle, poll, collect. Whether the URL is `https`,
whether the credential comes from the environment, and how long to
keep waiting are configuration, not adapter identity, and splitting
on them bought duplication while leaving this side stuck with the
weaker credential story it had deferred.

So the three arrived here as optional blocks, and "cloud" became a
profile:

| block | absent | present |
|---|---|---|
| [`auth`](AuthSchema) | the profile carries no credential, or reaches one through its own params | the credential is named by environment variable and never persisted |
| [`fetch`](FetchSchema) | the backend's own URL is the locator | the bytes are pulled into custody first, and *that path* is the locator |
| `deadline_seconds` | poll until the backend answers | a job past its deadline fails as expired |

The distinction worth keeping is a different one: a backend reachable
as an HTTP job API — which a profile covers — versus a backend that
ships an SDK, which will not be a profile at all.

## Params schema

`CreateDispatchCommand.params_json` deserialises into
[`HttpDispatchParams`]. All three phases (`submit` / `poll` /
`harvest`) are configured up front so the runner can drive the state
machine without re-reading params on every tick.

```json
{
  "endpoint": "http://backend.example.com",

  "submit": {
    "method": "POST",
    "path": "/generate",
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
      "source_url":    "{{item.url}}",
      "cover_hint":    "{{item.caption?}}",
      "labels_static": ["batch:{{dispatch_id}}"]
    }
  },

  "auth": {
    "secret_ref":     "BACKEND_KEY",
    "header":         "authorization",
    "value_template": "Bearer {{secret}}"
  },
  "fetch": { "authenticated": false },
  "deadline_seconds": 86400,

  "extras": {
    "prompt": "photo studio portrait"
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
[`TemplateAdapter`] / [`ResponsePath`] traits, and the implementation
it holds is [`SecretGrammar`] — the shared roots plus `{{secret}}`,
bound per call from whatever the `auth` block names. A profile
without that block binds nothing, and `{{secret}}` in one is refused
rather than rendered away.

## Where a credential may live, and what that costs

Params are persisted unedited: the blob handed to
`CreateDispatchCommand` is stored whole as `dispatch_job.params_json`
and handed back out as `DispatchDto.params_json` on every read of the
dispatch, and nothing on that path filters, redacts, or drops a
field. A credential reached by `{{params.…}}` is therefore readable
by anything that can list dispatches — and, since the call is
recorded (below), it also rides on the assets the job produced.

`auth.secret_ref` is the way out, and it holds an environment
variable *name*, never a value: the credential is resolved per call,
rendered into `{{secret}}`, and is in neither the params blob nor
anything written down. Loading a `.env` file is the binary's job,
done once at startup — an adapter that went looking for dotenv files
itself would make "which file did this credential come from"
invisible to the profile that named it.

## The call is recorded, and it arrives with the artefact

The request as sent and the response as received are kept on the
dispatch row, in the exporter-owned handle payload the runner already
persists and hands back — and the harvest copies that record onto
every [`Derived`] it returns, under `extra.http.call`, together with
the finished job's response whole. A backend that answers with a
result URL and little else is the ordinary case: the model can be an
ambient default, the seed is an input that is usually not echoed, and
an enhanced prompt is not the prompt that was sent. None of that is
in the bytes, so the moment of the call is the only moment it exists.
The response is kept whole because those values are siblings of the
artefacts array rather than fields of an item.

The recorded copy is scrubbed of the credential the `auth` block
named, and its headers are redacted. So is the handle itself, on the
way into the payload: `submit.handle_from` defaults to the whole
submit response, a backend is free to echo the request it was sent,
and that payload is handed back out on every read of the dispatch —
including to a caller that never touches the database. What no scrub
can reach is a token a profile interpolated out of its own params
into a URL or a body: the adapter was never told it was a
credential. That is the same trade the paragraph above describes,
one surface further along.

## Modules

- [`custody`](custody.md): Where a produced file lands once we hold it.
- [`secret`](secret.md): The profile grammar this adapter uses: the shared one, plus

