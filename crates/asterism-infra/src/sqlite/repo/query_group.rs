//! SQLite adapter for `QueryGroupRepository` — the persistence half of
//! the Query Group evaluation core.
//!
//! Three primitives, all against the existing schema (no migration — the
//! `bucket.kind` / `query_json` columns land in W2):
//!
//! - [`expand_group_closure`](SqliteQueryGroupRepository::expand_group_closure)
//!   — recursive-CTE nesting expansion over `bucket_link`, reusing the
//!   reachability shape the link cycle check uses (`group.rs`).
//! - [`fetch_sortable_assets`](SqliteQueryGroupRepository::fetch_sortable_assets)
//!   — the SQL filter evaluated with no `LIMIT`, projecting exactly the
//!   columns the sort evaluator reads. The `WHERE` clause is built by the
//!   shared [`QueryParts`] so the evaluate path and the read path can
//!   never drift.
//! - [`replace_membership`](SqliteQueryGroupRepository::replace_membership)
//!   — bulk `DELETE` + positioned bulk `INSERT` in one transaction.

use asterism_contract::query_group::QueryGroupQuery;
use asterism_core::domain::asset::AssetQuery;
use asterism_core::domain::query_group_eval::{DependencyGraph, dependency_graph, reaches};
use asterism_core::domain::repository::{QueryGroupRepository, QueryGroupRow};
use asterism_core::domain::sort_eval::SortableAsset;
use asterism_core::domain::value::{AssetId, GroupId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::params;
use rusqlite::types::Value;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, json_to_strings};
use crate::sqlite::repo::asset::QueryParts;

/// Loads the composite dependency graph (`bucket_link`
/// containment edges ∪ query-rule reference edges) for one persona,
/// **inside an open connection** — so a caller can run the cycle guard
/// and its write in the same isle call, making check-then-write
/// genuinely atomic on the serialized writer (the property the design
/// borrows from the existing bucket_link CTE guard).
///
/// Unparsable rules / non-UUID refs contribute no edges: an
/// unevaluable rule cannot drive a refresh loop, and rules are
/// validated at their own write time.
pub(crate) fn load_dependency_graph(
    conn: &rusqlite::Connection,
    persona: Uuid,
) -> Result<DependencyGraph, rusqlite::Error> {
    let mut containment: Vec<(GroupId, GroupId)> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT bl.parent_id, bl.child_id FROM bucket_link bl \
             JOIN bucket b ON b.id = bl.parent_id WHERE b.persona_id = ?1",
        )?;
        let mut rows = stmt.query(params![persona])?;
        while let Some(row) = rows.next()? {
            containment.push((
                GroupId::from_uuid(row.get(0)?),
                GroupId::from_uuid(row.get(1)?),
            ));
        }
    }
    let mut rules: Vec<(GroupId, Vec<GroupId>)> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, query_json FROM bucket \
             WHERE persona_id = ?1 AND kind = 'query' AND query_json IS NOT NULL",
        )?;
        let mut rows = stmt.query(params![persona])?;
        while let Some(row) = rows.next()? {
            let id: Uuid = row.get(0)?;
            let json: String = row.get(1)?;
            let refs: Vec<GroupId> = QueryGroupQuery::parse(&json)
                .map(|q| {
                    q.filter
                        .group_ids
                        .iter()
                        .filter_map(|s| s.parse().ok().map(GroupId::from_uuid))
                        .collect()
                })
                .unwrap_or_default();
            rules.push((GroupId::from_uuid(id), refs));
        }
    }
    Ok(dependency_graph(containment, rules))
}

/// SQLite adapter (writer isle).
#[derive(Clone)]
pub struct SqliteQueryGroupRepository {
    isle: AsyncIsle,
}

impl SqliteQueryGroupRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// Raw sortable row read inside the isle closure. `group_ids` are attached
/// in a second bulk pass (see `fetch_sortable_assets`).
struct SortRow {
    id: Uuid,
    persona_id: Uuid,
    // NULL (unclassified, asset-model v4) reads as "" — the evaluator
    // ranks unknown slugs last, which mirrors the UI comparator's
    // `findIndex < 0 → len` fallback, so the sentinel needs no branch.
    modality: Option<String>,
    occurred_at: i64,
    created_at: i64,
    // The `UpdatedAt` axis key. NOT NULL in the schema and seeded equal
    // to `created_at`, so there is no absent state to model.
    updated_at: i64,
    labels: String,
    cover: Option<String>,
    // NULL = unrated, which the `Rating` axis keeps as a distinct state
    // (tail in both directions) rather than folding into a number.
    rating: Option<i64>,
    // NULL = nothing plays: a still image by nature, a container the
    // importer could not probe by accident. The `Duration` axis keeps
    // that state distinct for the reason the rating axis does — a
    // stand-in 0 would lead a "shortest first" page.
    duration_ms: Option<i64>,
    // NULL = the row's bytes were never recorded (`FileSize` axis).
    // Same three-valued treatment one column over.
    file_size_bytes: Option<i64>,
    // NULL = nothing measured the row's dimensions (`Pixels` axis).
    // Multiplied out by SQLite, because the axis orders on the product:
    // the stored pair is coded (pre-orientation) and either side alone
    // says nothing about how large the picture is.
    pixel_count: Option<i64>,
}

impl SortRow {
    // The three metric columns sit at the end because index-based reads
    // below pair with position here: appending leaves every existing
    // index alone, where inserting in the middle would silently
    // re-point all of them.
    const COLUMNS: &'static str = "id, persona_id, modality, occurred_at, created_at, \
         updated_at, labels, cover, rating, duration_ms, file_size_bytes, \
         (width_px * height_px) AS pixel_count";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            modality: row.get(2)?,
            occurred_at: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
            labels: row.get(6)?,
            cover: row.get(7)?,
            rating: row.get(8)?,
            duration_ms: row.get(9)?,
            file_size_bytes: row.get(10)?,
            pixel_count: row.get(11)?,
        })
    }
}

#[async_trait]
impl QueryGroupRepository for SqliteQueryGroupRepository {
    async fn expand_group_closure(&self, raw: &[GroupId]) -> Result<Vec<GroupId>, DomainError> {
        if raw.is_empty() {
            return Ok(Vec::new());
        }
        let seeds: Vec<Vec<u8>> = raw
            .iter()
            .map(|g| g.as_uuid().as_bytes().to_vec())
            .collect();
        let uuids: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                // Anchor the recursion on the seed ids that exist as
                // buckets; the seeds themselves are unioned back in below
                // regardless. Dropping a vanished seed entirely would be
                // wrong in the all-seeds-deleted case: an empty closure
                // means *no group constraint* to the filter (falls open to
                // the whole persona corpus), while the read path's
                // `group_ids IN (...)` EXISTS matches nothing for the same
                // input. The recursive arm walks `bucket_link` child edges
                // exactly as the link cycle check's `reach` CTE does —
                // `parent_id -> child_id`, following every nesting level
                // (grandchildren and deeper), query-group children
                // included.
                let placeholders = vec!["?"; seeds.len()].join(", ");
                let sql = format!(
                    "WITH RECURSIVE reach(id) AS ( \
                         SELECT id FROM bucket WHERE id IN ({placeholders}) \
                         UNION \
                         SELECT bl.child_id FROM bucket_link bl \
                           JOIN reach ON bl.parent_id = reach.id \
                     ) \
                     SELECT id FROM reach"
                );
                let params: Vec<Value> = seeds.iter().map(|b| Value::Blob(b.clone())).collect();
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params), |row| {
                        row.get::<_, Uuid>(0)
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        // Union the raw seeds back in so a seed that no longer exists as a
        // bucket still constrains the filter (matching nothing) instead of
        // silently widening it.
        let mut seen: std::collections::HashSet<Uuid> = uuids.into_iter().collect();
        for g in raw {
            seen.insert(*g.as_uuid());
        }
        Ok(seen.into_iter().map(GroupId::from_uuid).collect())
    }

