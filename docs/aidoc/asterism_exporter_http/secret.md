# asterism-exporter-http::secret

The profile grammar this adapter uses: the shared one, plus
`{{secret}}` when the profile names a credential.

The root is bound per call, from the environment variable an `auth`
block names, and the value is never written into the params blob —
which matters because that blob is persisted unedited and handed back
on every read of the dispatch. A profile that instead interpolates a
token it stored in its own params (`{{params.extras.api_key}}`, the
pattern this adapter shipped with before it had an `auth` block) is
still allowed and still works; what it loses is exactly this: nothing
can scrub a value the adapter was never told was a credential.

A profile with no `auth` block binds no secret. `{{secret}}` is then
an error rather than an empty string: rendering it away would send an
unauthenticated request that looks like an authenticated one, and the
backend's 401 is a worse place to learn it than the dispatch that
refuses to start.

Only [`TemplateAdapter::render`] is overridden. The JSON-leaf and
header traversals are the trait's default methods, written in terms
of `render`, so `{{secret}}` means the same thing in a header, in a
body field and in a query string without three implementations
agreeing to.

## Types

- `SecretGrammar` — [`CommonExportAdapter`] plus the `{{secret}}` root.

