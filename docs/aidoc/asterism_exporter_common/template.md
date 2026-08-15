# asterism-exporter-common::template

`{{...}}` substitution over a dispatch — the other half of what a
schema-driven exporter needs in order to be configured rather than
written.

Simple textual replacement, no arithmetic and no conditionals. The
resolvable roots are the dispatch's own ids, the input assets, the
params blob itself, the handle once one exists, and the current item
during a per-item mapping:

```text
{{selection_id}} {{dispatch_id}} {{persona_id}} {{action}}
{{input[0].source_locator}}
{{params.some.nested.key}}
{{handle.job_id}}
{{item.url}}
```

A trailing `?` (`{{item.caption?}}`) resolves a missing path to the
empty string instead of failing. Without it an unresolved placeholder
is an error, and that is the right default: a profile that names a
field the backend does not send has a bug in it, and silently sending
an empty prompt is a worse way to find out than a rejected dispatch.

# `params` is the caller's namespace

The exporter knows nothing about what is in the params blob beyond
the fields its own schema names. `{{params.<dot.path>}}` reaches
anywhere inside it, so a profile author nests per-backend values
wherever they like and references them from templates.

Two consequences worth stating together, because the first is the
reason the second matters. Params are persisted unedited and handed
back on every read of the dispatch — nothing on that path filters or
redacts. So a value reachable by `{{params.…}}` is readable by
anything that can list dispatches, and a credential does not belong
there. An adapter that needs one resolves it outside the blob.

# What is here and what is on the trait

This module is the grammar: what a placeholder may name, and how one
template string is rendered. Everything built *on top* of that — JSON
documents, header maps — lives on [`crate::TemplateAdapter`] as default
methods, so an adapter that changes the grammar changes it in one
place and the traversals follow.

## Functions

- `render` — Renders one template string.
- `value_to_display_string` — How a resolved value is spelled when it lands in a string.

## Types

- `TemplateEnv` — What a placeholder can be resolved against.

