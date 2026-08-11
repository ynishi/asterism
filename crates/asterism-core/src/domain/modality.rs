//! `ModalityDef` — the open Modality master entry (the `modality` table).
//!
//! Two-layer model:
//! identity + presentation metadata live here (an **open**, dynamically
//! addable/removable master row); behaviour lives in the **closed**
//! [`ContentKind`] the row points at through `kind`. Consumers (jobs /
//! UI / import) branch on `kind` / capabilities, never on the raw slug.
//!
//! The row carries only identity + presentation:
//!
//! - `slug` — the primary key ([`Modality`], an open slug).
//! - `label` — display name.
//! - `kind` — the single reference into behaviour ([`ContentKind`]).
//! - `sort_order` — sidebar rank (the SoT for modality ordering).
//! - `hidden` — soft-retirement flag (the operational alternative to a
//!   delete when assets still reference the slug).
//! - `cover_template` — optional override of the kind's default
//!   [`CoverTemplate`].

use crate::domain::value::{CoverTemplate, Modality};

/// One row of the `modality` master — an open, user-editable entry.
///
/// It carries identity and presentation only. Behaviour used to hang
/// off a `kind` column pointing at a closed `ContentKind`, but v4 moved
/// the decisions it fed to where the facts are: thumbnails and media
/// rendering read the material's mime
/// ([`render_policy`](crate::domain::render::render_policy)) and
/// containment reads `asset.role`. What survived is the single question
/// mime cannot answer, and it is a yes/no — see [`Self::terminal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModalityDef {
    /// Primary key — the open modality slug.
    pub slug: Modality,
    /// Display name shown in the sidebar / filter chips.
    pub label: String,
    /// Whether assets of this classification read as terminal
    /// transcripts. A tape is `text/plain` like any other note, so no
    /// amount of byte inspection reveals that it should render as a
    /// terminal — that is what this bit is for, and the only behaviour
    /// the semantic axis still decides.
    pub terminal: bool,
    /// Sidebar sort rank (the source of truth for modality ordering).
    pub sort_order: i64,
    /// Whether the modality is hidden from the default UI listing.
    /// Retirement path when a delete is not allowed (assets present).
    pub hidden: bool,
    /// Optional cover template. `None` means the generic
    /// first-meaningful-line template.
    pub cover_template: Option<CoverTemplate>,
}

/// A [`ModalityDef`] paired with the number of assets currently
/// carrying its slug — the shape the master listing returns so the UI
/// can badge counts and the delete guard can gate on `count == 0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModalityView {
    /// The master row.
    pub def: ModalityDef,
    /// Count of `asset` rows whose `modality` equals `def.slug`.
    pub asset_count: u64,
}
