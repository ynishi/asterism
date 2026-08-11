//! `QueryGroupService` — the Query Group evaluate-and-materialize
//! pipeline.
//!
//! One entry point,
//! [`evaluate_and_materialize`](QueryGroupService::evaluate_and_materialize),
//! runs the whole pass for a single query group:
//!
//! ```text
//! query_json ──parse──▶ QueryGroupQuery
//!            │
//!            ├─ filter ──to_asset_query──▶ AssetQuery
//!            │                 │
//!            │   raw group_ids ─┴─ expand_group_closure (recursive CTE)
//!            │                                │
//!            ├─ search_text ──▶ AssetQuery::text_match (a SQL term)
//!            │                                │
//!            │            fetch_sortable_assets (SQL filter, no LIMIT)
//!            │                                │
//!            ├─ sort ─ SortContext (persona / modality / group lookups)
//!            │                                │
//!            │                        sort_asset_ids  → ordered ids
//!            │                                │
//!            └────────────── replace_membership (bulk DELETE + INSERT)
//! ```
//!
//! # Callers
//!
//! Three of them sit on the transport side and each evaluates the one
//! group it just touched: [`create_query_group`](QueryGroupService::create_query_group)
//! and [`update_query`](QueryGroupService::update_query) (both a Tauri
//! command *and* an HTTP route), plus `DispatchService::run`'s
//! pre-freeze refresh. The fourth is
//! [`QueryGroupRefreshService`](crate::application_support::QueryGroupRefreshService),
//! which loops this over every group for the W4 refresh job and the
//! startup pass — that sweep has no wire surface, which is why it
//! lives in `application_support` while this evaluator stays here.
//! The service touches no jobs infrastructure either way: it is pure
//! orchestration over the repository ports, callable from any context.
//!
//! The two wire verbs take an [`AttributionContext`] they do not
//! persist (no group column carries attribution); the evaluator itself
//! takes none at all — see its own doc comment for why that asymmetry
//! is the doctrine rather than an omission.
//!
//! # A Query Group is defined by predicates only
//!
//! `search_text` resolves to `AssetQuery::text_match`, a `WHERE` term
//! evaluated in SQL alongside the tag / modality / date terms. It is
//! **not** routed through the retrieval port, and this service holds no
//! handle on one.
//!
//! It used to. The `search_text` branch asked for a ranked shortlist and
//! intersected it with the SQL result, which put two properties into a
//! stored set definition that a stored set definition cannot have:
//!
//! - a shortlist is capped by construction, so a text matching more
//!   assets than the ceiling dropped the tail out of the membership with
//!   nothing on screen to say so, and
//! - retrieval promises no determinism, so two refreshes over unchanged
//!   data could name different members.
//!
//! As a predicate both go away — membership is exact, countable, and the
//! same on every refresh.

use std::sync::Arc;

use asterism_contract::command::{CreateQueryGroupCommand, UpdateQueryGroupQueryCommand};
use asterism_contract::dto::GroupDto;
use asterism_contract::query_group::QueryGroupQuery;
use chrono::Utc;

use crate::application::mapping::{group_to_dto, parse_asset_id, parse_persona_id, to_asset_query};
use crate::domain::attribution::AttributionContext;
use crate::domain::group::GroupKind;
use crate::domain::repository::{
    AssetRepository, GroupRepository, PersonaRepository, QueryGroupRepository,
};
use crate::domain::sort_eval::{SortContext, sort_asset_ids};
use crate::domain::value::{GroupId, PersonaId};
use crate::error::DomainError;

/// Application-layer surface for Query Group evaluation.
pub struct QueryGroupService {
    query_groups: Arc<dyn QueryGroupRepository>,
    personas: Arc<dyn PersonaRepository>,
    assets: Arc<dyn AssetRepository>,
    groups: Arc<dyn GroupRepository>,
}

impl QueryGroupService {
    /// Wires the service around its repository ports.
    pub fn new(
        query_groups: Arc<dyn QueryGroupRepository>,
        personas: Arc<dyn PersonaRepository>,
        assets: Arc<dyn AssetRepository>,
        groups: Arc<dyn GroupRepository>,
    ) -> Self {
        Self {
            query_groups,
            personas,
            assets,
            groups,
        }
    }

