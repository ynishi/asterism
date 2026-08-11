//! Redirecting a named id set through the folds that happened after it
//! was written down.
//!
//! [`Asset::folded_into`](crate::domain::asset::Asset::folded_into)
//! states the read rule in one sentence: **paths that enumerate a row
//! drop it, paths that name it keep it.** The enumerating half is a
//! `WHERE` term and lives in the adapter. This module is the naming
//! half, and it exists because reaching the headstone is only the first
//! part of the answer — what a caller does *after* reaching it is where
//! the surfaces disagreed.
//!
//! # Why redirect rather than drop
//!
//! Every caller here holds an id set it did not compute just now: a
//! Snapshot's frozen membership, the ids an export was created against,
//! the members of the freezes an asset once appeared in. Those sets are
//! content, not a query result.
//!
//! - A **fold does not rewrite a Snapshot** (`a_fold_never_rewrites_a_snapshot`
//!   in the SQLite asset repository: "a content-addressed member set
//!   must not be edited by a fold"). So the id set still names the
//!   headstone afterwards, correctly.
//! - **Dropping** the headstone there loses a member. A four-member
//!   freeze becomes a three-member freeze because somebody merged two
//!   rows elsewhere, and the set's identity is gone.
//! - **Redirecting** collapses it onto the keeper, and onto a keeper the
//!   set already holds if it holds one — which is what stopped an export
//!   from receiving the same artefact twice.
//!
//! Redirecting is also just what a fold means. "This row is now that
//! row" is the whole content of `folded_into`; a reader that stops at
//! the headstone has read half of it.
//!
//! # Where this is called from, and where it is not
//!
//! Called from every surface that hands
//! [`AssetRepository::cards_by_ids`](crate::domain::repository::AssetRepository::cards_by_ids)
//! an id set of its **own**: `snapshot_members`, `mint_snapshot`, the
//! dispatch runtime's input slice, and the constellation's
//! `same_selection` synthesis.
//!
//! **Not** called from `hydrate_cards` (`POST /assets/hydrate`), and
//! that is a contract rather than an oversight: its caller is the grid,
//! whose ids come from a `list_index` read that already applied the
//! enumerating half of the rule. Redirecting them a second time would
//! be redundant work on the hottest read in the app.
//!
//! `cards_by_ids` itself is untouched for the reason its own doc gives —
//! it is the twin of `find` by id, deliberately unfiltered, and the
//! trash view's hydration depends on that.

use std::collections::{HashMap, HashSet};

use crate::domain::asset::AssetCard;
use crate::domain::repository::AssetRepository;
use crate::domain::value::{AssetId, Viewer};
use crate::error::DomainError;

/// An id set redirected through its folds, and the cards behind it.
///
/// [`Default`] is the empty answer, for a caller that treats a failed
/// hydration as "no siblings to draw" rather than as an error.
#[derive(Default)]
pub struct NamedCards {
    /// The ids after redirection, in the order they were named, with
    /// duplicates collapsed.
    ///
    /// Duplicates are collapsed **whether or not a fold produced them**:
    /// naming a row twice names one row, and a rule that only deduped
    /// after a fold would answer differently depending on history.
    pub ids: Vec<AssetId>,
    /// The cards for those ids, as a set — free to be in any order, and
    /// shorter than [`ids`](Self::ids) when the viewer cannot see a row
    /// or nothing holds the id at all. A caller that needs the named
    /// order re-projects `cards` onto `ids`.
    pub cards: Vec<AssetCard>,
    /// The redirections this call made, headstone → keeper. Empty when
    /// nothing in the set was folded.
    ///
    /// Carried because a caller that has to say *which* of its own sets
    /// a card came from can only ask about the id it stored, not the one
    /// it got back — the constellation's `same_selection` label is that
    /// caller, and without this it would silently stop naming the freeze
    /// for exactly the members this module redirected.
    pub redirected: HashMap<AssetId, AssetId>,
}

/// Replaces every headstone in `ids` with the row it was folded into,
/// keeps the named order, and collapses duplicates. Returns the
/// redirected ids and the redirections that produced them.
///
/// An id the repository could not resolve — a live row, an id nothing
/// holds, a chain that dead-ends — comes back as itself
/// ([`AssetRepository::resolve_folds`] records why absence is the right
/// answer there).
pub async fn redirect(
    assets: &dyn AssetRepository,
    ids: &[AssetId],
) -> Result<(Vec<AssetId>, HashMap<AssetId, AssetId>), DomainError> {
    let keepers = assets.resolve_folds(ids).await?;
    let mut seen: HashSet<AssetId> = HashSet::with_capacity(ids.len());
    let mut named = Vec::with_capacity(ids.len());
    for id in ids {
        let target = keepers.get(id).copied().unwrap_or(*id);
        if seen.insert(target) {
            named.push(target);
        }
    }
    Ok((named, keepers))
}

/// [`redirect`] followed by the hydration the caller wanted — the one
/// call a surface holding its own id set makes instead of reaching for
/// `cards_by_ids` directly.
///
/// Two round trips at most, and one when nothing in the set was folded
/// (the resolve statement returns no rows).
pub async fn hydrate_named(
    assets: &dyn AssetRepository,
    ids: &[AssetId],
    viewer: &Viewer,
) -> Result<NamedCards, DomainError> {
    let (ids, redirected) = redirect(assets, ids).await?;
    let cards = assets.cards_by_ids(&ids, viewer).await?;
    Ok(NamedCards {
        ids,
        cards,
        redirected,
    })
}
