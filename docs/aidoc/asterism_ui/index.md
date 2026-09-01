# asterism-ui 0.0.0

# asterism-ui — Asterism desktop UI (Tauri v2 backend)

## Role

Tauri v2 + Rust + Svelte 5 desktop UI. The grid is dense, hovers pull
up a small constellation burst, and there is no persistent chat
surface (a Command-K palette is left as future work).

## Design intent

- Local-first — no cloud dependencies; everything ships with the app.
- Grid density over card size; a hover surfaces a few related items
  in a side panel rather than growing the card.
- Register / tone is expressed through persona accent colours and
  cover-text typography.
- Image binaries are served through the Tauri v2 asset protocol
  (`convertFileSrc()`), never through IPC.

## `assetProtocol` scope (`tauri.conf.json`)

The scope is `$HOME/**` and nothing else. Every widened alternative
was considered and rejected:

- It cannot be narrowed to the profile home. An asset's `locator` is
  wherever the user's file already lives — `~/Pictures`, `~/Desktop`,
  a project directory — and `convertFileSrc(locator)` is the only
  path by which the grid, the detail pane and the wallpaper theme
  read an original. Narrowing to `~/.asterism/**` would leave every
  in-place import unreadable.
- It does not need `/tmp/**` or `/private/tmp/**`. Those entries were
  here for dropped macOS screenshots, but [`commands::rehome_dropped_path`]
  copies any TEMP-rooted drop under `$HOME` *before* the locator is
  persisted, precisely so the durable copy lands inside this scope.
  Keeping them granted read access to two world-writable directories
  for a path the app no longer produces.

An original that lives outside `$HOME` (an external volume,
`/Users/Shared`) is consequently not readable over this protocol —
unchanged by the narrowing above, since neither removed entry
covered those paths either.

## Modules

- [`commands`](commands.md): Tauri command handlers — a thin translation layer. They pass DTOs
- [`error`](error.md): `UiError` — error type crossed by Tauri command handlers.
- [`state`](state.md): `AppState` — service DI + backend initialisation.
- [`stored_connection`](stored_connection.md): What this machine remembers about a team server between windows

