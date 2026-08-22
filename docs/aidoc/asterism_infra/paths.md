# asterism-infra::paths

Data-profile and on-disk layout conventions.

Asterism separates three local workloads that have very different
durability requirements:

- [`DataProfile::Dev`] — disposable development data.
- [`DataProfile::Dogfood`] — durable, real daily-use data.
- [`DataProfile::Bench`] — reproducible large/stress datasets.

`$ASTERISM_PROFILE` names the profile and `$ASTERISM_HOME` overrides
where it lives; with neither, the build decides (`dev` for debug,
`dogfood` for release). Named profiles live below
`$HOME/.asterism/profiles/<profile>`.

An explicit home used to fall back to `custom` when the profile was
absent, which made the unguarded mode something you reached by
forgetting. It is now asked for by name, and the table is:

| `$ASTERISM_HOME` | `$ASTERISM_PROFILE` | result |
|---|---|---|
| unset | unset | the build's default — `dev` in debug, `dogfood` in release |
| unset | `dev` / `dogfood` / `bench` | that profile, under `$HOME/.asterism/profiles/` |
| unset | `custom` | error: `custom` is a home, and none was given |
| set | unset | error: name the profile too |
| set | `dev` / `dogfood` / `bench` | that profile, at the explicit path |
| set | `custom` | `custom` at the explicit path, unguarded |

`$ASTERISM_PROFILE` alone is ordinary. It is the home-without-a-name
direction that is refused, because that is the one that used to
silently disable the marker.

Named homes contain a `.asterism-profile` marker. Opening a home whose
marker disagrees with `$ASTERISM_PROFILE` is rejected before SQLite or
Tantivy is touched; this is the last guard against a mistyped launch
command pointing a development build at durable dogfood data.

## Functions

- `active_profile` — Returns the active data profile without creating directories.
- `asterism_home` — Returns (creating on demand) the isolated Asterism home directory.
- `default_db_path` — Returns the default SQLite database path (`<asterism_home>/asterism.db`).
- `models_dir` — Returns (creating on demand) the profile-local model-package root
- `tantivy_index_dir` — Returns (creating on demand) the profile-local Tantivy index directory.

## Types

- `DataProfile` — Isolated local dataset selected for the current process.

