# asterism-exporter-cloud::grammar

The profile grammar this adapter uses: the shared one, plus
`{{secret}}`.

This is the case [`asterism_exporter_common`]'s traits exist for. The
HTTP exporter must **not** have a `{{secret}}` root — everything it
can reach comes out of the params blob, which is persisted unedited,
so a root that resolved to a credential there would be a root that
wrote one down. Here the credential is resolved from the environment
at call time and never persisted, so the root is safe *in this
adapter and not in that one*.

Only [`TemplateAdapter::render`] is overridden. The JSON-leaf and
header traversals are the trait's default methods, written in terms
of `render`, so `{{secret}}` means the same thing in a header, in a
body field and in a query string without three implementations
agreeing to.

## Types

- `CloudGrammar` — [`CommonExportAdapter`] plus the `{{secret}}` root.

