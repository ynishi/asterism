# asterism-infra::paths

Data-profile and on-disk layout conventions.

Asterism separates three local workloads that have very different
durability requirements:

- [`DataProfile::Dev`] — disposable development data.
- [`DataProfile::Dogfood`] — durable, real daily-use data.
- [`DataProfile::Bench`] — reproducible large/stress datasets.

Resolution order is `$ASTERISM_HOME` (explicit path) followed by
`$ASTERISM_PROFILE`, then a build-mode default (`dev` for debug builds,
`dogfood` for release builds). Named profiles live below
`$HOME/.asterism/profiles/<profile>`. An explicit home without a profile
is treated as `custom`, preserving scratch/test workflows.

Named homes contain a `.asterism-profile` marker. Opening a home whose
marker disagrees with `$ASTERISM_PROFILE` is rejected before SQLite or
Tantivy is touched; this is the last guard against a mistyped launch
command pointing a development build at durable dogfood data.

## Functions

- `active_profile` — Returns the active data profile without creating directories.
- `asterism_home` — Returns (creating on demand) the isolated Asterism home directory.
- `default_db_path` — Returns the default SQLite database path (`<asterism_home>/asterism.db`).
- `tantivy_index_dir` — Returns (creating on demand) the profile-local Tantivy index directory.

## Types

- `DataProfile` — Isolated local dataset selected for the current process.

