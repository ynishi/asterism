# asterism-ui::error

`UiError` — error type crossed by Tauri command handlers.

Tauri commands must return `Result<T, E>` where `E: Serialize`
(`anyhow` will not do). We use a `serde` tagged enum
(`{ kind, message }`) so the TypeScript side can pattern-match on the
variant.

## Types

- `UiError` — Error returned to the frontend from every Tauri command.

