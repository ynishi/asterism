//! `PersonaTheme` — a 1:1 aggregate holding the persona-scoped visual
//! chrome (wallpaper reference) that the UI applies when the persona is
//! the active selection.
//!
//! Kept separate from `Persona` on purpose: the persona entity is the
//! identity of an actor (id, name, pack ref), and adding presentation
//! fields onto it would drag the aggregate across two responsibilities.
//! The theme is a small side-record — optional per persona — that the
//! UI overlays; missing theme is the "no chrome, defaults only" case.
//!
//! Content is stored as a reference to an existing image `Asset`
//! (`wallpaper_asset_id`), not as bytes. That preserves the "Asterism
//! is an archive of already-imported material" boundary — the persona
//! theme cannot introduce new artefacts, only re-use ones the user
//! already brought in through an importer.

use chrono::{DateTime, Utc};

use crate::domain::value::{AssetId, PersonaId};

/// Per-persona visual chrome. Optional: `PersonaThemeRepository::get`
/// returns `None` when the persona has never had a theme set, and the
/// UI falls back to the built-in defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonaTheme {
    /// Owner persona (primary key).
    pub persona_id: PersonaId,
    /// Image asset used as the wallpaper. `None` clears the wallpaper
    /// while keeping the theme row so the "custom theme exists" flag
    /// stays true (useful when other theme fields land later).
    pub wallpaper_asset_id: Option<AssetId>,
    /// Last-change timestamp — surfaced to the UI so a "recently
    /// updated" hint can render without a follow-up query.
    pub updated_at: DateTime<Utc>,
}

impl PersonaTheme {
    /// Fresh theme with the given wallpaper (or `None` to clear).
    pub fn new(persona_id: PersonaId, wallpaper_asset_id: Option<AssetId>) -> Self {
        Self {
            persona_id,
            wallpaper_asset_id,
            updated_at: Utc::now(),
        }
    }
}