    /// Evaluates `query_json` and freezes the result into `bucket_id`'s
    /// membership, returning the number of members written.
    ///
    /// **Takes no `AttributionContext`, on purpose.** It writes a derived
    /// value — the membership a stored rule currently resolves to — and
    /// derived writes are outside the set of mutations that receive one
    /// (the same basis that keeps
    /// `thumb_service::put` and the refresh sweep out). The operation
    /// that *is* attributable is the
    /// one that reached here: `create_query_group` / `update_query` /
    /// `DispatchService::run`'s pre-freeze refresh each take a context of
    /// their own, and the W4 refresh sweep is a system write with nothing
    /// to attribute. Giving this method one would mean either handing it
    /// the same context twice or inventing one for the sweep.
    ///
    /// `persona_id` is the query group's owning persona and scopes the
    /// whole evaluation — it overrides any `persona_id` carried inside the
    /// stored filter so the SQL filter and the Tantivy search always agree
    /// on scope. The stored `filter.group_ids` are treated as **raw** and
    /// expanded here; the frozen `position` is the sort
    /// evaluator's output so a single-selected query group renders in its
    /// saved sort order.
    pub async fn evaluate_and_materialize(
        &self,
        query_json: &str,
        persona_id: &PersonaId,
        bucket_id: &GroupId,
    ) -> Result<u64, DomainError> {
        let result = self
            .evaluate_and_materialize_inner(query_json, persona_id, bucket_id)
            .await;
        // W4-b failure signal: every evaluate stamps its outcome
        // on the bucket — the sidebar staleness chip reads it. Wrapped
        // here (not per caller) so the four evaluate paths (create /
        // update / refresh job / pre-dispatch refresh) cannot drift. A
        // stamp failure must not mask the evaluation result — log and
        // fall through.
        let (status, error) = match &result {
            Ok(_) => ("ok", None),
            Err(e) => ("failed", Some(e.to_string())),
        };
        if let Err(stamp_err) = self
            .query_groups
            .mark_refresh_result(bucket_id, status, error.as_deref(), Utc::now())
            .await
        {
            tracing::warn!(
                event = "diag.query_group.stamp_failed",
                bucket_id = %bucket_id,
                error = %stamp_err,
                "mark_refresh_result failed"
            );
        }
        result
    }

    async fn evaluate_and_materialize_inner(
        &self,
        query_json: &str,
        persona_id: &PersonaId,
        bucket_id: &GroupId,
    ) -> Result<u64, DomainError> {
        // 1. Parse the versioned rule (loud on an unknown shape).
        let rule = QueryGroupQuery::parse(query_json)
            .map_err(|e| DomainError::Validation(format!("query_json parse failed: {e}")))?;

        // 2. Build the domain query; force persona scope + full evaluation.
        let mut query = to_asset_query(&rule.filter)?;
        query.persona_id = Some(*persona_id);
        query.offset = 0;
        query.limit = u64::MAX; // ignored by fetch_sortable_assets (no LIMIT)
        // The rule's full-text condition is a **predicate**, resolved in
        // SQL beside the other filters. It used to be a retrieval
        // — a ranked shortlist intersected with the SQL result — which
        // meant a group's membership was capped by the shortlist and
        // could differ between two refreshes over unchanged data. A
        // stored set definition cannot be built out of an answer that
        // makes no completeness or determinism promise.
        //
        // `search_text` keeps its wire name (stored rules carry it) and
        // lands on the same predicate the grid's exact box uses, so a
        // rule and a filter bar spelling the same text select the same
        // assets.
        if let Some(text) = rule.search_text.as_deref().map(str::trim)
            && !text.is_empty()
        {
            query.text_match = Some(text.to_string());
        }
        // A stored rule is persisted JSON, so it can carry any wire
        // field the contract accepts — including a trash selector. Pin
        // it: materialising trashed assets into `asset_bucket` would
        // burn positions for rows the grid will not show, and a Query
        // Group is a live view by definition.
        query.trash = crate::domain::asset::TrashFilter::LiveOnly;

        // 3. Nesting expansion — raw group_ids → descendant closure.
        query.group_ids = self
            .query_groups
            .expand_group_closure(&query.group_ids)
            .await?;

        // 4. SQL filter, whole set (no LIMIT). The full-text condition
        //    is one of the terms in it now, so this single fetch is the
        //    membership — exact, and the same on every refresh over
        //    unchanged data.
        let assets = self.query_groups.fetch_sortable_assets(&query).await?;

        // 6. Sort — build the lookup context from the repos, then order.
        let ctx = self.build_sort_context(persona_id).await?;
        let ordered = sort_asset_ids(&rule.sort, &assets, &ctx)?;

        // 7. Materialize — bulk replace membership with burned-in positions.
        let ids = ordered
            .iter()
            .map(|s| parse_asset_id(s))
            .collect::<Result<Vec<_>, _>>()?;
        self.query_groups
            .replace_membership(bucket_id, &ids, Utc::now())
            .await
    }

