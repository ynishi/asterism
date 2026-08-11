//! `SnapshotService` — application surface for the immutable `Snapshot`
//! entity, reborn from the old `SelectionService`.
//!
//! - [`create`](SnapshotService::create) — *internal* materialise of a
//!   new Snapshot from a set of picked assets. No public command exposes
//!   it (the create surface was deliberately made internal); the
//!   dispatch / promote handlers are
//!   its only callers.
//! - [`get_snapshot`](SnapshotService::get_snapshot) — fetch one freeze
//!   by id (opening it from its referencing source).
//! - [`list_containing`](SnapshotService::list_containing) — the P5
//!   reverse lookup (`asset → freezes that include it`).
//! - [`promote_to_group`](SnapshotService::promote_to_group) —
//!   materialise a hand-owned Group from a freeze's members.
//! - [`promote_volatile_selection`](SnapshotService::promote_volatile_selection)
//!   — freeze the grid's volatile pick and promote it in one step
//!   (right-click "Group-ify selection", W5-d).
//!
//! Snapshots have no list / rename / delete surface; deletion is
//! the later GC job's concern, so nothing here removes rows.
//!
//! Every write here takes an [`AttributionContext`] it does not persist:
//! neither `snapshot` nor the Group a promote mints carries an
//! attribution column, and none is being added (see the
//! [`application`](crate::application) module doc for why the argument
//! is required anyway).

use std::sync::Arc;

use asterism_contract::command::{
    CreateSnapshotCommand, PromoteSnapshotToGroupCommand, PromoteSnapshotToGroupResult,
    PromoteVolatileSelectionCommand,
};
use asterism_contract::dto::SnapshotDto;
use chrono::Utc;

use crate::application::mapping::{
    parse_asset_id, parse_dir_id, parse_persona_id, parse_snapshot_id, snapshot_to_dto,
};
use crate::domain::attribution::AttributionContext;
use crate::domain::repository::{
    AssetRepository, GroupRepository, PersonaRepository, SnapshotRepository,
};
use crate::domain::snapshot::Snapshot;
use crate::domain::value::Viewer;
use crate::error::DomainError;

/// Application-layer surface for `Snapshot`.
pub struct SnapshotService {
    snapshots: Arc<dyn SnapshotRepository>,
    personas: Arc<dyn PersonaRepository>,
    assets: Arc<dyn AssetRepository>,
    groups: Arc<dyn GroupRepository>,
    /// Query Group invalidator (W4). Fired after `promote_to_group`
    /// mints a new manual bucket + attaches its members — those writes
    /// change what `group_ids` filters can resolve to, exactly like
    /// `AssetService::add_asset_to_group`.
    query_group_invalidator: crate::application::query_group_invalidation::QueryGroupInvalidator,
}

impl SnapshotService {
    /// Wires the service around its repository ports.
    pub fn new(
        snapshots: Arc<dyn SnapshotRepository>,
        personas: Arc<dyn PersonaRepository>,
        assets: Arc<dyn AssetRepository>,
        groups: Arc<dyn GroupRepository>,
        query_group_invalidator: crate::application::query_group_invalidation::QueryGroupInvalidator,
    ) -> Self {
        Self {
            snapshots,
            personas,
            assets,
            groups,
            query_group_invalidator,
        }
    }

    /// Internal materialise of a Snapshot from a set of picked assets.
    ///
    /// The members are frozen in the caller's pick order and deduped on
    /// `(persona_id, content_hash)` by the repository (validation lives
    /// in [`mint_snapshot`](Self::mint_snapshot)).
    pub async fn create(
        &self,
        command: CreateSnapshotCommand,
        _attribution: &AttributionContext,
    ) -> Result<SnapshotDto, DomainError> {
        let stored = self
            .mint_snapshot(&command.persona_id, &command.asset_ids)
            .await?;
        Ok(snapshot_to_dto(&stored))
    }

