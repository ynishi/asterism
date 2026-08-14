//! `PersonaProfile` — a 1:1 side aggregate holding the identity
//! signal for a Persona (avatar reference, short bio, role tag)
//! that asterism uses inside the app to say "who this persona
//! is".
//!
//! Deliberately kept separate from [`PersonaTheme`]: the theme is
//! chrome (wallpaper etc.) that follows the persona's mood, the
//! profile is stable identity metadata that changes rarely.
//!
//! `persona-pack` remains the source of truth for the persona
//! definition (`prompt.body`, `extra.*` character system). The
//! profile stored here is asterism's own note about the persona
//! for the archive UX — an avatar the user picks from their own
//! imported assets, a one-line bio, and a role tag. It is
//! deliberately shallow so it does not become a mirror of the
//! external SoT.

use chrono::{DateTime, Utc};

use crate::domain::value::{AssetId, PersonaId};

/// Per-persona identity signal used inside asterism. Optional —
/// missing profile means "no card yet, sidebar falls back to the
/// name and accent color".
#[derive(Debug, Clone, PartialEq)]
pub struct PersonaProfile {
    /// Owner persona (primary key).
    pub persona_id: PersonaId,
    /// Portrait / thumbnail asset. Must be an image asset the user
    /// already imported; the reference is set via the profile edit
    /// form, not by an external upload path.
    pub avatar_asset_id: Option<AssetId>,
    /// One-line description of who this persona is inside
    /// asterism. Not a mirror of `persona-pack.meta.short` — the
    /// user is free to write their own note here.
    pub bio_short: Option<String>,
    /// Free-form role tag (`"companion"`, `"agent"`,
    /// `"friend"`, ...). Rendered as a small chip on the card.
    pub role_tag: Option<String>,
    /// Last-change timestamp.
    pub updated_at: DateTime<Utc>,
}

impl PersonaProfile {
    /// Fresh profile with only the fields the caller supplies;
    /// omitted values default to `None`.
    pub fn new(
        persona_id: PersonaId,
        avatar_asset_id: Option<AssetId>,
        bio_short: Option<String>,
        role_tag: Option<String>,
    ) -> Self {
        Self {
            persona_id,
            avatar_asset_id,
            bio_short,
            role_tag,
            updated_at: Utc::now(),
        }
    }
}
