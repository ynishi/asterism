//! SQLite adapter for the `EdgeRepository` port.
//!
//! The port boundary is intentionally narrow so an alternative graph
//! backend could sit behind it later. v1 stores edges in the dedicated
//! `edge` table introduced in schema v1.

use asterism_core::domain::edge::{ConstellationEdge, EdgeDirection, EdgeKind, IncidentEdge};
use asterism_core::domain::repository::EdgeRepository;
use asterism_core::domain::value::{AssetId, EdgeId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::fault::StoreFault;
use crate::sqlite::map::infra_err;

/// Bind-parameter tuple of one edge insert, in column order.
type EdgeParams = (Uuid, Uuid, Uuid, String, Option<String>, Option<f64>);

/// Primitive row built inside the isle closure.
struct EdgeRow {
    id: Uuid,
    from_asset: Uuid,
    to_asset: Uuid,
    kind: String,
    label: Option<String>,
    weight: Option<f64>,
}

impl EdgeRow {
    const COLUMNS: &'static str = "id, from_asset, to_asset, kind, label, weight";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            from_asset: row.get(1)?,
            to_asset: row.get(2)?,
            kind: row.get(3)?,
            label: row.get(4)?,
            weight: row.get(5)?,
        })
    }

    fn into_domain(self) -> Result<ConstellationEdge, DomainError> {
        Ok(ConstellationEdge {
            id: EdgeId::from_uuid(self.id),
            from: AssetId::from_uuid(self.from_asset),
            to: AssetId::from_uuid(self.to_asset),
            kind: StoreFault::parsed("edge kind", EdgeKind::parse(&self.kind))?,
            label: self.label,
            weight: self.weight.map(|w| w as f32),
        })
    }
}

/// SQLite adapter for `EdgeRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqliteEdgeRepository {
    isle: AsyncIsle,
}