    async fn fetch_sortable_assets(
        &self,
        query: &AssetQuery,
    ) -> Result<Vec<SortableAsset>, DomainError> {
        // Same filter surface as the read path (`list` / `list_index`) — the
        // shared builder guarantees identical WHERE semantics. No LIMIT:
        // the evaluator consumes the whole set. The full-text predicate is
        // joined in by the service (Tantivy ∩ SQL), not here.
        let parts = QueryParts::build(query);
        let select_sql = format!("SELECT {} FROM asset {}", SortRow::COLUMNS, parts.where_sql,);
        // group_ids in one bulk pass: join `asset_bucket` against the same
        // predicate rather than probing per returned id (mirrors
        // `page_index`). The predicate columns are unambiguous — the
        // WHERE clause references no column that `asset_bucket` also has.
        //
        // Trashed Groups are filtered **outside** that inner query, not
        // by joining `bucket` into it: `bucket` carries `persona_id` and
        // `trashed_at` too, and the shared builder emits some predicates
        // unqualified, so pulling the table into the same scope makes
        // them ambiguous. The filter is needed because `asset_bucket`
        // rows outlive a trashed Group by design, so the link table alone
        // would report filings the sidebar no longer shows (same trap as
        // `fetch_group_ids_map`).
        // `position` rides along for `Group` + `Ordered`; the outer
        // ordering pins which filing counts as primary, same contract as
        // `asset::fetch_group_ids_map`.
        let group_sql = format!(
            "SELECT asset_id, bucket_id, position FROM ( \
                 SELECT asset_bucket.asset_id AS asset_id, \
                        asset_bucket.bucket_id AS bucket_id, \
                        asset_bucket.position AS position \
                 FROM asset_bucket JOIN asset ON asset.id = asset_bucket.asset_id {} \
             ) WHERE bucket_id IN (SELECT id FROM bucket WHERE trashed_at IS NULL) \
             ORDER BY asset_id, bucket_id",
            parts.where_sql,
        );
        let params = parts.params;

        let (rows, group_map) = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&select_sql)?;
                let rows = stmt
                    .query_map(
                        rusqlite::params_from_iter(params.iter().cloned()),
                        SortRow::from_row,
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut group_map: std::collections::HashMap<Uuid, Vec<(Uuid, i64)>> =
                    std::collections::HashMap::with_capacity(rows.len());
                {
                    let mut gstmt = conn.prepare(&group_sql)?;
                    let mut grows =
                        gstmt.query(rusqlite::params_from_iter(params.iter().cloned()))?;
                    while let Some(row) = grows.next()? {
                        let asset_id: Uuid = row.get(0)?;
                        let bucket_id: Uuid = row.get(1)?;
                        let position: i64 = row.get(2)?;
                        group_map
                            .entry(asset_id)
                            .or_default()
                            .push((bucket_id, position));
                    }
                }
                Ok((rows, group_map))
            })
            .await
            .map_err(infra_err)?;

        // Same primary-group rule as the card / index reads: when the
        // filter names exactly one Group, that Group owns `group_ids[0]`
        // and the `position` the `Group` + `Ordered` axis arranges on.
        // Sharing the helper is the point — this path feeds both the
        // sorted listing and the `position` a Query Group freezes, so a
        // second answer here would put the two out of step with the grid.
        let sole_group: Option<Uuid> = if query.group_ids.len() == 1 {
            Some(*query.group_ids[0].as_uuid())
        } else {
            None
        };
        rows.into_iter()
            .map(|r| {
                let entries = group_map
                    .get(&r.id)
                    .map(|es| crate::sqlite::repo::asset::primary_group_first(es, sole_group));
                let group_ids = entries
                    .as_ref()
                    .map(|es| {
                        es.iter()
                            .map(|(u, _)| GroupId::from_uuid(*u).to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                let primary_group_position = entries
                    .as_ref()
                    .and_then(|es| es.first())
                    .map(|(_, position)| *position);
                Ok(SortableAsset {
                    id: AssetId::from_uuid(r.id).to_string(),
                    persona_id: asterism_core::domain::value::PersonaId::from_uuid(r.persona_id)
                        .to_string(),
                    modality: r.modality.unwrap_or_default(),
                    occurred_at_ms: r.occurred_at,
                    created_at_ms: r.created_at,
                    updated_at_ms: r.updated_at,
                    labels: json_to_strings(&r.labels)?,
                    group_ids,
                    primary_group_position,
                    cover: r.cover,
                    // The column carries no CHECK constraint, so a value
                    // written past the domain writer (hand SQL, bulk
                    // import) could sit outside 0..=5. `as u8` would wrap
                    // it into a legal-looking star; degrade to unrated
                    // (sorts last) instead so corruption stays visible as
                    // "no rating", not as a wrong one.
                    rating: r
                        .rating
                        .and_then(|v| u8::try_from(v).ok().filter(|v| *v <= 5)),
                    // Handed over as stored, signed and unclamped —
                    // unlike the rating beside them, these three axes
                    // have no legal range to fall outside of, so there is
                    // no corrupt value to degrade into an absent one. The
                    // evaluator only ever compares them against each
                    // other.
                    duration_ms: r.duration_ms,
                    file_size_bytes: r.file_size_bytes,
                    pixel_count: r.pixel_count,
                })
            })
            .collect()
    }

    async fn replace_membership(
        &self,
        bucket_id: &GroupId,
        ordered: &[AssetId],
        now: DateTime<Utc>,
    ) -> Result<u64, DomainError> {
        let bucket = *bucket_id.as_uuid();
        let now_ms = datetime_to_ms(&now);
        let ids: Vec<Uuid> = ordered.iter().map(|a| *a.as_uuid()).collect();
        let written = self
            .isle
            .call(move |conn| {
                // One transaction: a mid-way failure must not leave the
                // bucket half-emptied. DELETE then positioned bulk INSERT
                // via a single prepared statement reused across the loop
                // (the 100k-member bulk pattern).
                let tx = conn.transaction()?;
                tx.execute(
                    "DELETE FROM asset_bucket WHERE bucket_id = ?1",
                    rusqlite::params![bucket],
                )?;
                {
                    let mut stmt = tx.prepare(
                        "INSERT INTO asset_bucket \
                            (asset_id, bucket_id, added_at, position) \
                         VALUES (?1, ?2, ?3, ?4)",
                    )?;
                    for (position, asset) in ids.iter().enumerate() {
                        stmt.execute(rusqlite::params![asset, bucket, now_ms, position as i64])?;
                    }
                }
                tx.commit()?;
                Ok(ids.len() as u64)
            })
            .await
            .map_err(infra_err)?;
        Ok(written)
    }

    async fn create_query_group(
        &self,
        persona_id: asterism_core::domain::value::PersonaId,
        name: String,
        query_json: String,
        now: DateTime<Utc>,
    ) -> Result<asterism_core::domain::group::Group, DomainError> {
        use asterism_core::domain::group::{Group, GroupKind};
        // Domain constructor performs the name-non-empty check; the
        // query fields are stamped on top (Group::new builds manual).
        let mut group = Group::new(persona_id, name, None, now)?;
        group.kind = GroupKind::Query;
        group.query_json = Some(query_json.clone());
        let uuid = *group.id.as_uuid();
        let persona_uuid = *group.persona_id.as_uuid();
        let name_owned = group.name.clone();
        let now_ms = crate::sqlite::map::datetime_to_ms(&now);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO bucket \
                        (id, persona_id, name, created_at, updated_at, kind, query_json) \
                     VALUES (?1, ?2, ?3, ?4, ?4, 'query', ?5)",
                    params![uuid, persona_uuid, name_owned, now_ms, query_json],
                )?;
                Ok(())
            })
            .await
            .map_err(|err| {
                let msg = err.to_string();
                if msg.contains("UNIQUE") || msg.contains("unique") {
                    DomainError::clashes(format!(
                        "a group named {:?} already exists for this persona",
                        group.name
                    ))
                } else {
                    infra_err(err)
                }
            })?;
        Ok(group)
    }

    async fn set_query_json(
        &self,
        bucket_id: &GroupId,
        query_json: &str,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let target = *bucket_id;
        let uuid = *bucket_id.as_uuid();
        let json = query_json.to_string();
        let now_ms = crate::sqlite::map::datetime_to_ms(&now);
        // Candidate reference edges — parsed up front so a malformed
        // blob errs before any isle round-trip.
        let new_refs: Vec<GroupId> = QueryGroupQuery::parse(&json)
            .map_err(|e| DomainError::Validation(format!("query_json invalid: {e}")))?
            .filter
            .group_ids
            .iter()
            .filter_map(|s| s.parse().ok().map(GroupId::from_uuid))
            .collect();
        // 0 = ok, 1 = not found / not query, 2 = cycle. The composite
        // cycle guard runs in the SAME isle call as the write, so
        // check-then-write is atomic on the serialized writer (review
        // F1: two separate calls left a TOCTOU window against a
        // concurrent link()).
        let verdict: u8 = self
            .isle
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                let persona: Option<Uuid> = conn
                    .query_row(
                        "SELECT persona_id FROM bucket WHERE id = ?1 AND kind = 'query'",
                        params![uuid],
                        |r| r.get(0),
                    )
                    .optional()?;
                let Some(persona) = persona else {
                    return Ok(1);
                };
                let mut graph = load_dependency_graph(conn, persona)?;
                // Wholesale replacement of the target's adjacency is
                // correct because a query group never holds containment
                // out-edges (link() rejects query parents) — if that
                // invariant ever changes, merge instead of insert
                // (review F3).
                graph.insert(target, new_refs.clone());
                for r in &new_refs {
                    if reaches(&graph, r, &target) {
                        return Ok(2);
                    }
                }
                conn.execute(
                    "UPDATE bucket SET query_json = ?1, updated_at = ?2 \
                     WHERE id = ?3 AND kind = 'query'",
                    params![json, now_ms, uuid],
                )?;
                Ok(0)
            })
            .await
            .map_err(infra_err)?;
        match verdict {
            1 => Err(DomainError::not_found("query group", bucket_id)),
            2 => Err(DomainError::Validation(
                "query reference cycle: the new rule reaches back to this group \
                 (a cyclic rule would make refresh a mutual-trigger loop)"
                    .into(),
            )),
            _ => Ok(()),
        }
    }

    async fn mark_refresh_result(
        &self,
        bucket_id: &GroupId,
        status: &str,
        error: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let uuid = *bucket_id.as_uuid();
        let status = status.to_string();
        let error = error.map(|e| e.to_string());
        let at_ms = crate::sqlite::map::datetime_to_ms(&at);
        self.isle
            .call(move |conn| {
                // Unknown id (deleted mid-refresh) → 0 rows, silently
                // fine. `updated_at` stays untouched — the stamp is
                // telemetry, not a user edit (trait contract).
                conn.execute(
                    "UPDATE bucket SET last_refresh_at = ?2, \
                        last_refresh_status = ?3, last_refresh_error = ?4 \
                     WHERE id = ?1",
                    params![uuid, at_ms, status, error],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    async fn member_ids(&self, bucket_id: &GroupId) -> Result<Vec<AssetId>, DomainError> {
        let bucket = *bucket_id.as_uuid();
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT asset_id FROM asset_bucket \
                     WHERE bucket_id = ?1 ORDER BY position, asset_id",
                )?;
                let rows = stmt
                    .query_map(params![bucket], |row| row.get::<_, Uuid>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }

    async fn list_query_groups(&self) -> Result<Vec<QueryGroupRow>, DomainError> {
        let rows: Vec<(Uuid, Uuid, String)> = self
            .isle
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, persona_id, query_json FROM bucket \
                     WHERE kind = 'query' AND query_json IS NOT NULL \
                     ORDER BY persona_id, name",
                )?;
                let rows = stmt
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, persona, query_json)| QueryGroupRow {
                bucket_id: GroupId::from_uuid(id),
                persona_id: asterism_core::domain::value::PersonaId::from_uuid(persona),
                query_json,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_core::domain::attribution::AttributionContext;
    use asterism_core::domain::value::PersonaId;
    use rusqlite::params;

    /// The unrecorded context, spelled the way a crate outside
    /// `asterism-core` has to spell it (`unrecorded()` is crate-private
    /// there): an empty assertion is defined to be the same value. These
    /// fixtures are about the freeze, not about who asked for it.
    fn nobody() -> AttributionContext {
        AttributionContext::asserted(None, None).unwrap()
    }

    /// Seed one persona and return its id.
    async fn seed_persona(isle: &AsyncIsle) -> PersonaId {
        let pid = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p', 'P', 0, 0)",
                params![pid],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        PersonaId::from_uuid(pid)
    }

    /// Insert one asset with the given occurred/label; returns its id.
    async fn seed_asset(
        isle: &AsyncIsle,
        persona: &PersonaId,
        occurred: i64,
        labels_json: &str,
    ) -> AssetId {
        let aid = Uuid::now_v7();
        let pid = *persona.as_uuid();
        let labels = labels_json.to_string();
        // One locator for every seeded row. It used to be derived from
        // the id because the schema made the Source pair unique; V61
        // demoted that to a lookup, so these rows can say what they are
        // — several assets standing at one address, which is what
        // `N : 1` means and what these fixtures never cared about.
        let locator = crate::sqlite::stored_locator("seed.md");
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, labels, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', ?5, 'dialogue', ?3, ?4, ?4, ?4)",
                params![aid, pid, labels, occurred, locator],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        AssetId::from_uuid(aid)
    }

    /// [`seed_asset`] carrying the two metric columns. Either may be
    /// absent, and the two are set from one call so a fixture can put
    /// them in disagreement: a still image has no length but may well
    /// have a recorded size, and the reverse is what an unprobed
    /// container looks like.
    async fn seed_measured_asset(
        isle: &AsyncIsle,
        persona: &PersonaId,
        occurred: i64,
        duration_ms: Option<i64>,
        file_size_bytes: Option<i64>,
    ) -> AssetId {
        let asset = seed_asset(isle, persona, occurred, "[]").await;
        let aid = *asset.as_uuid();
        isle.call(move |conn| {
            let written = conn.execute(
                "UPDATE asset SET duration_ms = ?2, file_size_bytes = ?3 WHERE id = ?1",
                params![aid, duration_ms, file_size_bytes],
            )?;
            assert_eq!(written, 1, "the fixture must actually carry its metrics");
            Ok(())
        })
        .await
        .unwrap();
        asset
    }

    /// Create a bucket (group) with the given name.
    async fn seed_bucket(isle: &AsyncIsle, persona: &PersonaId, name: &str) -> GroupId {
        let gid = Uuid::now_v7();
        let pid = *persona.as_uuid();
        let name = name.to_string();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO bucket (id, persona_id, name, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 0, 0)",
                params![gid, pid, name],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        GroupId::from_uuid(gid)
    }

    async fn link_buckets(isle: &AsyncIsle, parent: &GroupId, child: &GroupId) {
        let p = *parent.as_uuid();
        let c = *child.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO bucket_link (parent_id, child_id, added_at, position)
                 VALUES (?1, ?2, 0, 0)",
                params![p, c],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    async fn add_to_bucket(isle: &AsyncIsle, asset: &AssetId, bucket: &GroupId) {
        let a = *asset.as_uuid();
        let b = *bucket.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset_bucket (asset_id, bucket_id, added_at, position)
                 VALUES (?1, ?2, 0, 0)",
                params![a, b],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    async fn membership(isle: &AsyncIsle, bucket: &GroupId) -> Vec<(Uuid, i64)> {
        let b = *bucket.as_uuid();
        isle.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT asset_id, position FROM asset_bucket \
                 WHERE bucket_id = ?1 ORDER BY position ASC",
            )?;
            let rows = stmt
                .query_map(params![b], |row| {
                    Ok((row.get::<_, Uuid>(0)?, row.get::<_, i64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn expand_closure_walks_grandchildren() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteQueryGroupRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        // root -> child -> grandchild (two nesting levels).
        let root = seed_bucket(&isle, &persona, "root").await;
        let child = seed_bucket(&isle, &persona, "child").await;
        let grandchild = seed_bucket(&isle, &persona, "grandchild").await;
        let unrelated = seed_bucket(&isle, &persona, "unrelated").await;
        link_buckets(&isle, &root, &child).await;
        link_buckets(&isle, &child, &grandchild).await;

        let mut out = repo.expand_group_closure(&[root]).await.unwrap();
        out.sort_by_key(|g| g.to_string());
        let mut expected = vec![root, child, grandchild];
        expected.sort_by_key(|g| g.to_string());
        assert_eq!(out, expected, "closure must include the grandchild");
        assert!(
            !out.contains(&unrelated),
            "unrelated bucket must not appear in the closure"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn expand_closure_keeps_vanished_seeds() {
        // A query group may reference a group that was later deleted. The
        // closure must keep the vanished seed so the filter still matches
        // nothing, instead of dropping to "no group constraint" (= the
        // whole persona corpus). Read-path parity: `EXISTS ... IN (gone)`
        // also matches nothing.
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteQueryGroupRepository::new(isle.clone());
        let gone = GroupId::from_uuid(Uuid::now_v7());
        let out = repo.expand_group_closure(&[gone]).await.unwrap();
        assert_eq!(out, vec![gone]);
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn expand_closure_empty_input_is_empty() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteQueryGroupRepository::new(isle.clone());
        assert!(repo.expand_group_closure(&[]).await.unwrap().is_empty());
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn fetch_sortable_scopes_by_group_and_attaches_group_ids() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteQueryGroupRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let g = seed_bucket(&isle, &persona, "g").await;
        let in_group = seed_asset(&isle, &persona, 100, "[\"x\"]").await;
        let out_group = seed_asset(&isle, &persona, 200, "[]").await;
        add_to_bucket(&isle, &in_group, &g).await;

        let query = AssetQuery {
            persona_id: Some(persona),
            group_ids: vec![g],
            limit: 0, // ignored — full eval
            ..Default::default()
        };
        let assets = repo.fetch_sortable_assets(&query).await.unwrap();
        assert_eq!(assets.len(), 1, "only the in-group asset passes the filter");
        let a = &assets[0];
        assert_eq!(a.id, in_group.to_string());
        assert_eq!(a.persona_id, persona.to_string());
        assert_eq!(a.occurred_at_ms, 100);
        assert_eq!(a.labels, vec!["x".to_string()]);
        assert_eq!(a.group_ids, vec![g.to_string()], "group_ids attached");
        // out_group is absent (sanity that the filter, not a full scan, ran).
        assert!(!assets.iter().any(|s| s.id == out_group.to_string()));

        driver.shutdown().await.unwrap();
    }

    /// A Query Group is a saved filter that materialises into a real
    /// bucket, so what it evaluates over has to be what the grid shows
    /// — that is the whole reason the evaluator borrows the read
    /// path's `WHERE` builder instead of writing its own. A headstone
    /// (V49) is therefore not a member: without this, resolving a
    /// duplicate would leave the refresh job re-filing the folded row
    /// into every Group whose rule it matched, and the fold would look
    /// undone the next time the rule ran.
    ///
    /// The row is asserted present before the fold, so the assertion
    /// cannot pass over a set the filter excluded for some other
    /// reason.
    #[tokio::test]
    async fn a_folded_row_is_not_a_query_group_member() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteQueryGroupRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = seed_asset(&isle, &persona, 100, "[\"x\"]").await;
        let headstone = seed_asset(&isle, &persona, 200, "[\"x\"]").await;

        let query = AssetQuery {
            persona_id: Some(persona),
            limit: 0, // ignored — full eval
            ..Default::default()
        };
        let before = repo.fetch_sortable_assets(&query).await.unwrap();
        assert_eq!(before.len(), 2, "both match the rule before the fold");

        let (h, k) = (*headstone.as_uuid(), *keeper.as_uuid());
        isle.call(move |conn| {
            let marked = conn.execute(
                "UPDATE asset SET folded_into = ?2 WHERE id = ?1",
                params![h, k],
            )?;
            assert_eq!(marked, 1, "the fixture must actually stand a headstone");
            Ok(())
        })
        .await
        .unwrap();

        let after = repo.fetch_sortable_assets(&query).await.unwrap();
        assert_eq!(
            after.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
            vec![keeper.to_string()],
            "the evaluator sees the same population as the grid"
        );

        driver.shutdown().await.unwrap();
    }

    /// The evaluator can only rank what the adapter hands it, so the two
    /// metric columns have to arrive on the same row as everything else
    /// — and arrive absent when the row has nothing to say.
    ///
    /// The measured row's two values are deliberately unequal and not
    /// interchangeable (two minutes against half a megabyte): the
    /// projected column list and the positional reads that consume it
    /// are two lists which have to stay in step, and a row whose length
    /// and size happened to be the same number would let them swap
    /// places unnoticed.
    #[tokio::test]
    async fn fetch_sortable_carries_duration_and_size() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteQueryGroupRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let clip = seed_measured_asset(&isle, &persona, 100, Some(120_000), Some(500_000)).await;
        let still = seed_measured_asset(&isle, &persona, 200, None, None).await;

        let all = AssetQuery {
            persona_id: Some(persona),
            limit: 0, // ignored — full eval
            ..Default::default()
        };
        let assets = repo.fetch_sortable_assets(&all).await.unwrap();
        let row = |id: &AssetId| {
            assets
                .iter()
                .find(|a| a.id == id.to_string())
                .cloned()
                .expect("the seeded asset must reach the evaluator")
        };
        assert_eq!(row(&clip).duration_ms, Some(120_000));
        assert_eq!(row(&clip).file_size_bytes, Some(500_000));
        assert_eq!(
            row(&still).duration_ms,
            None,
            "no length must reach the evaluator as absent, not as a zero"
        );
        assert_eq!(row(&still).file_size_bytes, None);

        // The band travels with the filter: this path shares the read
        // path's `WHERE` builder, so a Query Group rule naming a length
        // drops the row that has none — an upper bound alone, which is
        // the case three-valued logic makes least obvious.
        let banded = repo
            .fetch_sortable_assets(&AssetQuery {
                duration_max_ms: Some(120_000),
                ..all
            })
            .await
            .unwrap();
        assert_eq!(
            banded.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
            vec![clip.to_string()],
            "the evaluator excludes what the listing excludes"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn replace_membership_burns_positions_and_replaces() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteQueryGroupRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let g = seed_bucket(&isle, &persona, "g").await;
        let a = seed_asset(&isle, &persona, 1, "[]").await;
        let b = seed_asset(&isle, &persona, 2, "[]").await;
        let c = seed_asset(&isle, &persona, 3, "[]").await;

        // Pre-existing stale membership that must be wiped.
        add_to_bucket(&isle, &a, &g).await;

        let now = Utc::now();
        let written = repo.replace_membership(&g, &[c, a, b], now).await.unwrap();
        assert_eq!(written, 3);

        let rows = membership(&isle, &g).await;
        // position 0,1,2 in the exact order supplied.
        assert_eq!(
            rows,
            vec![(*c.as_uuid(), 0), (*a.as_uuid(), 1), (*b.as_uuid(), 2),]
        );

        // A second replace fully overwrites (no leftover from the first).
        let written2 = repo.replace_membership(&g, &[b], now).await.unwrap();
        assert_eq!(written2, 1);
        let rows2 = membership(&isle, &g).await;
        assert_eq!(rows2, vec![(*b.as_uuid(), 0)]);

        driver.shutdown().await.unwrap();
    }

    // --- end-to-end pipeline (QueryGroupService over concrete repos) -----

    use asterism_core::application::query_group_service::QueryGroupService;
    use std::sync::Arc;

    fn build_service(isle: &AsyncIsle) -> QueryGroupService {
        use crate::sqlite::repo::asset::SqliteAssetRepository;
        use crate::sqlite::repo::group::SqliteGroupRepository;
        use crate::sqlite::repo::persona::SqlitePersonaRepository;
        QueryGroupService::new(
            Arc::new(SqliteQueryGroupRepository::new(isle.clone())),
            Arc::new(SqlitePersonaRepository::new(isle.clone())),
            Arc::new(SqliteAssetRepository::new(isle.clone())),
            Arc::new(SqliteGroupRepository::new(isle.clone())),
        )
    }

    /// Gives an asset a body through the production write path — the
    /// body cache plus the Query-side text index — so a `search_text`
    /// rule is evaluated against the same rows the app would have
    /// written.
    async fn seed_body(isle: &AsyncIsle, asset_id: &AssetId, text: &str) {
        use asterism_core::domain::repository::{AssetBodyRepository, AssetIndexer, IndexDoc};
        crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone())
            .upsert(asset_id, text)
            .await
            .unwrap();
        crate::sqlite::repo::SqliteAssetTextIndex::new(isle.clone())
            .upsert(&IndexDoc {
                asset_id: *asset_id,
                persona_id: PersonaId::new(),
                text: Some(text.to_string()),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_query_group_materializes_and_conflicts_on_name() {
        use asterism_contract::command::CreateQueryGroupCommand;
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let g1 = seed_bucket(&isle, &persona, "manual-src").await;
        let a1 = seed_asset(&isle, &persona, 100, "[]").await;
        add_to_bucket(&isle, &a1, &g1).await;

        let svc = build_service(&isle);
        let json = format!(
            r#"{{"v":1,"filter":{{"persona_id":"{persona}","group_ids":["{g1}"]}},"sort":{{"target":"occurred_at","order":"updated","reverse":false}}}}"#
        );
        let dto = svc
            .create_query_group(
                CreateQueryGroupCommand {
                    persona_id: persona.to_string(),
                    name: "QG".into(),
                    query_json: json.clone(),
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert_eq!(dto.kind, "query");
        assert_eq!(dto.query_json.as_deref(), Some(json.as_str()));
        // First evaluation ran synchronously — the member is frozen.
        let qg_id: Uuid = dto.id.parse().unwrap();
        let rows = membership(&isle, &GroupId::from_uuid(qg_id)).await;
        assert_eq!(rows, vec![(*a1.as_uuid(), 0)]);

        // Same name again → Conflict (bucket UNIQUE surfaces untouched).
        let dup = svc
            .create_query_group(
                CreateQueryGroupCommand {
                    persona_id: persona.to_string(),
                    name: "QG".into(),
                    query_json: json,
                },
                &nobody(),
            )
            .await;
        assert!(matches!(
            dup,
            Err(asterism_core::error::DomainError::Conflict {
                kind: asterism_core::error::ConflictKind::Clashes,
                ..
            })
        ));
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn update_query_rejects_dependency_cycles() {
        use asterism_contract::command::{CreateQueryGroupCommand, UpdateQueryGroupQueryCommand};
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let g1 = seed_bucket(&isle, &persona, "src").await;
        let g2 = seed_bucket(&isle, &persona, "outer").await;

        let svc = build_service(&isle);
        let json_g1 = format!(
            r#"{{"v":1,"filter":{{"persona_id":"{persona}","group_ids":["{g1}"]}},"sort":{{"target":"occurred_at","order":"updated","reverse":false}}}}"#
        );
        let qg = svc
            .create_query_group(
                CreateQueryGroupCommand {
                    persona_id: persona.to_string(),
                    name: "QG".into(),
                    query_json: json_g1.clone(),
                },
                &nobody(),
            )
            .await
            .unwrap();
        let qg_id = GroupId::from_uuid(qg.id.parse().unwrap());

        // Nest the query group under g2, then point its rule at g2:
        // g2 --contains--> qg and qg --refs--> g2 would loop refresh.
        link_buckets(&isle, &g2, &qg_id).await;
        let json_cycle = format!(
            r#"{{"v":1,"filter":{{"persona_id":"{persona}","group_ids":["{g2}"]}},"sort":{{"target":"occurred_at","order":"updated","reverse":false}}}}"#
        );
        let err = svc
            .update_query(
                UpdateQueryGroupQueryCommand {
                    group_id: qg.id.clone(),
                    query_json: json_cycle,
                },
                &nobody(),
            )
            .await;
        assert!(err.is_err(), "indirect cycle (containment + ref) rejected");

        // Self-reference is the degenerate cycle.
        let json_self = format!(
            r#"{{"v":1,"filter":{{"persona_id":"{persona}","group_ids":["{qg_id}"]}},"sort":{{"target":"occurred_at","order":"updated","reverse":false}}}}"#
        );
        let err = svc
            .update_query(
                UpdateQueryGroupQueryCommand {
                    group_id: qg.id.clone(),
                    query_json: json_self,
                },
                &nobody(),
            )
            .await;
        assert!(err.is_err(), "self-reference rejected");

        // The stored rule is untouched by the rejected writes, and a
        // benign update still succeeds.
        let ok = svc
            .update_query(
                UpdateQueryGroupQueryCommand {
                    group_id: qg.id.clone(),
                    query_json: json_g1.clone(),
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert_eq!(ok.query_json.as_deref(), Some(json_g1.as_str()));
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn link_rejects_composite_cycle_through_query_reference() {
        use asterism_core::domain::repository::GroupRepository;
        // qg --query-ref--> P; linking P --contains--> qg would close
        // P → qg → P through the composite graph. The bucket_link-only
        // CTE cannot see it; the composite guard inside link() must.
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let p = seed_bucket(&isle, &persona, "P").await;
        let repo = SqliteQueryGroupRepository::new(isle.clone());
        let json = format!(
            r#"{{"v":1,"filter":{{"persona_id":"{persona}","group_ids":["{p}"]}},"sort":{{"target":"occurred_at","order":"updated","reverse":false}}}}"#
        );
        let qg = repo
            .create_query_group(persona, "qg".into(), json, chrono::Utc::now())
            .await
            .unwrap();

        let groups = crate::sqlite::repo::group::SqliteGroupRepository::new(isle.clone());
        let err = groups.link(&p, &qg.id, chrono::Utc::now()).await;
        assert!(err.is_err(), "composite cycle must be rejected");

        // A reference-free sibling still links fine (guard is not
        // over-broad).
        let free = seed_bucket(&isle, &persona, "free").await;
        groups.link(&p, &free, chrono::Utc::now()).await.unwrap();
        driver.shutdown().await.unwrap();
    }

    // --- W3c: live-source dispatch + promote provenance ------------------

    /// Queue stub — dispatch tests only assert persistence, not the
    /// apalis run (enqueue is fire-and-forget in the service).
    struct NoopQueue;

    #[async_trait]
    impl asterism_core::domain::repository::JobQueue for NoopQueue {
        async fn enqueue(
            &self,
            _kind: asterism_core::domain::job::JobKind,
            _payload: serde_json::Value,
        ) -> Result<String, DomainError> {
            Ok("noop".into())
        }
    }

    fn build_dispatch_service(isle: &AsyncIsle) -> asterism_core::application::DispatchService {
        use crate::sqlite::repo::SqliteDispatchRepository;
        use crate::sqlite::repo::asset::SqliteAssetRepository;
        use crate::sqlite::repo::group::SqliteGroupRepository;
        use crate::sqlite::repo::persona::SqlitePersonaRepository;
        use crate::sqlite::repo::snapshot::SqliteSnapshotRepository;
        let query_groups = Arc::new(SqliteQueryGroupRepository::new(isle.clone()));
        let qgs = Arc::new(QueryGroupService::new(
            query_groups.clone(),
            Arc::new(SqlitePersonaRepository::new(isle.clone())),
            Arc::new(SqliteAssetRepository::new(isle.clone())),
            Arc::new(SqliteGroupRepository::new(isle.clone())),
        ));
        asterism_core::application::DispatchService::new(
            Arc::new(SqliteSnapshotRepository::new(isle.clone())),
            Arc::new(SqliteDispatchRepository::new(isle.clone())),
            Arc::new(NoopQueue),
            Arc::new(SqliteGroupRepository::new(isle.clone())),
            query_groups,
            qgs,
        )
    }

    #[tokio::test]
    async fn dispatch_run_freezes_group_with_provenance() {
        use asterism_contract::command::DispatchRunCommand;
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let g = seed_bucket(&isle, &persona, "g").await;
        let a1 = seed_asset(&isle, &persona, 100, "[]").await;
        let a2 = seed_asset(&isle, &persona, 200, "[]").await;
        add_to_bucket(&isle, &a1, &g).await;
        add_to_bucket(&isle, &a2, &g).await;

        let svc = build_dispatch_service(&isle);
        let dto = svc
            .run(
                DispatchRunCommand {
                    persona_id: persona.to_string(),
                    group_id: Some(g.to_string()),
                    asset_ids: Vec::new(),
                    exporter_slug: "file".into(),
                    action: "export".into(),
                    params_json: String::new(),
                    operator_ai: None,
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert_eq!(dto.source_group_id.as_deref(), Some(g.to_string().as_str()));
        assert_eq!(dto.source_query_json, None);
        // The freeze exists and carries the two members.
        let member_count: i64 = isle
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM snapshot_asset", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(member_count, 2);

        // Re-dispatching the same group content reuses the snapshot row
        // (content-hash dedupe, P2).
        let dto2 = svc
            .run(
                DispatchRunCommand {
                    persona_id: persona.to_string(),
                    group_id: Some(g.to_string()),
                    asset_ids: Vec::new(),
                    exporter_slug: "file".into(),
                    action: "export".into(),
                    params_json: String::new(),
                    operator_ai: None,
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert_eq!(dto.snapshot_id, dto2.snapshot_id, "snapshot row shared");
        let snap_count: i64 = isle
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM snapshot", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(snap_count, 1);
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dispatch_run_refreshes_query_group_before_freezing() {
        use asterism_contract::command::{CreateQueryGroupCommand, DispatchRunCommand};
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let src = seed_bucket(&isle, &persona, "src").await;
        let a1 = seed_asset(&isle, &persona, 100, "[]").await;
        add_to_bucket(&isle, &a1, &src).await;

        let qgs = build_service(&isle);
        let json = format!(
            r#"{{"v":1,"filter":{{"persona_id":"{persona}","group_ids":["{src}"]}},"sort":{{"target":"occurred_at","order":"updated","reverse":false}}}}"#
        );
        let qg = qgs
            .create_query_group(
                CreateQueryGroupCommand {
                    persona_id: persona.to_string(),
                    name: "QG".into(),
                    query_json: json,
                },
                &nobody(),
            )
            .await
            .unwrap();

        // Membership changes AFTER the query group was materialised —
        // the dispatch-time refresh must pick it up.
        let a2 = seed_asset(&isle, &persona, 200, "[]").await;
        add_to_bucket(&isle, &a2, &src).await;

        let svc = build_dispatch_service(&isle);
        let dto = svc
            .run(
                DispatchRunCommand {
                    persona_id: persona.to_string(),
                    group_id: Some(qg.id.clone()),
                    asset_ids: Vec::new(),
                    exporter_slug: "file".into(),
                    action: "export".into(),
                    params_json: String::new(),
                    operator_ai: None,
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert!(dto.source_query_json.is_some(), "rule frozen as provenance");
        // The frozen snapshot carries BOTH members — the pre-dispatch
        // refresh made the freeze fresh.
        let sid: Uuid = dto.snapshot_id.parse().unwrap();
        let frozen: i64 = isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM snapshot_asset WHERE snapshot_id = ?1",
                    params![sid],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(frozen, 2, "dispatch froze the refreshed evaluation");
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dispatch_run_volatile_selection_and_redispatch() {
        use asterism_contract::command::{DispatchRunCommand, RedispatchCommand};
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let a1 = seed_asset(&isle, &persona, 100, "[]").await;
        let a2 = seed_asset(&isle, &persona, 200, "[]").await;

        let svc = build_dispatch_service(&isle);
        let dto = svc
            .run(
                DispatchRunCommand {
                    persona_id: persona.to_string(),
                    group_id: None,
                    asset_ids: vec![a2.to_string(), a1.to_string()],
                    exporter_slug: "file".into(),
                    action: "export".into(),
                    params_json: String::new(),
                    operator_ai: None,
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert_eq!(dto.source_group_id, None, "volatile dispatch has no group");

        // Redispatch shares the frozen input, writes only a new job row.
        let re = svc
            .redispatch(
                RedispatchCommand {
                    dispatch_id: dto.id.clone(),
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert_eq!(re.snapshot_id, dto.snapshot_id);
        assert_ne!(re.id, dto.id);
        let jobs: i64 = isle
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM dispatch_job", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(jobs, 2);
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dispatch_run_rejects_ambiguous_and_empty_sources() {
        use asterism_contract::command::DispatchRunCommand;
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let g = seed_bucket(&isle, &persona, "g").await;
        let a = seed_asset(&isle, &persona, 100, "[]").await;

        let svc = build_dispatch_service(&isle);
        // Both group_id and asset_ids → reject.
        let both = svc
            .run(
                DispatchRunCommand {
                    persona_id: persona.to_string(),
                    group_id: Some(g.to_string()),
                    asset_ids: vec![a.to_string()],
                    exporter_slug: "file".into(),
                    action: "export".into(),
                    params_json: String::new(),
                    operator_ai: None,
                },
                &nobody(),
            )
            .await;
        assert!(both.is_err(), "group_id XOR asset_ids");
        // Neither → reject.
        let neither = svc
            .run(
                DispatchRunCommand {
                    persona_id: persona.to_string(),
                    group_id: None,
                    asset_ids: Vec::new(),
                    exporter_slug: "file".into(),
                    action: "export".into(),
                    params_json: String::new(),
                    operator_ai: None,
                },
                &nobody(),
            )
            .await;
        assert!(neither.is_err(), "a source is required");
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn promote_stamps_origin_snapshot_and_bulk_attaches() {
        use crate::sqlite::repo::asset::SqliteAssetRepository;
        use crate::sqlite::repo::group::SqliteGroupRepository;
        use crate::sqlite::repo::persona::SqlitePersonaRepository;
        use crate::sqlite::repo::snapshot::SqliteSnapshotRepository;
        use asterism_contract::command::{CreateSnapshotCommand, PromoteSnapshotToGroupCommand};
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let a1 = seed_asset(&isle, &persona, 100, "[]").await;
        let a2 = seed_asset(&isle, &persona, 200, "[]").await;

        let svc = asterism_core::application::SnapshotService::new(
            Arc::new(SqliteSnapshotRepository::new(isle.clone())),
            Arc::new(SqlitePersonaRepository::new(isle.clone())),
            Arc::new(SqliteAssetRepository::new(isle.clone())),
            Arc::new(SqliteGroupRepository::new(isle.clone())),
            asterism_core::application::query_group_invalidation::QueryGroupInvalidator::new(
                Arc::new(NoopQueue),
            ),
        );
        let snap = svc
            .create(
                CreateSnapshotCommand {
                    persona_id: persona.to_string(),
                    asset_ids: vec![a2.to_string(), a1.to_string()],
                },
                &nobody(),
            )
            .await
            .unwrap();
        let result = svc
            .promote_to_group(
                PromoteSnapshotToGroupCommand {
                    snapshot_id: snap.id.clone(),
                    name: "Promoted".into(),
                    description: None,
                    dir_id: None,
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert_eq!(result.asset_count, 2);

        // Birth record stamped + members attached in frozen order.
        let gid: Uuid = result.group_id.parse().unwrap();
        let (origin, first): (Option<Uuid>, Uuid) = isle
            .call(move |conn| {
                let origin: Option<Uuid> = conn.query_row(
                    "SELECT origin_snapshot_id FROM bucket WHERE id = ?1",
                    params![gid],
                    |r| r.get(0),
                )?;
                let first: Uuid = conn.query_row(
                    "SELECT asset_id FROM asset_bucket WHERE bucket_id = ?1 \
                     ORDER BY position LIMIT 1",
                    params![gid],
                    |r| r.get(0),
                )?;
                Ok((origin, first))
            })
            .await
            .unwrap();
        assert_eq!(origin, Some(snap.id.parse().unwrap()));
        assert_eq!(first, *a2.as_uuid(), "frozen order preserved (a2 first)");
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn promote_volatile_selection_fuses_freeze_and_promote() {
        use crate::sqlite::repo::asset::SqliteAssetRepository;
        use crate::sqlite::repo::group::SqliteGroupRepository;
        use crate::sqlite::repo::persona::SqlitePersonaRepository;
        use crate::sqlite::repo::snapshot::SqliteSnapshotRepository;
        use asterism_contract::command::{CreateSnapshotCommand, PromoteVolatileSelectionCommand};
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let a1 = seed_asset(&isle, &persona, 100, "[]").await;
        let a2 = seed_asset(&isle, &persona, 200, "[]").await;

        let svc = asterism_core::application::SnapshotService::new(
            Arc::new(SqliteSnapshotRepository::new(isle.clone())),
            Arc::new(SqlitePersonaRepository::new(isle.clone())),
            Arc::new(SqliteAssetRepository::new(isle.clone())),
            Arc::new(SqliteGroupRepository::new(isle.clone())),
            asterism_core::application::query_group_invalidation::QueryGroupInvalidator::new(
                Arc::new(NoopQueue),
            ),
        );
        // Pre-freeze the same ordered pick so the fused path must hit
        // the content-hash reuse instead of minting a twin.
        let pre = svc
            .create(
                CreateSnapshotCommand {
                    persona_id: persona.to_string(),
                    asset_ids: vec![a2.to_string(), a1.to_string()],
                },
                &nobody(),
            )
            .await
            .unwrap();
        let result = svc
            .promote_volatile_selection(
                PromoteVolatileSelectionCommand {
                    persona_id: persona.to_string(),
                    asset_ids: vec![a2.to_string(), a1.to_string()],
                    name: "Volatile pick".into(),
                    description: None,
                    dir_id: None,
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert_eq!(result.asset_count, 2);
        assert_eq!(
            result.snapshot_id, pre.id,
            "identical pick reuses the existing snapshot (content-hash dedupe)"
        );

        // Birth record points at the (reused) freeze; frozen order kept.
        let gid: Uuid = result.group_id.parse().unwrap();
        let (origin, first): (Option<Uuid>, Uuid) = isle
            .call(move |conn| {
                let origin: Option<Uuid> = conn.query_row(
                    "SELECT origin_snapshot_id FROM bucket WHERE id = ?1",
                    params![gid],
                    |r| r.get(0),
                )?;
                let first: Uuid = conn.query_row(
                    "SELECT asset_id FROM asset_bucket WHERE bucket_id = ?1 \
                     ORDER BY position LIMIT 1",
                    params![gid],
                    |r| r.get(0),
                )?;
                Ok((origin, first))
            })
            .await
            .unwrap();
        assert_eq!(origin, Some(pre.id.parse().unwrap()));
        assert_eq!(first, *a2.as_uuid(), "frozen order preserved (a2 first)");

        // Domain round-trip (W6-a): the repo row mapping surfaces the
        // birth record so the wire GroupDto can render "promoted from".
        let found = asterism_core::domain::repository::GroupRepository::find(
            &SqliteGroupRepository::new(isle.clone()),
            &asterism_core::domain::value::GroupId::from_uuid(gid),
        )
        .await
        .unwrap()
        .expect("promoted group exists");
        assert_eq!(
            found.origin_snapshot_id.map(|s| s.to_string()),
            Some(pre.id.clone()),
            "origin_snapshot_id round-trips through the domain Group"
        );

        // Cross-persona pick is rejected before anything is written.
        // (`seed_persona` pins pack_id='p' which is UNIQUE — seed the
        // second persona inline with its own pack id.)
        let stranger_uuid = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p2', 'P2', 0, 0)",
                params![stranger_uuid],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let stranger = PersonaId::from_uuid(stranger_uuid);
        let err = svc
            .promote_volatile_selection(
                PromoteVolatileSelectionCommand {
                    persona_id: stranger.to_string(),
                    asset_ids: vec![a1.to_string()],
                    name: "Stolen pick".into(),
                    description: None,
                    dir_id: None,
                },
                &nobody(),
            )
            .await;
        assert!(err.is_err(), "assets must belong to the command persona");
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn set_query_json_rejects_manual_buckets() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let manual = seed_bucket(&isle, &persona, "manual").await;
        let repo = SqliteQueryGroupRepository::new(isle.clone());
        let err = repo
            .set_query_json(&manual, "{\"v\":1}", chrono::Utc::now())
            .await;
        assert!(err.is_err(), "manual bucket must not accept a rule");
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn evaluate_stamps_refresh_outcome_on_bucket() {
        // W4-b failure signal: every evaluate (success or failure)
        // stamps last_refresh_* on the bucket for the staleness chip.
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let target = seed_bucket(&isle, &persona, "target-qg").await;
        let svc = build_service(&isle);

        let json = format!(
            r#"{{"v":1,"filter":{{"persona_id":"{persona}"}},"sort":{{"target":"occurred_at","order":"updated","reverse":false}}}}"#
        );
        svc.evaluate_and_materialize(&json, &persona, &target)
            .await
            .unwrap();
        let read_stamp = |isle: &AsyncIsle| {
            let uuid = *target.as_uuid();
            let isle = isle.clone();
            async move {
                isle.call(move |conn| {
                    conn.query_row(
                        "SELECT last_refresh_at, last_refresh_status, last_refresh_error \
                         FROM bucket WHERE id = ?1",
                        params![uuid],
                        |r| {
                            Ok((
                                r.get::<_, Option<i64>>(0)?,
                                r.get::<_, Option<String>>(1)?,
                                r.get::<_, Option<String>>(2)?,
                            ))
                        },
                    )
                })
                .await
                .unwrap()
            }
        };
        let (at, status, error) = read_stamp(&isle).await;
        assert!(at.is_some(), "success stamps a timestamp");
        assert_eq!(status.as_deref(), Some("ok"));
        assert!(error.is_none(), "success clears the error text");

        // A malformed rule fails the evaluate AND stamps 'failed' with
        // the error body (chip tooltip source).
        let err = svc
            .evaluate_and_materialize("{not json", &persona, &target)
            .await;
        assert!(err.is_err());
        let (_, status, error) = read_stamp(&isle).await;
        assert_eq!(status.as_deref(), Some("failed"));
        assert!(
            error.as_deref().unwrap_or("").contains("parse failed"),
            "failure records the error text: {error:?}"
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn pipeline_no_search_expands_nesting_and_freezes_sort() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;

        // root -> child -> grandchild nesting; a fourth bucket receives
        // the materialized members.
        let root = seed_bucket(&isle, &persona, "root").await;
        let child = seed_bucket(&isle, &persona, "child").await;
        let grand = seed_bucket(&isle, &persona, "grand").await;
        let target = seed_bucket(&isle, &persona, "target-qg").await;
        link_buckets(&isle, &root, &child).await;
        link_buckets(&isle, &child, &grand).await;

        let a_child = seed_asset(&isle, &persona, 100, "[]").await;
        let a_grand = seed_asset(&isle, &persona, 300, "[]").await;
        let a_none = seed_asset(&isle, &persona, 200, "[]").await;
        add_to_bucket(&isle, &a_child, &child).await;
        add_to_bucket(&isle, &a_grand, &grand).await;

        let svc = build_service(&isle);
        let json = format!(
            r#"{{"v":1,"filter":{{"persona_id":"{persona}","group_ids":["{root}"]}},"sort":{{"target":"occurred_at","order":"updated","reverse":false}}}}"#
        );
        let n = svc
            .evaluate_and_materialize(&json, &persona, &target)
            .await
            .unwrap();
        assert_eq!(n, 2, "child + grandchild members; unrelated excluded");

        let rows = membership(&isle, &target).await;
        // occurred DESC: grand(300) at position 0, child(100) at position 1.
        assert_eq!(rows, vec![(*a_grand.as_uuid(), 0), (*a_child.as_uuid(), 1)]);
        assert!(!rows.iter().any(|(id, _)| *id == *a_none.as_uuid()));

        // Re-evaluate with reverse=true → ascending, fully overwriting.
        let json_rev = format!(
            r#"{{"v":1,"filter":{{"persona_id":"{persona}","group_ids":["{root}"]}},"sort":{{"target":"occurred_at","order":"updated","reverse":true}}}}"#
        );
        svc.evaluate_and_materialize(&json_rev, &persona, &target)
            .await
            .unwrap();
        let rows_rev = membership(&isle, &target).await;
        assert_eq!(
            rows_rev,
            vec![(*a_child.as_uuid(), 0), (*a_grand.as_uuid(), 1)]
        );

        driver.shutdown().await.unwrap();
    }

    /// A rule's `search_text` selects members through the SQL text
    /// predicate, so membership is exactly the assets whose body carries
    /// the term — no shortlist, no ceiling.
    #[tokio::test]
    async fn pipeline_search_text_selects_by_body_predicate() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;

        let g = seed_bucket(&isle, &persona, "g").await;
        let target = seed_bucket(&isle, &persona, "t").await;
        let a1 = seed_asset(&isle, &persona, 100, "[]").await;
        let a2 = seed_asset(&isle, &persona, 200, "[]").await;
        let a3 = seed_asset(&isle, &persona, 300, "[]").await;
        add_to_bucket(&isle, &a1, &g).await;
        add_to_bucket(&isle, &a2, &g).await;
        add_to_bucket(&isle, &a3, &g).await;

        // a1 and a3 carry the term in their body; a2 passes the group
        // filter but not the text predicate.
        seed_body(&isle, &a1, "a sunset over the harbour").await;
        seed_body(&isle, &a2, "notes about breakfast").await;
        seed_body(&isle, &a3, "sunset, again").await;

        let svc = build_service(&isle);
        let json = format!(
            r#"{{"v":1,"filter":{{"persona_id":"{persona}","group_ids":["{g}"]}},"sort":{{"target":"occurred_at","order":"updated","reverse":false}},"search_text":"sunset"}}"#
        );
        let n = svc
            .evaluate_and_materialize(&json, &persona, &target)
            .await
            .unwrap();
        assert_eq!(n, 2, "only the two assets whose body carries the term");

        let rows = membership(&isle, &target).await;
        // occurred DESC among {a1(100), a3(300)} = a3 then a1.
        assert_eq!(rows, vec![(*a3.as_uuid(), 0), (*a1.as_uuid(), 1)]);

        driver.shutdown().await.unwrap();
    }

    /// Seed a tag and return its id.
    async fn seed_tag(isle: &AsyncIsle, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        let name = name.to_string();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO tag (id, name) VALUES (?1, ?2)",
                params![id, name],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        id
    }

    async fn link_tag(isle: &AsyncIsle, asset: &AssetId, tag: Uuid) {
        let asset = *asset.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset_tag (asset_id, tag_id) VALUES (?1, ?2)",
                params![asset, tag],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    /// A stored rule's `filter` is `ListAssetsQuery` verbatim, so
    /// `tag_match` rides the v1 blob with no field of its own on
    /// `QueryGroupQuery`. This pins that it reaches the SQL the members
    /// are selected by: the same rule under `all` and under the default
    /// must materialise different memberships, or the combinator is
    /// being dropped somewhere between the JSON and the `WHERE` clause.
    #[tokio::test]
    async fn pipeline_tag_match_all_narrows_membership() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;

        let target = seed_bucket(&isle, &persona, "t").await;
        let travel = seed_tag(&isle, "travel").await;
        let summer = seed_tag(&isle, "summer").await;

        let both = seed_asset(&isle, &persona, 300, "[]").await;
        let travel_only = seed_asset(&isle, &persona, 200, "[]").await;
        let summer_only = seed_asset(&isle, &persona, 100, "[]").await;
        link_tag(&isle, &both, travel).await;
        link_tag(&isle, &both, summer).await;
        link_tag(&isle, &travel_only, travel).await;
        link_tag(&isle, &summer_only, summer).await;

        let svc = build_service(&isle);
        let rule = |combinator: &str| {
            format!(
                r#"{{"v":1,"filter":{{"persona_id":"{persona}","tag_ids":["{travel}","{summer}"]{combinator}}},"sort":{{"target":"occurred_at","order":"updated","reverse":false}}}}"#
            )
        };

        // Default (omitted `tag_match`) — the union, all three assets.
        let n = svc
            .evaluate_and_materialize(&rule(""), &persona, &target)
            .await
            .unwrap();
        assert_eq!(n, 3, "an omitted combinator keeps the OR semantic");
        assert_eq!(
            membership(&isle, &target).await,
            vec![
                (*both.as_uuid(), 0),
                (*travel_only.as_uuid(), 1),
                (*summer_only.as_uuid(), 2)
            ]
        );

        // `all` — the intersection, and the membership is replaced
        // rather than added to.
        let n = svc
            .evaluate_and_materialize(&rule(r#","tag_match":"all""#), &persona, &target)
            .await
            .unwrap();
        assert_eq!(n, 1, "only the asset carrying both tags stays a member");
        assert_eq!(membership(&isle, &target).await, vec![(*both.as_uuid(), 0)]);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn pipeline_empty_search_hits_yields_empty_membership() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let g = seed_bucket(&isle, &persona, "g").await;
        let target = seed_bucket(&isle, &persona, "t").await;
        let a1 = seed_asset(&isle, &persona, 100, "[]").await;
        add_to_bucket(&isle, &a1, &g).await;
        // A stale row in target that must be wiped even when 0 members.
        add_to_bucket(&isle, &a1, &target).await;
        // The asset has a body; it just does not carry the term. A
        // fixture with no body at all would pass for the wrong reason
        // (nothing to match rather than no match).
        seed_body(&isle, &a1, "a body that says something else").await;

        let svc = build_service(&isle);
        let json = format!(
            r#"{{"v":1,"filter":{{"persona_id":"{persona}","group_ids":["{g}"]}},"sort":{{"target":"occurred_at","order":"updated","reverse":false}},"search_text":"nomatch"}}"#
        );
        let n = svc
            .evaluate_and_materialize(&json, &persona, &target)
            .await
            .unwrap();
        assert_eq!(n, 0);
        assert!(membership(&isle, &target).await.is_empty());

        driver.shutdown().await.unwrap();
    }
}
