# asterism-importer-sdk::client

Thin HTTP client for the asterism-server API.

Wraps the endpoints an importer actually needs: the two that land
records (`POST /asterism/assets/add` and `/add-batch`), and the two
that keep its position between runs, behind
[`HttpSyncStore`].

## Types

- `ApiClient` — HTTP client bound to a running `asterism-server`.
- `HttpSyncStore` — The [`SyncStore`] an adapter that pushes over HTTP uses.