impl SqliteEdgeRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }

    /// The shared unit of work of the two rebuild ports: atomically
    /// replace the edges of `owned_kinds` that originate from
    /// `asset_id`. The `kind IN (…)` clause is what keeps everything
    /// outside the owning rebuild's subset alive across a pass, and
    /// edges outside the subset are dropped at the door rather than
    /// persisted where the next pass would delete them.
    async fn replace_owned_kinds(
        &self,
        asset_id: &AssetId,
        edges: Vec<ConstellationEdge>,
        owned_kinds: &'static [EdgeKind],
    ) -> Result<(), DomainError> {
        let uuid = *asset_id.as_uuid();
        let rows: Vec<EdgeParams> = edges
            .iter()
            .filter(|e| owned_kinds.contains(&e.kind))
            .map(|e| {
                (
                    *e.id.as_uuid(),
                    *e.from.as_uuid(),
                    *e.to.as_uuid(),
                    e.kind.as_str().to_string(),
                    e.label.clone(),
                    e.weight.map(|w| w as f64),
                )
            })
            .collect();
        let kind_slugs: Vec<String> = owned_kinds.iter().map(|k| k.as_str().to_string()).collect();
        self.isle
            .call(move |conn| {
                let placeholders = std::iter::repeat_n("?", kind_slugs.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let delete_sql =
                    format!("DELETE FROM edge WHERE from_asset = ?1 AND kind IN ({placeholders})");
                let tx = conn.transaction()?;
                {
                    let mut delete_params: Vec<&dyn rusqlite::ToSql> =
                        Vec::with_capacity(kind_slugs.len() + 1);
                    delete_params.push(&uuid);
                    for kind in &kind_slugs {
                        delete_params.push(kind);
                    }
                    tx.execute(&delete_sql, delete_params.as_slice())?;
                }
                {
                    let mut stmt = tx.prepare(
                        "INSERT OR IGNORE INTO edge (id, from_asset, to_asset, kind, label, weight)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )?;
                    for (id, from, to, kind, label, weight) in &rows {
                        stmt.execute(params![id, from, to, kind, label, weight])?;
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }
}

#[async_trait]
impl EdgeRepository for SqliteEdgeRepository {
    async fn edges_of(
        &self,
        asset_id: &AssetId,
        kind: Option<EdgeKind>,
        limit: u32,
    ) -> Result<Vec<ConstellationEdge>, DomainError> {
        let uuid = *asset_id.as_uuid();
        let kind_slug = kind.map(|k| k.as_str().to_string());
        let rows = self
            .isle
            .call(move |conn| {
                // NULL weights sort last (the hover burst picks the
                // top N by weight).
                let sql = match &kind_slug {
                    Some(_) => format!(
                        "SELECT {} FROM edge WHERE from_asset = ?1 AND kind = ?2
                         ORDER BY weight IS NULL, weight DESC LIMIT ?3",
                        EdgeRow::COLUMNS
                    ),
                    None => format!(
                        "SELECT {} FROM edge WHERE from_asset = ?1
                         ORDER BY weight IS NULL, weight DESC LIMIT ?2",
                        EdgeRow::COLUMNS
                    ),
                };
                let mut stmt = conn.prepare(&sql)?;
                let rows = match &kind_slug {
                    Some(slug) => stmt
                        .query_map(params![uuid, slug, limit as i64], EdgeRow::from_row)?
                        .collect::<Result<Vec<_>, _>>()?,
                    None => stmt
                        .query_map(params![uuid, limit as i64], EdgeRow::from_row)?
                        .collect::<Result<Vec<_>, _>>()?,
                };
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(EdgeRow::into_domain).collect()
    }

    async fn edges_incident(
        &self,
        asset_id: &AssetId,
        kind: Option<EdgeKind>,
        limit: u32,
    ) -> Result<Vec<IncidentEdge>, DomainError> {
        let uuid = *asset_id.as_uuid();
        let kind_slug = kind.map(|k| k.as_str().to_string());
        let rows = self
            .isle
            .call(move |conn| {
                // `OR` guarantees we pick up edges where the queried
                // asset sits on either side. The union of
                // `idx_edge_from(from_asset, kind, weight DESC)` and
                // `idx_edge_to_kind_weight(to_asset, kind, weight DESC)`
                // (added in schema V3) lets the planner serve both
                // sides from an index. NULL weights sort last so the
                // hover burst's top-N stays weight-descending.
                let sql = match &kind_slug {
                    Some(_) => format!(
                        "SELECT {} FROM edge
                         WHERE (from_asset = ?1 OR to_asset = ?1) AND kind = ?2
                         ORDER BY weight IS NULL, weight DESC LIMIT ?3",
                        EdgeRow::COLUMNS
                    ),
                    None => format!(
                        "SELECT {} FROM edge
                         WHERE from_asset = ?1 OR to_asset = ?1
                         ORDER BY weight IS NULL, weight DESC LIMIT ?2",
                        EdgeRow::COLUMNS
                    ),
                };
                let mut stmt = conn.prepare(&sql)?;
                let rows = match &kind_slug {
                    Some(slug) => stmt
                        .query_map(params![uuid, slug, limit as i64], EdgeRow::from_row)?
                        .collect::<Result<Vec<_>, _>>()?,
                    None => stmt
                        .query_map(params![uuid, limit as i64], EdgeRow::from_row)?
                        .collect::<Result<Vec<_>, _>>()?,
                };
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;

        rows.into_iter()
            .map(|r| {
                let direction = if r.from_asset == uuid {
                    EdgeDirection::Outgoing
                } else {
                    EdgeDirection::Incoming
                };
                let edge = r.into_domain()?;
                Ok(IncidentEdge { edge, direction })
            })
            .collect()
    }

    async fn replace_synth_edges_of(
        &self,
        asset_id: &AssetId,
        edges: Vec<ConstellationEdge>,
    ) -> Result<(), DomainError> {
        // The windowed rebuild owns exactly the windowed kinds: an
        // asserted `derived_from` must survive it, and so must a
        // `visual_similarity` the visual rebuild derived from vectors
        // this job knows nothing about.
        self.replace_owned_kinds(asset_id, edges, EdgeKind::windowed_synth_kinds())
            .await
    }

    async fn replace_visual_edges_of(
        &self,
        asset_id: &AssetId,
        edges: Vec<ConstellationEdge>,
    ) -> Result<(), DomainError> {
        self.replace_owned_kinds(asset_id, edges, EdgeKind::visual_synth_kinds())
            .await
    }

    async fn add_edges(&self, edges: Vec<ConstellationEdge>) -> Result<(), DomainError> {
        if edges.is_empty() {
            return Ok(());
        }
        let rows: Vec<EdgeParams> = edges
            .iter()
            .map(|e| {
                (
                    *e.id.as_uuid(),
                    *e.from.as_uuid(),
                    *e.to.as_uuid(),
                    e.kind.as_str().to_string(),
                    e.label.clone(),
                    e.weight.map(|w| w as f64),
                )
            })
            .collect();
        self.isle
            .call(move |conn| {
                // One transaction so a multi-parent assertion (an
                // N-input export) lands whole or not at all. `OR
                // IGNORE` leans on `UNIQUE (from_asset, to_asset,
                // kind)`: re-stating the same link is how a retried
                // ingest behaves, and it must not double the edge.
                let tx = conn.transaction()?;
                {
                    let mut stmt = tx.prepare(
                        "INSERT OR IGNORE INTO edge (id, from_asset, to_asset, kind, label, weight)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )?;
                    for (id, from, to, kind, label, weight) in &rows {
                        stmt.execute(params![id, from, to, kind, label, weight])?;
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;

    /// Seed one persona + two assets and return their ids.
    async fn seed_two_assets(isle: &AsyncIsle) -> (AssetId, AssetId) {
        let persona_uuid = Uuid::now_v7();
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        isle.call(move |conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p', 'P', 0, 0)",
                params![persona_uuid],
            )?;
            tx.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', 'a.png', 'image', 0, 0, 0)",
                params![a, persona_uuid],
            )?;
            tx.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', 'a.png#prompt', 'memory', 0, 0, 0)",
                params![b, persona_uuid],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
        (AssetId::from_uuid(a), AssetId::from_uuid(b))
    }

    #[tokio::test]
    async fn edges_incident_surfaces_both_directions() {
        // Simulates the write pattern that `edge_rebuild` produces:
        // an edge is written only from the *newer* asset (`b`) to
        // the older sibling (`a`). Neither side should be blind to
        // the link — that is the whole point of `edges_incident`.
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteEdgeRepository::new(isle.clone());
        let (a, b) = seed_two_assets(&isle).await;

        let mut edge = ConstellationEdge::new(b, a, EdgeKind::TimeProximity).unwrap();
        edge.weight = Some(1.0);
        edge.label = Some("same-session".into());
        repo.replace_synth_edges_of(&b, vec![edge]).await.unwrap();

        // From newer side: outgoing.
        let from_b = repo.edges_incident(&b, None, 10).await.unwrap();
        assert_eq!(from_b.len(), 1);
        assert_eq!(from_b[0].direction, EdgeDirection::Outgoing);
        assert_eq!(from_b[0].other_side(), a);

        // From older side: incoming (this is what `edges_of` misses).
        let from_a = repo.edges_incident(&a, None, 10).await.unwrap();
        assert_eq!(from_a.len(), 1);
        assert_eq!(from_a[0].direction, EdgeDirection::Incoming);
        assert_eq!(from_a[0].other_side(), b);

        // Baseline: legacy `edges_of` only sees the outgoing side.
        let legacy_a = repo.edges_of(&a, None, 10).await.unwrap();
        assert!(legacy_a.is_empty());
        let legacy_b = repo.edges_of(&b, None, 10).await.unwrap();
        assert_eq!(legacy_b.len(), 1);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn edges_incident_kind_filter_narrows_result() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteEdgeRepository::new(isle.clone());
        let (a, b) = seed_two_assets(&isle).await;

        let mut e_time = ConstellationEdge::new(b, a, EdgeKind::TimeProximity).unwrap();
        e_time.weight = Some(1.0);
        let mut e_kw = ConstellationEdge::new(b, a, EdgeKind::KeywordOverlap).unwrap();
        e_kw.weight = Some(0.5);
        repo.replace_synth_edges_of(&b, vec![e_time, e_kw])
            .await
            .unwrap();

        // Filter narrows from-both-sides.
        let filtered = repo
            .edges_incident(&a, Some(EdgeKind::TimeProximity), 10)
            .await
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].edge.kind, EdgeKind::TimeProximity);
        assert_eq!(filtered[0].direction, EdgeDirection::Incoming);

        // Same filter on the writer side is also fine.
        let filtered_b = repo
            .edges_incident(&b, Some(EdgeKind::KeywordOverlap), 10)
            .await
            .unwrap();
        assert_eq!(filtered_b.len(), 1);
        assert_eq!(filtered_b[0].edge.kind, EdgeKind::KeywordOverlap);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn edges_incident_ranks_by_weight_desc_with_nulls_last() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteEdgeRepository::new(isle.clone());
        let (a, b) = seed_two_assets(&isle).await;

        let mut edge_null = ConstellationEdge::new(b, a, EdgeKind::TimeProximity).unwrap();
        edge_null.weight = None;
        let mut edge_low = ConstellationEdge::new(b, a, EdgeKind::KeywordOverlap).unwrap();
        edge_low.weight = Some(0.3);
        let mut edge_hi = ConstellationEdge::new(b, a, EdgeKind::Cadence).unwrap();
        edge_hi.weight = Some(0.9);
        repo.replace_synth_edges_of(&b, vec![edge_null, edge_low, edge_hi])
            .await
            .unwrap();

        let out = repo.edges_incident(&a, None, 10).await.unwrap();
        assert_eq!(out.len(), 3);
        // Sorted weight-desc with NULL last.
        assert_eq!(out[0].edge.kind, EdgeKind::Cadence);
        assert_eq!(out[1].edge.kind, EdgeKind::KeywordOverlap);
        assert_eq!(out[2].edge.kind, EdgeKind::TimeProximity);
        assert!(out[2].edge.weight.is_none());

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_rebuild_replaces_synth_edges_but_spares_provenance() {
        // The regression this scoping exists for: an asserted
        // `derived_from` used to be collateral damage of the next
        // `edge_rebuild`, and nothing could recompute it.
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteEdgeRepository::new(isle.clone());
        let (a, b) = seed_two_assets(&isle).await;

        let mut provenance = ConstellationEdge::new(b, a, EdgeKind::DerivedFrom).unwrap();
        provenance.label = Some("dispatch:file".into());
        provenance.weight = Some(1.0);
        repo.add_edges(vec![provenance]).await.unwrap();

        let mut stale = ConstellationEdge::new(b, a, EdgeKind::TimeProximity).unwrap();
        stale.weight = Some(0.2);
        repo.replace_synth_edges_of(&b, vec![stale]).await.unwrap();

        // A later rebuild lands a different synth set: the old synth
        // edge is gone, the assertion is still there.
        let mut fresh = ConstellationEdge::new(b, a, EdgeKind::KeywordOverlap).unwrap();
        fresh.weight = Some(0.6);
        repo.replace_synth_edges_of(&b, vec![fresh]).await.unwrap();

        let kinds: Vec<EdgeKind> = repo
            .edges_of(&b, None, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert!(kinds.contains(&EdgeKind::DerivedFrom), "{kinds:?}");
        assert!(kinds.contains(&EdgeKind::KeywordOverlap), "{kinds:?}");
        assert!(!kinds.contains(&EdgeKind::TimeProximity), "{kinds:?}");

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_rebuild_path_refuses_to_write_provenance() {
        // Provenance written through the rebuild port would be deleted
        // by the very next pass, so it is dropped at the door rather
        // than persisted somewhere it cannot survive.
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteEdgeRepository::new(isle.clone());
        let (a, b) = seed_two_assets(&isle).await;

        let smuggled = ConstellationEdge::new(b, a, EdgeKind::DerivedFrom).unwrap();
        let mut synth = ConstellationEdge::new(b, a, EdgeKind::TimeProximity).unwrap();
        synth.weight = Some(0.4);
        repo.replace_synth_edges_of(&b, vec![smuggled, synth])
            .await
            .unwrap();

        let kinds: Vec<EdgeKind> = repo
            .edges_of(&b, None, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, vec![EdgeKind::TimeProximity]);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn restating_the_same_assertion_does_not_double_it() {
        // A retried ingest re-declares the same parent. One link.
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteEdgeRepository::new(isle.clone());
        let (a, b) = seed_two_assets(&isle).await;

        let first = ConstellationEdge::new(b, a, EdgeKind::DerivedFrom).unwrap();
        repo.add_edges(vec![first]).await.unwrap();
        // Fresh edge id, same (from, to, kind) — this is what a retry
        // produces, and `UNIQUE` is what absorbs it.
        let retry = ConstellationEdge::new(b, a, EdgeKind::DerivedFrom).unwrap();
        repo.add_edges(vec![retry]).await.unwrap();

        let edges = repo
            .edges_of(&b, Some(EdgeKind::DerivedFrom), 10)
            .await
            .unwrap();
        assert_eq!(edges.len(), 1);

        driver.shutdown().await.unwrap();
    }

    /// `identical_to` needs no migration — `edge.kind` is a plain
    /// `TEXT NOT NULL` with no CHECK and no enum — but "needs no
    /// migration" is a claim about a live database, so it is made by
    /// writing one and reading the stored slug back.
    ///
    /// The pair is oriented newcomer (`b`) → incumbent (`a`), the
    /// direction [`EdgeKind::IdenticalTo`] fixes, and the incumbent's
    /// side is checked through `edges_incident`: `edges_of(a)` cannot
    /// see an edge that points *at* `a`, and the incumbent is the row a
    /// user is most likely looking at.
    #[tokio::test]
    async fn an_identical_to_edge_round_trips_and_does_not_double() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteEdgeRepository::new(isle.clone());
        let (a, b) = seed_two_assets(&isle).await;

        let mut edge = ConstellationEdge::new(b, a, EdgeKind::IdenticalTo).unwrap();
        // Detection walks three axes, and the label says which one
        // agreed — the same slug the queue row carries.
        edge.label = Some("artefact".into());
        repo.add_edges(vec![edge]).await.unwrap();

        // The slug SQLite holds, not the one the enum would produce on
        // the way back out.
        let stored: Vec<(String, Option<String>)> = isle
            .call(|conn| {
                let mut stmt = conn.prepare("SELECT kind, label FROM edge")?;
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<_, _>>()
            })
            .await
            .unwrap();
        assert_eq!(
            stored,
            vec![("identical_to".to_string(), Some("artefact".to_string()))]
        );

        // Newcomer side: outgoing. Incumbent side: incoming — and
        // invisible to the outgoing-only read.
        let from_newcomer = repo
            .edges_incident(&b, Some(EdgeKind::IdenticalTo), 10)
            .await
            .unwrap();
        assert_eq!(from_newcomer.len(), 1);
        assert_eq!(from_newcomer[0].direction, EdgeDirection::Outgoing);
        assert_eq!(from_newcomer[0].other_side(), a);
        assert_eq!(from_newcomer[0].edge.kind, EdgeKind::IdenticalTo);
        assert_eq!(from_newcomer[0].edge.label.as_deref(), Some("artefact"));

        let from_incumbent = repo
            .edges_incident(&a, Some(EdgeKind::IdenticalTo), 10)
            .await
            .unwrap();
        assert_eq!(from_incumbent.len(), 1);
        assert_eq!(from_incumbent[0].direction, EdgeDirection::Incoming);
        assert_eq!(from_incumbent[0].other_side(), b);
        assert!(
            repo.edges_of(&a, Some(EdgeKind::IdenticalTo), 10)
                .await
                .unwrap()
                .is_empty(),
            "the outgoing-only read is blind from the incumbent's side"
        );

        // Re-detecting the same pair (a re-run of the hash job) is one
        // edge, absorbed by UNIQUE(from, to, kind).
        let again = ConstellationEdge::new(b, a, EdgeKind::IdenticalTo).unwrap();
        repo.add_edges(vec![again]).await.unwrap();
        assert_eq!(
            repo.edges_of(&b, Some(EdgeKind::IdenticalTo), 10)
                .await
                .unwrap()
                .len(),
            1
        );

        // A rebuild must not take it: nothing recomputes a hash
        // agreement over the window that produced it.
        let mut synth = ConstellationEdge::new(b, a, EdgeKind::TimeProximity).unwrap();
        synth.weight = Some(0.5);
        repo.replace_synth_edges_of(&b, vec![synth]).await.unwrap();
        let kinds: Vec<EdgeKind> = repo
            .edges_of(&b, None, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert!(kinds.contains(&EdgeKind::IdenticalTo), "{kinds:?}");

        driver.shutdown().await.unwrap();
    }

    /// The two rebuilds run from different inputs on different
    /// cadences, so each pass must leave the other's edges — and the
    /// asserted ones — standing. This is the regression the scope
    /// split exists for.
    #[tokio::test]
    async fn the_two_rebuilds_cannot_destroy_each_others_edges() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteEdgeRepository::new(isle.clone());
        let (a, b) = seed_two_assets(&isle).await;

        let mut visual = ConstellationEdge::new(b, a, EdgeKind::VisualSimilarity).unwrap();
        visual.weight = Some(0.83);
        visual.label = Some("test-model".into());
        repo.replace_visual_edges_of(&b, vec![visual])
            .await
            .unwrap();

        let mut windowed = ConstellationEdge::new(b, a, EdgeKind::TimeProximity).unwrap();
        windowed.weight = Some(0.7);
        repo.replace_synth_edges_of(&b, vec![windowed])
            .await
            .unwrap();

        // The windowed pass ran after the visual one: the visual edge
        // must still be there, and vice versa after an empty visual
        // pass replaces the visual set.
        let kinds: Vec<EdgeKind> = repo
            .edges_of(&b, None, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert!(kinds.contains(&EdgeKind::VisualSimilarity), "{kinds:?}");
        assert!(kinds.contains(&EdgeKind::TimeProximity), "{kinds:?}");

        // An empty visual rebuild (model produced no matches above the
        // floor) clears the visual set and nothing else.
        repo.replace_visual_edges_of(&b, vec![]).await.unwrap();
        let kinds: Vec<EdgeKind> = repo
            .edges_of(&b, None, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert!(!kinds.contains(&EdgeKind::VisualSimilarity), "{kinds:?}");
        assert!(kinds.contains(&EdgeKind::TimeProximity), "{kinds:?}");

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_visual_path_refuses_kinds_it_does_not_own() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteEdgeRepository::new(isle.clone());
        let (a, b) = seed_two_assets(&isle).await;

        let smuggled_windowed = ConstellationEdge::new(b, a, EdgeKind::TimeProximity).unwrap();
        let smuggled_assertion = ConstellationEdge::new(b, a, EdgeKind::DerivedFrom).unwrap();
        let mut visual = ConstellationEdge::new(b, a, EdgeKind::VisualSimilarity).unwrap();
        visual.weight = Some(0.9);
        repo.replace_visual_edges_of(&b, vec![smuggled_windowed, smuggled_assertion, visual])
            .await
            .unwrap();

        let kinds: Vec<EdgeKind> = repo
            .edges_of(&b, None, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, vec![EdgeKind::VisualSimilarity]);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn adding_no_edges_is_a_no_op() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteEdgeRepository::new(isle.clone());
        let (_a, b) = seed_two_assets(&isle).await;

        repo.add_edges(vec![]).await.unwrap();
        assert!(repo.edges_of(&b, None, 10).await.unwrap().is_empty());

        driver.shutdown().await.unwrap();
    }
}
