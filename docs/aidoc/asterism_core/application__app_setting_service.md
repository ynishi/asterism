# asterism-core::application::app_setting_service

`AppSettingService` — use cases for application settings.

Resolves the closed registry
([`SETTING_REGISTRY`](crate::domain::app_setting::SETTING_REGISTRY))
against the process environment and the stored overrides in
`app_setting`, in that order of increasing precedence.

## Why the stored row wins over the environment

`default` → `env` → `stored`, uniformly for every key. A settings
screen makes exactly one promise — that what you pick is what runs —
and a process-wide variable quietly outranking it breaks that promise
with no recourse from inside the app. The environment therefore acts
as a *seed*: it supplies the value while nothing is stored, and gives
way once someone chooses one.

One order for all keys is deliberate. Splitting the registry into
"user preference" and "operational knob" halves, each with its own
precedence, was considered and rejected as premature — a per-key rule
is only worth its complexity once a key actually needs enforcement,
and none does today. The escape hatch when that changes is a
separate, explicitly-named lock, not a second ordering.

An env var whose contents do not parse as the declared kind, or which
falls outside the key's declared range, is **ignored** rather than
fatal: a typo in a shell export should not stop the application from
starting. It is logged, because an override that silently does
nothing is the hardest kind to diagnose.

## Every layer is kept

[`EffectiveSetting::layers`] carries each layer that has a value, not
just the winner, so a client can show what a value is shadowing.
Collapsing to the winner is what previously left a stored row hidden
underneath an env var with no way to see or clear it.

## Attribution

[`set`](AppSettingService::set) and [`reset`](AppSettingService::reset)
take an [`AttributionContext`] they do not persist: `app_setting` is a
closed key → value registry with no room for a writer, and none is
being added (see the [`application`](crate::application) module doc
for why the argument is required anyway).

## Types

- `AppSettingService` — Application settings use-case service. Shared as an `Arc` through
- `ProcessEnv` — [`EnvSource`] backed by the real process environment.

## Traits

- `EnvSource` — Reads the process environment. Injected so tests can resolve against

