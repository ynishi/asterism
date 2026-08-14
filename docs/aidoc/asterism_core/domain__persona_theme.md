# asterism-core::domain::persona_theme

`PersonaTheme` — a 1:1 aggregate holding the persona-scoped visual
chrome (wallpaper reference) that the UI applies when the persona is
the active selection.

Kept separate from `Persona` on purpose: the persona entity is the
identity of an actor (id, name, pack ref), and adding presentation
fields onto it would drag the aggregate across two responsibilities.
The theme is a small side-record — optional per persona — that the
UI overlays; missing theme is the "no chrome, defaults only" case.

Content is stored as a reference to an existing image `Asset`
(`wallpaper_asset_id`), not as bytes. That preserves the "Asterism
is an archive of already-imported material" boundary — the persona
theme cannot introduce new artefacts, only re-use ones the user
already brought in through an importer.

## Types

- `PersonaTheme` — Per-persona visual chrome. Optional: `PersonaThemeRepository::get`