    /// "Save as Group": validates the rule, mints the
    /// `kind='query'` Group, and evaluates it synchronously once so the
    /// group is never visible with empty members (refresh-job wiring is
    /// W4; the synchronous first evaluation is the standing guarantee).
    ///
    /// No cycle check is needed at create time — a freshly minted group
    /// has no inbound edges (nothing references or contains it yet), so
    /// no path can lead back to it.
    ///
    /// The create and the first evaluation are not one transaction: an
    /// evaluation failure surfaces as the command's error while the
    /// (member-less) group row remains — loud, visible, and repairable
    /// via "Update query" or the startup refresh.
    pub async fn create_query_group(
        &self,
        command: CreateQueryGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<GroupDto, DomainError> {
        let persona_id = parse_persona_id(&command.persona_id)?;
        if self.personas.find(&persona_id).await?.is_none() {
            return Err(DomainError::PersonaNotFound(persona_id));
        }
        QueryGroupQuery::parse(&command.query_json)
            .map_err(|e| DomainError::Validation(format!("query_json invalid: {e}")))?;
        let group = self
            .query_groups
            .create_query_group(
                persona_id,
                command.name,
                command.query_json.clone(),
                Utc::now(),
            )
            .await?;
        self.evaluate_and_materialize(&command.query_json, &persona_id, &group.id)
            .await?;
        Ok(group_to_dto(&group))
    }

    /// "Update query": validates the replacement rule, rejects a
    /// rule that would close a dependency cycle (write-site (a)),
    /// persists it, and synchronously re-evaluates the membership.
    pub async fn update_query(
        &self,
        command: UpdateQueryGroupQueryCommand,
        _attribution: &AttributionContext,
    ) -> Result<GroupDto, DomainError> {
        let group_id = crate::application::mapping::parse_group_id(&command.group_id)?;
        let group = self
            .groups
            .find(&group_id)
            .await?
            .ok_or_else(|| DomainError::not_found("group", group_id))?;
        if group.kind != GroupKind::Query {
            return Err(DomainError::Validation(
                "update_query targets a query group; this group is manual".into(),
            ));
        }
        QueryGroupQuery::parse(&command.query_json)
            .map_err(|e| DomainError::Validation(format!("query_json invalid: {e}")))?;

        // The composite cycle guard (write-site (a)) lives inside
        // `set_query_json` — same isle call as the write, so
        // check-then-write is atomic on the serialized writer.
        self.query_groups
            .set_query_json(&group_id, &command.query_json, Utc::now())
            .await?;
        self.evaluate_and_materialize(&command.query_json, &group.persona_id, &group_id)
            .await?;
        let updated = self
            .groups
            .find(&group_id)
            .await?
            .ok_or_else(|| DomainError::Validation(format!("group vanished: {group_id}")))?;
        Ok(group_to_dto(&updated))
    }

    /// Persona-scoped [`SortContext`], delegated to the shared builder so
    /// this evaluator and `AssetService::list` cannot drift apart on what a
    /// given [`SortSpec`](asterism_contract::sort::SortSpec) means
    /// (`application::sort_context`).
    async fn build_sort_context(&self, persona_id: &PersonaId) -> Result<SortContext, DomainError> {
        crate::application::sort_context::build_sort_context(
            &*self.personas,
            &*self.assets,
            &*self.groups,
            Some(persona_id),
        )
        .await
    }
}