    /// Validates a caller-picked asset list and freezes it as a
    /// (deduped) Snapshot. Shared by [`create`](Self::create) and
    /// [`promote_volatile_selection`](Self::promote_volatile_selection).
    ///
    /// Validates:
    /// - `persona_id` exists.
    /// - Every asset in `asset_ids` exists and belongs to `persona_id`
    ///   (a Snapshot cannot span personas).
    /// - `asset_ids` is non-empty (delegated to [`Snapshot::new`]).
    async fn mint_snapshot(
        &self,
        persona_id: &str,
        asset_ids: &[String],
    ) -> Result<Snapshot, DomainError> {
        let persona_id = parse_persona_id(persona_id)?;
        if self.personas.find(&persona_id).await?.is_none() {
            return Err(DomainError::PersonaNotFound(persona_id));
        }
        if asset_ids.is_empty() {
            return Err(DomainError::Validation(
                "snapshot asset_ids must not be empty".into(),
            ));
        }
        // Resolve card projections in one round trip so the cross-persona
        // guard costs O(1) queries regardless of the pick size.
        let parsed: Vec<_> = asset_ids
            .iter()
            .map(|s| parse_asset_id(s))
            .collect::<Result<Vec<_>, _>>()?;
        // A caller picking rows off a stale panel can name a row that has
        // since been folded. Freezing that id would mint a set whose
        // members the grid refuses to draw; redirecting freezes what the
        // ids now name (`fold_redirect`), which is also what makes
        // "the keeper and its headstone" one member rather than two.
        let named = crate::application::fold_redirect::hydrate_named(
            self.assets.as_ref(),
            &parsed,
            &Viewer::Owner,
        )
        .await?;
        if named.cards.len() != named.ids.len() {
            return Err(DomainError::Validation(
                "snapshot asset_ids contains ids the viewer cannot see or that do not exist".into(),
            ));
        }
        for card in &named.cards {
            if card.persona_id != persona_id {
                return Err(DomainError::Validation(
                    "snapshot asset_ids must all belong to persona_id".into(),
                ));
            }
        }
        // `named.ids` is in the caller's input order (`cards_by_ids` does
        // not guarantee order); the frozen membership uses it.
        let snapshot = Snapshot::new(persona_id, named.ids, Utc::now())?;
        self.snapshots.create_or_reuse(&snapshot).await
    }

    /// Reverse lookup — every Snapshot whose frozen membership contains
    /// `asset_id` (P5). Used by the detail panel to render "this asset
    /// appears in these freezes" chips.
    pub async fn list_containing(
        &self,
        asset_id: &str,
        limit: u32,
    ) -> Result<Vec<SnapshotDto>, DomainError> {
        let aid = parse_asset_id(asset_id)?;
        let rows = self.snapshots.list_containing_asset(&aid, limit).await?;
        Ok(rows.iter().map(snapshot_to_dto).collect())
    }

    /// Fetches one Snapshot as its wire DTO (`snapshot_get` —
    /// the Snapshot view's metadata + frozen ids).
    pub async fn get_snapshot(&self, id: &str) -> Result<SnapshotDto, DomainError> {
        let sid = parse_snapshot_id(id)?;
        let snapshot = self
            .snapshots
            .find(&sid)
            .await?
            .ok_or_else(|| DomainError::not_found("snapshot", id))?;
        Ok(snapshot_to_dto(&snapshot))
    }

    /// The freeze's members as renderable cards, in frozen `position`
    /// order (`snapshot_members` — the read-only member grid of
    /// the Snapshot view). Members whose assets were later deleted are
    /// simply absent from the result (the freeze keeps their ids; the
    /// cards cannot be hydrated).
    ///
    /// A member that has since been folded is drawn as **the keeper**,
    /// not as the headstone and not as a gap. The freeze is not rewritten
    /// by a fold (`a_fold_never_rewrites_a_snapshot`), so the stored id
    /// still names the headstone and this read is where it is redirected
    /// — see [`fold_redirect`](crate::application::fold_redirect) for why
    /// redirecting rather than dropping is the answer for a
    /// content-addressed member set. A freeze that named both a keeper
    /// and a row later folded into it therefore draws one card.
    pub async fn snapshot_members(
        &self,
        id: &str,
    ) -> Result<Vec<asterism_contract::dto::AssetCardDto>, DomainError> {
        let sid = parse_snapshot_id(id)?;
        let snapshot = self
            .snapshots
            .find(&sid)
            .await?
            .ok_or_else(|| DomainError::not_found("snapshot", id))?;
        let named = crate::application::fold_redirect::hydrate_named(
            self.assets.as_ref(),
            &snapshot.asset_ids,
            &Viewer::Owner,
        )
        .await?;
        // `cards_by_ids` does not guarantee order — restore the frozen
        // order (position is content), reading it off the redirected
        // ids so a keeper stands where its headstone was named.
        let mut by_id: std::collections::HashMap<String, _> = named
            .cards
            .into_iter()
            .map(|c| (c.id.to_string(), c))
            .collect();
        let ordered = named
            .ids
            .iter()
            .filter_map(|aid| by_id.remove(&aid.to_string()))
            .map(|c| crate::application::mapping::card_to_dto(&c))
            .collect();
        Ok(ordered)
    }

