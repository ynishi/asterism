# asterism-core::domain::persona_profile

`PersonaProfile` — a 1:1 side aggregate holding the identity
signal for a Persona (avatar reference, short bio, role tag)
that asterism uses inside the app to say "who this persona
is".

Deliberately kept separate from [`PersonaTheme`]: the theme is
chrome (wallpaper etc.) that follows the persona's mood, the
profile is stable identity metadata that changes rarely.

`persona-pack` remains the source of truth for the persona
definition (`prompt.body`, `extra.*` character system). The
profile stored here is asterism's own note about the persona
for the archive UX — an avatar the user picks from their own
imported assets, a one-line bio, and a role tag. It is
deliberately shallow so it does not become a mirror of the
external SoT.

## Types

- `PersonaProfile` — Per-persona identity signal used inside asterism. Optional —

