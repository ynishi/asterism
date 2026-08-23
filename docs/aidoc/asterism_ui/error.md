# asterism-ui::error

`UiError` — error type crossed by Tauri command handlers.

Tauri commands must return `Result<T, E>` where `E: Serialize`
(`anyhow` will not do). We use a `serde` tagged enum
(`{ kind, message }`) so the TypeScript side can pattern-match on the
variant.

Internally tagged with named fields rather than
`content = "message"` around a tuple, and the reason is one field
on one variant: a conflict carries `reason` beside its message, and
a variant holding two things cannot be a tuple behind a `content`
key without nesting them. What crosses the wire is unchanged for
every other variant — `{ kind, message }`, read that way in
`src/lib/mutate.ts` — and a conflict is that plus one field a
reader is free to ignore.

## Types

- `UiError` — Error returned to the frontend from every Tauri command.

