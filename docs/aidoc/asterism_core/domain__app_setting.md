# asterism-core::domain::app_setting

Application settings — the closed key registry and its stored
overrides (the `app_setting` table).

Two-layer model, mirroring [`Modality`](crate::domain::value::Modality)
/ [`ContentKind`](crate::domain::value::ContentKind):

- **Closed** — [`SETTING_REGISTRY`] enumerates every key the
  application understands, together with its value kind, its code
  default, and (optionally) the environment variable that seeds it.
  A key that is not in the registry cannot be written.
- **Open** — the `app_setting` table holds only the rows a user has
  actually changed. An unset key is not a row; it resolves to the
  registry default.

## Why the backend owns this

`localStorage` is reachable only from the webview, so `--headless`
runs, the HTTP server, and the job engine could never read a user
preference. Keeping settings in the profile database also means they
travel with the profile: one `asterism.db` backup carries the data
*and* the preferences, and `dev` / `dogfood` / `bench` stay isolated
from each other for free.

## Resolution order

`default` → `env var` → `stored row` (last wins). **A value the user
chose outranks the environment**, uniformly, for every key.

The environment is a *seed*, not a ceiling: it supplies the value
while nothing is stored, and steps aside once someone sets one. This
is the shape Open WebUI settled on for the same reason — a control
that accepts input and then has it silently discarded by a
process-wide variable breaks the only contract a settings screen
has.

The inverse order (`env` last) is the convention for *deployment*
configuration, where an operator's variable must beat whatever a file
says. These keys are user preferences, so that convention does not
apply; mixing the two ownership domains in one key namespace is what
made the earlier ordering wrong. If a key ever genuinely needs
enforcement, the established answer is a separate, explicitly-named
lock — not a reordering of this stack.

Every layer that has a value is kept, not just the winner
([`EffectiveSetting::layers`]), so a caller can show where a value
came from and what it is shadowing. Collapsing to the winner is what
previously made a shadowed stored row invisible *and* unreachable.

## Types

- `AppSetting` — A user override as stored in `app_setting`. Absence of a row is the
- `EffectiveSetting` — A key resolved through the full layer stack — what a caller should
- `SettingDef` — One entry of the closed registry: the contract for a single key.
- `SettingKey` — A registry-backed key. Construction is the membership check, so any
- `SettingLayer` — One layer that has a value for a key.
- `SettingSource` — Which layer supplied the value the application will actually use.
- `SettingValueKind` — Value shape a setting accepts. Kept deliberately small — a setting

## Constants

- `SETTING_REGISTRY` — Every key the application understands.

