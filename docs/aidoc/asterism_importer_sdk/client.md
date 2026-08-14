# asterism-importer-sdk::client

Thin HTTP client for the asterism-server API.

Only wraps the two endpoints an importer actually needs:
`POST /asterism/assets/add` (single) and
`POST /asterism/assets/add-batch`.

## Types

- `ApiClient` — HTTP client bound to a running `asterism-server`.

