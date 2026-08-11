//! Assembles the [`SortContext`] the sort evaluator needs, sourcing every
//! lookup from a repository.
//!
//! Two callers, one implementation: the Query Group evaluator freezes the
//! resulting order into `asset_bucket.position`, and
//! [`AssetService::list`](crate::application::AssetService::list) answers a
//! caller that named an axis on the wire. They have to agree — a page that
//! comes back in a different order than the one a Query Group materialised
//! under the same spec would make the two features disagree about what
//! `Sort: Group` / `Order: ordered` means.
//!
//! Lookups and their fidelity:
//!
//! - **persona order / names** — [`PersonaRepository::list`] sorted by
//!   `display_order` (then id for stability): the authentic backend
//!   analogue of the UI sidebar order.
//! - **modality order** — [`AssetRepository::counts_by_modality`]. **Known
//!   drift**: this is corpus-frequency order, not the UI's hand-arranged
//!   sidebar order (which lives only in the browser's `localStorage`,
//!   `App.svelte` `MODALITIES`). It only affects the `modality` +
//!   `ordered` axis; every other axis is frequency-independent. A
//!   persisted backend modality order is the proper fix and is out of
//!   W1's scope.
//! - **group names** — [`GroupRepository::list`].
//!
//! `persona` scopes the modality and group lookups. `None` means "every
//! persona", which is what an unscoped listing asks for: scoping those two
//! to one persona while the filter selects across all of them would rank
//! the axis by a corpus the page does not show.

use crate::domain::repository::{AssetRepository, GroupRepository, PersonaRepository};
use crate::domain::sort_eval::SortContext;
use crate::domain::value::PersonaId;
use crate::error::DomainError;

/// Builds the lookup context for [`sort_asset_ids`](crate::domain::sort_eval::sort_asset_ids).
pub async fn build_sort_context(
    personas: &dyn PersonaRepository,
    assets: &dyn AssetRepository,
    groups: &dyn GroupRepository,
    persona: Option<&PersonaId>,
) -> Result<SortContext, DomainError> {
    let mut rows = personas.list().await?;
    rows.sort_by(|a, b| {
        a.display_order
            .cmp(&b.display_order)
            .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
    });
    let persona_order: Vec<String> = rows.iter().map(|p| p.id.to_string()).collect();
    let persona_names = rows
        .iter()
        .map(|p| (p.id.to_string(), p.name.clone()))
        .collect();

    let modality_order: Vec<String> = assets
        // Live side: a Query Group's materialised membership is live-only
        // by definition, and a trash-view listing has no use for an axis
        // ranked by the live corpus either — it is the same order both
        // callers already saw.
        .counts_by_modality(persona, crate::domain::asset::TrashFilter::LiveOnly)
        .await?
        .into_iter()
        .map(|(slug, _)| slug)
        .collect();

    let group_names = groups
        .list(persona)
        .await?
        .into_iter()
        .map(|s| (s.group.id.to_string(), s.group.name))
        .collect();

    Ok(SortContext::new(
        &persona_order,
        persona_names,
        &modality_order,
        group_names,
    ))
}
