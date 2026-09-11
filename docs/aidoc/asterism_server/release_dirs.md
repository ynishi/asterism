# asterism-server::release_dirs

The two registered directories the release surface uses, resolved.

[`release.output_dir`] names where a change point is written out to
and [`send.profile_dir`] names where destination profiles are read
from. Both are
[`Text`](asterism_core::domain::app_setting::SettingValueKind::Text)
keys in
[`SETTING_REGISTRY`](asterism_core::domain::app_setting::SETTING_REGISTRY),
and both default to the empty string.

# Why the empty string, and why this module exists

A registry default is a `&'static str`, and neither of these paths
is a constant: the profile home is resolved at run time from
`$ASTERISM_HOME` and `$ASTERISM_PROFILE`, differs between two
processes of the same build, and is verified against a marker on the
way out. There is nothing to write into the registry, so the empty
string is what it writes, and it means the leaf this module gives
that key under the profile home — `releases/` or `transfer/`.

That convention needs exactly one reader, or it becomes two
resolvers that disagree the first time somebody changes the leaf
name. This module is it: callers ask it rather than joining a path
of their own, and the frontend is handed the answer rather than
deriving it — it cannot read the environment at all, and a second
copy of the rule in TypeScript is the copy nobody would edit.

# A value somebody set is used as typed

Only the empty case reaches the profile home. A path a person put in
the settings screen is taken as it stands and never joined onto
anything, because a setting that silently became a subdirectory of
itself is a value that cannot be pointed anywhere.

[`release.output_dir`]: asterism_core::domain::app_setting::SETTING_REGISTRY
[`send.profile_dir`]: asterism_core::domain::app_setting::SETTING_REGISTRY

## Functions

- `output_dir` — Where the next release writes its copies.
- `profile_dir` — Where destination profiles are read from.
- `resolved_output_dir` — Where the next release writes its copies, read from the registry.
- `resolved_profile_dir` — Where destination profiles are read from, read from the registry.

## Constants

- `OUTPUT_DIR_KEY` — The registry key naming where a release writes its copies.
- `PROFILE_DIR_KEY` — The registry key naming where destination profiles are read from.