    /// Materialises a hand-owned Group from a freeze's members
    /// (promote / "Group-ify"): mint the Group, bulk-attach the
    /// members in frozen order, and stamp the birth record
    /// (`origin_snapshot_id` — the direction-flipped successor of the
    /// old `promoted_group_id`).
    pub async fn promote_to_group(
        &self,
        command: PromoteSnapshotToGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<PromoteSnapshotToGroupResult, DomainError> {
        let sid = parse_snapshot_id(&command.snapshot_id)?;
        let snapshot = self
            .snapshots
            .find(&sid)
            .await?
            .ok_or_else(|| DomainError::not_found("snapshot", &command.snapshot_id))?;
        self.promote_snapshot(&snapshot, command.name, command.description, command.dir_id)
            .await
    }

    /// Fuses freeze + promote for the grid's volatile pick (right-click
    /// "Group-ify selection", W5-d): mints a Snapshot from the picked
    /// ids (content-hash deduped) and promotes it into a hand-owned
    /// Group in one call. A promote-half failure (e.g. group-name
    /// conflict) leaves the minted Snapshot behind — acceptable by
    /// design: it is a nameless content object the GC job reclaims.
    pub async fn promote_volatile_selection(
        &self,
        command: PromoteVolatileSelectionCommand,
        _attribution: &AttributionContext,
    ) -> Result<PromoteSnapshotToGroupResult, DomainError> {
        let snapshot = self
            .mint_snapshot(&command.persona_id, &command.asset_ids)
            .await?;
        self.promote_snapshot(&snapshot, command.name, command.description, command.dir_id)
            .await
    }

    /// Promote body shared by [`promote_to_group`](Self::promote_to_group)
    /// and [`promote_volatile_selection`](Self::promote_volatile_selection):
    /// mint the Group, bulk-attach the frozen members, stamp the birth
    /// record, and invalidate the persona's query groups.
    async fn promote_snapshot(
        &self,
        snapshot: &Snapshot,
        name: String,
        description: Option<String>,
        dir_id: Option<String>,
    ) -> Result<PromoteSnapshotToGroupResult, DomainError> {
        let dir_id = dir_id.as_deref().map(parse_dir_id).transpose()?;
        let now = Utc::now();
        // Create the Group (name uniqueness is enforced by the
        // GroupRepository via a `Conflict` error — surfaces the same way
        // as a manual create).
        let group = self
            .groups
            .create(snapshot.persona_id, name, description, now)
            .await?;
        if let Some(dir) = &dir_id {
            self.groups.set_dir(&group.id, Some(dir), now).await?;
        }
        let attached = self
            .groups
            .add_bulk(&group.id, &snapshot.asset_ids, now)
            .await?;
        self.groups
            .set_origin_snapshot(&group.id, &snapshot.id, now)
            .await?;
        // A new manual group + bulk member attach changes what every
        // Query Group's `group_ids` filter can resolve to.
        self.query_group_invalidator
            .notify_persona(snapshot.persona_id);
        Ok(PromoteSnapshotToGroupResult {
            group_id: group.id.to_string(),
            snapshot_id: snapshot.id.to_string(),
            name: group.name,
            asset_count: attached,
        })
    }
}
