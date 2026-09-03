# asterism-ui::state

`AppState` — service DI + backend initialisation.

Tauri's own `State` container gives us `Arc`-style sharing, so this
struct just holds `Arc<Service>` fields and does not wrap again. The
heavy lifting is delegated to the shared `asterism_server::core_init`
(`Full` mode: read-write tantivy index + job worker); this module only
adapts the returned `CoreCtx` into `AppState` and supplies the Tauri
progress emitter.

## Functions

- `init` — Initialises the whole backend and returns both the Tauri `AppState`

## Types

- `AppState` — Bundle of services registered as Tauri state.
- `ProviderSignInInFlight` — A sign-in through the team's identity provider that is waiting for
- `TeamsConnection` — A live team session, and the pair that names it.

