# asterism-core::application::thumb_service

`ThumbService` — pre-generated thumbnail cache use cases.

Thin wrapper over the `ThumbRepository` port. The service exists so
the wire adapters (Tauri command, HTTP handler) share the same
entry-point instead of each poking at the repo directly. Encoding
and resize decisions live upstream in the importer — the service
only stores and retrieves opaque bytes.

## Types

- `ThumbService` — Thumbnail cache use-case service. Shared as an `Arc`.

