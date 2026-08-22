# teams-infra::paths

Data-profile and on-disk layout conventions for the teams plane.

This mirrors `asterism-infra`'s profile mechanism — the same
selection table, the same marker guard — with every name swapped so
the two planes cannot open each other's data by accident:

| | local app | teams plane |
|---|---|---|
| profile env | `ASTERISM_PROFILE` | `ASTERISM_TEAMS_PROFILE` |
| home env | `ASTERISM_HOME` | `ASTERISM_TEAMS_HOME` |
| named home | `~/.asterism/profiles/<p>` | `~/.asterism-teams/profiles/<p>` |
| marker | `.asterism-profile` | `.asterism-teams-profile` |
| database | `asterism.db` | `teams.db` |

Mirrored rather than imported because importing would mean a
`teams-* → asterism-infra` edge, which #83 §4 forbids in any form.

The selection table is the one `asterism-infra` settled on:

| `$ASTERISM_TEAMS_HOME` | `$ASTERISM_TEAMS_PROFILE` | result |
|---|---|---|
| unset | unset | the build's default — `dev` in debug, `dogfood` in release |
| unset | `dev` / `dogfood` / `bench` | that profile, under `~/.asterism-teams/profiles/` |
| unset | `custom` | error: `custom` is a home, and none was given |
| set | unset | error: name the profile too |
| set | `dev` / `dogfood` / `bench` | that profile, at the explicit path |
| set | `custom` | `custom` at the explicit path, unguarded |

Named homes contain a marker; opening a home whose marker disagrees
with the requested profile is rejected before SQLite is touched.
The marker is published the way the sibling publishes its own —
contents written to a temporary file, synced, then hard-linked
under the marker's name (`create_new` fallback only where the
filesystem has no hard links): the name appears already complete or
not at all, so no observable state ever holds an empty marker. A
crash between creating the name and writing its contents would
otherwise wedge the home permanently — an empty marker rejects
every later open with `marker says ""`, and nothing here repairs
one — which is why the mechanism is mirrored rather than simplified
for this plane's quieter topology. `hard_link` rather than `rename`
for the sibling's reason too: rename replaces, and two profiles
racing for one fresh home must never both claim it.

## Functions

- `active_profile` — Returns the active teams-plane data profile without creating
- `default_blob_root` — Returns the default blob store root (`<teams_home>/blobs`), under
- `default_db_path` — Returns the default teams SQLite database path
- `teams_home` — Returns (creating on demand) the teams-plane home directory for the

## Types

- `DataProfile` — Isolated local dataset selected for the current process — same four

