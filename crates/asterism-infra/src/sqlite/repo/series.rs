//! SQLite adapter for the `SeriesRepository` port — the series axis's
//! two tables (`series_strategy` / `material_series`, V73).
//!
//! Follows the crate-wide adapter convention: only `rusqlite` primitives
//! inside the isle closure, promotion into domain types outside it
//! ([`StrategyRow::into_domain`]).
//!
//! The one thing worth stating beyond that convention is where the path
//! lists are taken apart. `include` / `exclude` are JSON columns (the
//! V73 doc comment argues why they are not a side table), so the
//! `serde_json` call is the promotion step and a column this build
//! cannot parse is [`DomainError::Infra`] — a rule read as though it
//! selected nothing would derive keys nobody wrote.

use asterism_core::domain::repository::{RegisteredStrategy, SeriesRepository, UnderivedSeries};
use asterism_core::domain::series::{Decode, Path, SeriesKey, Strategy, is_series_key};
use asterism_core::domain::value::{AssetId, MimeType, StrategyId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `SeriesRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqliteSeriesRepository {
    isle: AsyncIsle,
}

impl SqliteSeriesRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// Primitive row built inside the isle closure.
struct StrategyRow {
    id: Uuid,
    name: String,
    applies_to: String,
    decode: String,
    include: String,
    exclude: String,
}

impl StrategyRow {
    const COLUMNS: &'static str = "id, name, applies_to, decode, include, exclude";

    /// The same list, qualified for a join — the walk selects a rule
    /// beside a material, and one list of columns is what keeps the
    /// select and [`from_row_at`](Self::from_row_at) in step.
    fn columns_of(alias: &str) -> String {
        Self::COLUMNS
            .split(", ")
            .map(|column| format!("{alias}.{column}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Reads the six columns starting at `first`, so a statement can put
    /// them after a material's — or, in [`RegisteredRow`], in front of
    /// the row's own provenance.
    fn from_row_at(row: &rusqlite::Row<'_>, first: usize) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(first)?,
            name: row.get(first + 1)?,
            applies_to: row.get(first + 2)?,
            decode: row.get(first + 3)?,
            include: row.get(first + 4)?,
            exclude: row.get(first + 5)?,
        })
    }

    fn into_domain(self) -> Result<Strategy, DomainError> {
        let id = self.id;
        Ok(Strategy {
            id: StrategyId::from_uuid(id),
            name: self.name,
            // Parsed, not compared as text: `applies_to` is written by
            // whoever registered the rule, and `MimeType::parse` is what
            // makes ` IMAGE/PNG; charset=binary ` and `image/png` the
            // one format they are — the normalisation `claims` relies on.
            applies_to: MimeType::parse(&self.applies_to),
            decode: Decode::parse(&self.decode)?,
            include: paths_from_json(&self.include, "include", id)?,
            exclude: paths_from_json(&self.exclude, "exclude", id)?,
        })
    }
}

/// One whole `series_strategy` row — the rule plus the three columns
/// that say where the row came from and when it was last written.
///
/// A second row type rather than three more fields on [`StrategyRow`],
/// because the derivation walk selects that one per *pair*: the cross
/// join hands out one rule beside every material, and three columns
/// nothing on that path reads would be carried the width of the library
/// times the rules. The rule columns are still spelled once —
/// [`Self::columns`] appends to [`StrategyRow::COLUMNS`] — so the two
/// statements cannot come to disagree about what a rule is.
struct RegisteredRow {
    rule: StrategyRow,
    system: i64,
    created_at: i64,
    updated_at: i64,
}

impl RegisteredRow {
    fn columns() -> String {
        format!("{}, system, created_at, updated_at", StrategyRow::COLUMNS)
    }

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            rule: StrategyRow::from_row_at(row, 0)?,
            system: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    }

    fn into_domain(self) -> Result<RegisteredStrategy, DomainError> {
        Ok(RegisteredStrategy {
            strategy: self.rule.into_domain()?,
            // Any non-zero reads as "a migration wrote this". The column
            // is written as a literal `1` by the seed and as a literal
            // `0` by `create_strategy`, so the comparison is about
            // reading a row some other build wrote rather than about the
            // two writers here.
            system: self.system != 0,
            created_at: ms_to_datetime(self.created_at)?,
            updated_at: ms_to_datetime(self.updated_at)?,
        })
    }
}

/// Reads one of the two path columns.
///
/// The nesting is the shape the column holds — a list of paths, each a
/// list of segments — and it is deserialised as exactly that rather than
/// as a `Value` walked by hand, so a column holding something else
/// (`["vdsl","script"]`, one path where a list of them belongs) is an
/// error here instead of a rule that quietly means something different.
///
/// `Infra`, not `Validation`: nothing was asked of the caller. The row
/// is unreadable, and the id is in the message because `list_strategies`
/// collects into one `Result` — without it, one bad row makes every rule
/// in the library `Err` with no way left to say which.
fn paths_from_json(json: &str, column: &str, id: Uuid) -> Result<Vec<Path>, DomainError> {
    let raw: Vec<Vec<String>> = serde_json::from_str(json).map_err(|e| {
        DomainError::Infra(anyhow::anyhow!(
            "series_strategy {id} holds a {column} column that is not a list of paths: {e}"
        ))
    })?;
    Ok(raw.into_iter().map(Path::new).collect())
}

/// The derivation walk's statement — **the only place it is written**,
/// so the test that measures its plan measures the statement that runs.
///
/// `?1` is the page limit; with `cursor`, `?2` / `?3` / `?4` are the
/// `(asset_id, ord, strategy_id)` the page resumes after.
///
/// The `JOIN` carries no `ON`, which in SQLite is a cross join, and the
/// cross is the point: the population is every material carrying metadata
/// against every registered rule. What narrows it is the `NOT EXISTS` —
/// pairs already answered — and nothing else. In particular **not the
/// mime**; `SeriesRepository::scan_underived` holds that argument.
fn underived_page_sql(cursor: bool) -> String {
    format!(
        "SELECT m.asset_id, m.ord, m.mime, m.meta_kv, {columns} \
           FROM material m \
           JOIN series_strategy s \
          WHERE m.meta_kv IS NOT NULL \
            AND NOT EXISTS (SELECT 1 FROM material_series ms \
                             WHERE ms.asset_id = m.asset_id \
                               AND ms.ord = m.ord \
                               AND ms.strategy_id = s.id) \
                {cursor_clause} \
          ORDER BY m.asset_id, m.ord, s.id \
          LIMIT ?1",
        columns = StrategyRow::columns_of("s"),
        cursor_clause = if cursor {
            "AND (m.asset_id > ?2 \
                  OR (m.asset_id = ?2 \
                      AND (m.ord > ?3 \
                           OR (m.ord = ?3 AND s.id > ?4))))"
        } else {
            ""
        }
    )
}

/// Serialises a path list for storage — the inverse of
/// [`paths_from_json`], and the only place the column's form is written.
fn paths_to_json(paths: &[Path]) -> Result<String, DomainError> {
    let raw: Vec<&[String]> = paths.iter().map(Path::segments).collect();
    serde_json::to_string(&raw)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("a path list would not serialise: {e}")))
}

#[async_trait]
impl SeriesRepository for SqliteSeriesRepository {
    async fn list_strategies(&self) -> Result<Vec<RegisteredStrategy>, DomainError> {
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM series_strategy ORDER BY created_at, id",
                    RegisteredRow::columns()
                ))?;
                let rows: Vec<RegisteredRow> = stmt
                    .query_map([], RegisteredRow::from_row)?
                    .collect::<Result<_, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(RegisteredRow::into_domain).collect()
    }

    async fn find_strategy(
        &self,
        id: &StrategyId,
    ) -> Result<Option<RegisteredStrategy>, DomainError> {
        let id = *id.as_uuid();
        let row = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM series_strategy WHERE id = ?1",
                    RegisteredRow::columns()
                ))?;
                let mut rows = stmt.query_map(params![id], RegisteredRow::from_row)?;
                rows.next().transpose()
            })
            .await
            .map_err(infra_err)?;
        row.map(RegisteredRow::into_domain).transpose()
    }

    async fn create_strategy(
        &self,
        strategy: &Strategy,
        at: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let id = *strategy.id.as_uuid();
        let name = strategy.name.clone();
        let applies_to = strategy.applies_to.as_str().to_string();
        let decode = strategy.decode.as_str().to_string();
        let include = paths_to_json(&strategy.include)?;
        let exclude = paths_to_json(&strategy.exclude)?;
        let stamp = datetime_to_ms(&at);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO series_strategy \
                         (id, name, applies_to, decode, include, exclude, \
                          system, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)",
                    params![id, name, applies_to, decode, include, exclude, stamp],
                )?;
                Ok(())
            })
            .await
            .map_err(|err| {
                // The id is the only unique column, so a collision means
                // the caller re-registered one it already holds.
                let msg = err.to_string();
                if msg.contains("UNIQUE") || msg.contains("unique") {
                    DomainError::clashes(format!("strategy {id} is already registered"))
                } else {
                    infra_err(err)
                }
            })?;
        // `system` is written as 0 here and is not a parameter: the
        // column says a *migration* seeded the row (see the V73 doc), so
        // a value arriving over this port cannot claim it.
        Ok(())
    }

    /// Overwrites the five rule fields and moves `updated_at`.
    ///
    /// `system` and `created_at` are absent from the `SET` list rather
    /// than being written back unchanged: a statement that names them is
    /// a statement that can get them wrong, and what they record —
    /// *a migration wrote this row, then* — is not something a later
    /// edit can make truer.
    ///
    /// The stamp is the other half of V73's identification test
    /// (`system = 1 AND updated_at = created_at`). Leaving it alone
    /// would make an edited seed read as pristine to the next corrective
    /// migration, which would then overwrite somebody's rule with the
    /// one the migration meant to fix.
    async fn update_strategy(
        &self,
        strategy: &Strategy,
        at: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let id = *strategy.id.as_uuid();
        let name = strategy.name.clone();
        let applies_to = strategy.applies_to.as_str().to_string();
        let decode = strategy.decode.as_str().to_string();
        let include = paths_to_json(&strategy.include)?;
        let exclude = paths_to_json(&strategy.exclude)?;
        let stamp = datetime_to_ms(&at);
        let changed = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE series_strategy SET \
                         name = ?2, applies_to = ?3, decode = ?4, \
                         include = ?5, exclude = ?6, updated_at = ?7 \
                      WHERE id = ?1",
                    params![id, name, applies_to, decode, include, exclude, stamp],
                )
            })
            .await
            .map_err(infra_err)?;
        if changed == 0 {
            return Err(DomainError::not_found("series strategy", id));
        }
        Ok(())
    }

    /// Removes a rule, and with it — by the V73 cascade — every answer
    /// filed under it.
    ///
    /// The row count is checked rather than discarded so a caller that
    /// named nothing hears so. Deleting a rule discards keys, and a
    /// silent success on a mistyped id reads as "your library changed".
    async fn delete_strategy(&self, id: &StrategyId) -> Result<(), DomainError> {
        let id = *id.as_uuid();
        let removed = self
            .isle
            .call(move |conn| {
                conn.execute("DELETE FROM series_strategy WHERE id = ?1", params![id])
            })
            .await
            .map_err(infra_err)?;
        if removed == 0 {
            return Err(DomainError::not_found("series strategy", id));
        }
        Ok(())
    }

    /// The invalidation statement, written out: the same
    /// `DELETE … WHERE strategy_id = ?` SQLite runs for the cascade
    /// above, aimed at the derived table alone so the rule survives it.
    ///
    /// V74's index is what keeps it a seek; without one this is a scan of
    /// the library times the rules, per edit
    /// (`the_per_strategy_delete_is_served_by_the_strategy_index`
    /// measures the plan).
    async fn clear_derived(&self, id: &StrategyId) -> Result<u64, DomainError> {
        let id = *id.as_uuid();
        let removed = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM material_series WHERE strategy_id = ?1",
                    params![id],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(removed as u64)
    }

    /// Refuses a reserved key at the door rather than filing it.
    ///
    /// [`SeriesKey::Derived`] carries a `String` a caller can build, so
    /// "derive never produces the empty digest" is a property of
    /// [`derive`](asterism_core::domain::series::derive) and not of this
    /// column. Written, it would be a well-formed `sk1-` value nothing
    /// downstream could tell from a real one, and every material whose
    /// rule selected nothing would share it — the group the constant is
    /// reserved against. The same refusal `find_by_content_hash` makes
    /// against a value that is not a duplicate key.
    async fn record(
        &self,
        asset_id: &AssetId,
        ord: u32,
        strategy_id: &StrategyId,
        key: &SeriesKey,
        at: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if let Some(value) = key.key()
            && !is_series_key(value)
        {
            return Err(DomainError::Validation(format!(
                "{value:?} is not a value this axis groups on"
            )));
        }
        let asset = *asset_id.as_uuid();
        let ord = i64::from(ord);
        let strategy = *strategy_id.as_uuid();
        let outcome = key.outcome_slug().to_string();
        let stored_key = key.key().map(str::to_string);
        let stamp = datetime_to_ms(&at);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO material_series \
                         (asset_id, ord, strategy_id, key, outcome, derived_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                     ON CONFLICT(asset_id, ord, strategy_id) DO UPDATE SET \
                         key = excluded.key, \
                         outcome = excluded.outcome, \
                         derived_at = excluded.derived_at",
                    params![asset, ord, strategy, stored_key, outcome, stamp],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    /// One page of the derivation walk.
    ///
    /// The population is a cross join — every material carrying a
    /// metadata object, every registered rule — minus the pairs already
    /// answered. The port's doc holds the argument for both halves of
    /// that shape: why the mime is not compared here, and why a caller
    /// has to file an answer for every pair it is handed.
    ///
    /// `NOT EXISTS` rather than a `LEFT JOIN … WHERE ms.asset_id IS NULL`
    /// because the two say the same thing and only one of them says it
    /// where the reader is: the subquery is a point lookup on
    /// `material_series`' full primary key, which — the table being
    /// `WITHOUT ROWID` — is the table itself, so the plan is a seek per
    /// pair with nothing materialised.
    /// `the_series_walk_asks_material_series_by_primary_key` measures it.
    ///
    /// The cursor is spelled out as nested comparisons rather than as a
    /// row value, matching `scan_unhashed_materials`: one idiom for every
    /// composite cursor in this codebase.
    ///
    /// # One unreadable rule stops the whole axis
    ///
    /// Every rule on the page is promoted through
    /// [`StrategyRow::into_domain`], so a `series_strategy` row this build
    /// cannot read — a `decode` token from a later build, an `include`
    /// column that is not a list of paths — makes **every** page `Err`,
    /// and no material gets a key under *any* rule. Failing loud is the
    /// right direction and it is S2's decision (deriving under a rule
    /// nobody wrote is the alternative); what S3 adds is the blast
    /// radius, which is the axis rather than the offending rule. The
    /// reachable case is a downgrade after
    /// [`Decode::Exif`](asterism_core::domain::series::Decode) ships,
    /// which that enum's doc already calls a scheduled event. Narrowing it
    /// — skipping the unreadable rule and saying so, rather than failing
    /// the page — needs a place to put "this rule is unreadable" that a
    /// person will see, and there is no rule-status surface yet.
    async fn scan_underived(
        &self,
        after: Option<(&AssetId, u32, &StrategyId)>,
        limit: u32,
    ) -> Result<Vec<UnderivedSeries>, DomainError> {
        let cursor =
            after.map(|(id, ord, strategy)| (*id.as_uuid(), i64::from(ord), *strategy.as_uuid()));
        let limit = i64::from(limit);
        let rows: Vec<(Uuid, i64, Option<String>, Option<String>, StrategyRow)> = self
            .isle
            .call(move |conn| {
                let read = |r: &rusqlite::Row<'_>| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        StrategyRow::from_row_at(r, 4)?,
                    ))
                };
                match cursor {
                    None => {
                        let mut stmt = conn.prepare(&underived_page_sql(false))?;
                        stmt.query_map(params![limit], read)?
                            .collect::<Result<_, _>>()
                    }
                    Some((asset, ord, strategy)) => {
                        let mut stmt = conn.prepare(&underived_page_sql(true))?;
                        stmt.query_map(params![limit, asset, ord, strategy], read)?
                            .collect::<Result<_, _>>()
                    }
                }
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(asset_id, ord, mime, meta_kv, strategy)| {
                Ok(UnderivedSeries {
                    asset_id: AssetId::from_uuid(asset_id),
                    ord: ord.max(0) as u32,
                    // Parsed, like every other reader of this column: the
                    // rule's `applies_to` is parsed too, and a comparison
                    // between a parsed value and a raw one is the silent
                    // half of `Strategy::claims` going wrong.
                    mime: mime.as_deref().map(MimeType::parse),
                    // A column that will not parse is an empty map, not
                    // an error and not a skipped row — the port's doc
                    // says why: this walk shrinks only by answering, and
                    // a row it refuses to hand out is a pair no pass ever
                    // answers.
                    meta_kv: meta_kv
                        .as_deref()
                        .and_then(|json| serde_json::from_str(json).ok())
                        .unwrap_or_default(),
                    // The rule is promoted by the same path
                    // `list_strategies` uses, so a column this build
                    // cannot read fails here rather than deriving keys
                    // under a rule nobody wrote.
                    strategy: strategy.into_domain()?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_core::error::ConflictKind;

    /// The rule V73 seeds, addressed by the id the migration froze.
    const VDSL_STRATEGY_ID: &str = "019fe8f8-1400-7000-8000-000000000001";

    /// A fixed moment, because the column stores epoch milliseconds and
    /// a clock read carrying microseconds would not survive the trip.
    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp_millis(1_786_320_000_000).unwrap()
    }

    async fn seed_persona(isle: &AsyncIsle) -> Uuid {
        let pid = Uuid::now_v7();
        let pack = format!("pack-{pid}");
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
                 VALUES (?1, ?2, 'P', 0, 0)",
                params![pid, pack],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        pid
    }

    /// One PNG asset with one material — what a derived row hangs off.
    async fn seed_material(isle: &AsyncIsle, persona: Uuid) -> AssetId {
        seed_material_carrying(isle, persona, "image/png", None).await
    }

    /// The same, with the two columns the walk reads spelled out: what
    /// the row says it is, and what its container carried.
    async fn seed_material_carrying(
        isle: &AsyncIsle,
        persona: Uuid,
        mime: &str,
        meta_kv: Option<&str>,
    ) -> AssetId {
        let aid = Uuid::now_v7();
        let mime = mime.to_string();
        let meta_kv = meta_kv.map(str::to_string);
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                    modality, occurred_at, created_at, updated_at) \
                 VALUES (?1, ?2, 'fs', ?3, 'tape', 0, 0, 0)",
                params![aid, persona, format!("/pics/{aid}.png")],
            )?;
            conn.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, meta_kv, \
                                       created_at, updated_at) \
                 VALUES (?1, 0, ?2, ?3, ?4, 0, 0)",
                params![
                    aid,
                    format!("{{\"kind\":\"file\",\"path\":\"/pics/{aid}.png\"}}"),
                    mime,
                    meta_kv
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        AssetId::from_uuid(aid)
    }

    /// A **secondary** original beside the first — the RAW next to the
    /// JPEG, `ord = 1`, its own container and so its own metadata.
    ///
    /// The fixture nothing else here has. `material_series` is keyed by
    /// `(asset_id, ord, strategy_id)` and the port files an answer per
    /// original, but every other seed in this file writes `ord = 0`, so
    /// an `AND m.ord = 0` added to the walk would leave the whole suite
    /// green while every secondary original in a library silently lost
    /// its keys.
    async fn attach_secondary_material(
        isle: &AsyncIsle,
        asset: AssetId,
        mime: &str,
        meta_kv: Option<&str>,
    ) -> AssetId {
        let aid = *asset.as_uuid();
        let mime = mime.to_string();
        let meta_kv = meta_kv.map(str::to_string);
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, meta_kv, \
                                       created_at, updated_at) \
                 VALUES (?1, 1, ?2, ?3, ?4, 0, 0)",
                params![
                    aid,
                    format!("{{\"kind\":\"file\",\"path\":\"/pics/{aid}.raw\"}}"),
                    mime,
                    meta_kv
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        asset
    }

    /// A `vdsl` chunk as the container carries it, built by the
    /// serialiser rather than typed out — a chunk one escape away from
    /// being invalid JSON lands on `NothingToSelect` and would read as
    /// the walk having failed.
    fn vdsl_meta_kv(script: &str) -> String {
        serde_json::to_string(&serde_json::json!({
            "vdsl": serde_json::to_string(&serde_json::json!({
                "script": script,
                "version": "0.4.0",
            }))
            .unwrap(),
        }))
        .unwrap()
    }

    /// A well-formed rule under a fresh id — registered nowhere until
    /// somebody registers it, which is what a caller naming an unknown
    /// Strategy hands over.
    fn strategy_named(name: &str) -> Strategy {
        Strategy {
            id: StrategyId::new(),
            name: name.to_string(),
            applies_to: MimeType::parse("image/png"),
            decode: Decode::RawJson,
            include: vec![Path::new(["vdsl", "script"])],
            exclude: vec![],
        }
    }

    /// A second rule beside the seeded one, so "this rule's rows" is a
    /// statement a test can falsify.
    async fn register_second_rule(repo: &SqliteSeriesRepository) -> StrategyId {
        let card = Strategy {
            id: StrategyId::new(),
            name: "character card".to_string(),
            applies_to: MimeType::parse("image/png"),
            decode: Decode::Base64Json,
            include: vec![Path::new(["ccv3", "data", "name"])],
            exclude: vec![],
        };
        repo.create_strategy(&card, at()).await.unwrap();
        card.id
    }

    /// What one derived row holds, read past the port (which does not
    /// read them back yet).
    async fn filed(isle: &AsyncIsle, asset: AssetId) -> Vec<(Option<String>, String, i64)> {
        let aid = *asset.as_uuid();
        isle.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT key, outcome, derived_at FROM material_series \
                  WHERE asset_id = ?1 ORDER BY strategy_id",
            )?;
            let rows = stmt
                .query_map(params![aid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
    }

    /// The seeded rule comes back off the port as the rule the
    /// migration wrote — decoder, mime and paths intact.
    ///
    /// The path assertion is the load-bearing one: the column is JSON,
    /// so the round trip crosses a serialiser twice, and the shape that
    /// survives has to be the nesting (a list of paths, each a list of
    /// segments) rather than a flattened list of segments that would
    /// read as one path and select something else entirely. It is
    /// checked by *applying* the rule as well — the same measurement
    /// `domain/series.rs` freezes, run through the rule as stored.
    ///
    /// Checked by mutation on 2026-08-10: with `paths_from_json`
    /// building one segment per path (`segments.join(".")`, the dotted
    /// convention somebody will eventually reach for), this failed —
    /// left `[Path(["vdsl.script"])]`, right `[Path(["vdsl", "script"])]`.
    /// Restored, it passes.
    #[tokio::test]
    async fn the_seeded_vdsl_rule_round_trips_with_its_paths_intact() {
        use asterism_core::domain::series::derive;
        use std::collections::BTreeMap;

        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSeriesRepository::new(isle.clone());

        let listed = repo.list_strategies().await.unwrap();
        assert_eq!(listed.len(), 1, "one system rule is seeded");
        let seeded = &listed[0];
        assert!(seeded.system, "the migration wrote it");
        assert_eq!(
            seeded.created_at, seeded.updated_at,
            "and nothing has edited it — the pair a corrective migration \
             tells a pristine seed by"
        );
        let vdsl = &seeded.strategy;
        assert_eq!(
            *vdsl.id.as_uuid(),
            Uuid::parse_str(VDSL_STRATEGY_ID).unwrap()
        );
        assert_eq!(vdsl.name, "VDSL recipe");
        assert_eq!(vdsl.applies_to, MimeType::parse("image/png"));
        assert_eq!(vdsl.decode, Decode::RawJson);
        assert_eq!(vdsl.include, vec![Path::new(["vdsl", "script"])]);
        assert!(vdsl.exclude.is_empty());

        // And it still reads the container it was measured against: two
        // images off one run land on one key, a third off another run
        // does not.
        let png = MimeType::parse("image/png");
        // Built by the serialiser, not written by hand — the discipline
        // `domain/series.rs` states over its own fixtures. A chunk typed
        // out as a string literal is one escape away from being a value
        // the decoder refuses, which lands on `NothingToSelect` and
        // would read here as the rule having failed to survive storage.
        let image = |script: &str, seed: u64| -> BTreeMap<String, String> {
            let chunk = |value: serde_json::Value| {
                serde_json::to_string(&value).expect("the fixture is built by the serialiser")
            };
            BTreeMap::from([
                (
                    "vdsl".to_string(),
                    chunk(serde_json::json!({"script": script, "version": "0.4.0"})),
                ),
                (
                    "prompt".to_string(),
                    chunk(serde_json::json!({"seed": seed})),
                ),
            ])
        };
        let first = derive(vdsl, Some(&png), &image("phase8_hires.lua", 1));
        let same_run = derive(vdsl, Some(&png), &image("phase8_hires.lua", 2));
        let other_run = derive(vdsl, Some(&png), &image("phase9_portrait.lua", 3));
        assert!(first.key().is_some(), "{first:?}");
        assert_eq!(first, same_run, "one script, one key");
        assert_ne!(first, other_run, "two scripts, two keys");

        driver.shutdown().await.unwrap();
    }

    /// A registered rule round-trips beside the seeded one, and the
    /// paths it was registered with come back in the order and the
    /// multiplicity they were written in.
    ///
    /// The fixture repeats a path and puts two of them out of sorted
    /// order on purpose. Neither moves a key — `select` files by path in
    /// a `BTreeMap` — so nothing but this assertion would notice a
    /// column that normalised what it stored, and a person reading their
    /// own rule back would find it edited.
    #[tokio::test]
    async fn a_registered_rule_keeps_the_paths_it_was_written_with() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSeriesRepository::new(isle.clone());

        let card = Strategy {
            id: StrategyId::new(),
            name: "character card".to_string(),
            applies_to: MimeType::parse("image/png"),
            decode: Decode::Base64Json,
            include: vec![
                Path::new(["ccv3", "data", "name"]),
                Path::new(["ccv3", "spec"]),
                Path::new(["ccv3", "data", "name"]),
            ],
            exclude: vec![Path::new(["ccv3", "data", "description"])],
        };
        repo.create_strategy(&card, at()).await.unwrap();

        let listed = repo.list_strategies().await.unwrap();
        assert_eq!(listed.len(), 2, "the seed and the registered rule");
        let read_back = listed
            .iter()
            .find(|s| s.strategy.id == card.id)
            .expect("registered under the id the caller holds");
        assert_eq!(
            read_back.strategy, card,
            "every field, including both path lists"
        );
        assert!(
            !read_back.system,
            "a rule arriving over the port is not a seed"
        );

        // The same row through the single-id read the partial update
        // resolves against, so a `PATCH` and a listing cannot disagree
        // about what is stored.
        let found = repo
            .find_strategy(&card.id)
            .await
            .unwrap()
            .expect("registered a moment ago");
        assert_eq!(found, *read_back);
        assert_eq!(
            repo.find_strategy(&StrategyId::new()).await.unwrap(),
            None,
            "an id nothing is registered under is absence, not an error"
        );

        // The id is the identity, so registering it twice is a conflict
        // rather than a second rule.
        assert!(matches!(
            repo.create_strategy(&card, at()).await,
            Err(DomainError::Conflict {
                kind: ConflictKind::Clashes,
                ..
            })
        ));
        // …and a rule arriving over the port is not a system row,
        // whatever it calls itself.
        let system: i64 = isle
            .call({
                let id = *card.id.as_uuid();
                move |conn| {
                    conn.query_row(
                        "SELECT system FROM series_strategy WHERE id = ?1",
                        params![id],
                        |r| r.get(0),
                    )
                }
            })
            .await
            .unwrap();
        assert_eq!(system, 0, "only a migration seeds a system row");

        driver.shutdown().await.unwrap();
    }

    /// Each of the three answers is filed as itself, and re-deriving
    /// replaces the row rather than adding one.
    ///
    /// The two silences are the pair that would be indistinguishable
    /// under an `Option<String>` signature: both store no key, and only
    /// `outcome` says which of them a reader is looking at.
    #[tokio::test]
    async fn all_three_outcomes_are_filed_as_themselves_and_a_re_derive_replaces() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSeriesRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let strategy = StrategyId::from_uuid(Uuid::parse_str(VDSL_STRATEGY_ID).unwrap());
        let key = format!("sk1-sha256:{}", "a".repeat(64));

        for (answer, expected) in [
            (
                SeriesKey::Derived(key.clone()),
                (Some(key.clone()), "derived"),
            ),
            (SeriesKey::NothingToSelect, (None, "nothing_to_select")),
            (SeriesKey::NotApplicable, (None, "not_applicable")),
        ] {
            let asset = seed_material(&isle, persona).await;
            repo.record(&asset, 0, &strategy, &answer, at())
                .await
                .unwrap();
            assert_eq!(
                filed(&isle, asset).await,
                vec![(expected.0, expected.1.to_string(), 1_786_320_000_000)],
                "{answer:?} did not come back as itself"
            );
        }

        // A rule re-run over one material answers again; it does not
        // answer twice.
        let asset = seed_material(&isle, persona).await;
        repo.record(&asset, 0, &strategy, &SeriesKey::NothingToSelect, at())
            .await
            .unwrap();
        let later = at() + chrono::Duration::seconds(60);
        repo.record(
            &asset,
            0,
            &strategy,
            &SeriesKey::Derived(key.clone()),
            later,
        )
        .await
        .unwrap();
        assert_eq!(
            filed(&isle, asset).await,
            vec![(Some(key), "derived".to_string(), datetime_to_ms(&later))],
            "one material, one rule, one answer — the latest"
        );

        driver.shutdown().await.unwrap();
    }

    /// The reserved key is refused, and the refusal is a caller error
    /// rather than an infrastructure one.
    ///
    /// `SERIES_KEY_EMPTY` is a well-formed `sk1-` value — the schema has
    /// nothing to say about it, and neither would a reader — so the
    /// assertion that the row was *not* written is the one that matters:
    /// filed, it would put every material whose rule selected nothing
    /// into one group.
    #[tokio::test]
    async fn the_reserved_key_never_reaches_the_column() {
        use asterism_core::domain::series::SERIES_KEY_EMPTY;

        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSeriesRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let strategy = StrategyId::from_uuid(Uuid::parse_str(VDSL_STRATEGY_ID).unwrap());
        let asset = seed_material(&isle, persona).await;

        let err = repo
            .record(
                &asset,
                0,
                &strategy,
                &SeriesKey::Derived(SERIES_KEY_EMPTY.to_string()),
                at(),
            )
            .await
            .expect_err("the digest of an empty selection is not a grouping");
        assert!(
            matches!(err, DomainError::Validation(_)),
            "a caller handing over a value the axis reserves is a caller error, got {err:?}"
        );
        assert!(
            filed(&isle, asset).await.is_empty(),
            "the refused answer must not have reached the row"
        );

        // The neighbouring value — same shape, not reserved — is filed,
        // so the guard is about this value and not about `sk1-` at large.
        repo.record(
            &asset,
            0,
            &strategy,
            &SeriesKey::Derived(format!("sk1-sha256:{}", "b".repeat(64))),
            at(),
        )
        .await
        .unwrap();
        assert_eq!(filed(&isle, asset).await.len(), 1);

        driver.shutdown().await.unwrap();
    }

    /// The plan behind V73's index: "which materials did this rule put
    /// on this key" is served by `idx_material_series_strategy_key`, not
    /// by a scan.
    ///
    /// The same shape as
    /// `the_hash_lookup_is_served_by_the_content_hash_index`, and for
    /// the same reason — the statement is the one an S3 grouping query
    /// will run, and a partial index is only reached when the predicate
    /// implies its `WHERE`, which is a property of the pair rather than
    /// of the index alone.
    ///
    /// Checked by mutation on 2026-08-10: with the `CREATE INDEX`
    /// commented out of V73, this failed — *"the rule-and-key lookup
    /// planned without its index: SCAN material_series"*. Restored, it
    /// passes.
    #[tokio::test]
    async fn the_series_lookup_is_served_by_the_strategy_key_index() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();

        let plan: Vec<String> = isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "EXPLAIN QUERY PLAN \
                     SELECT asset_id, ord FROM material_series \
                      WHERE strategy_id = ?1 AND key = ?2",
                )?;
                stmt.query_map(
                    params![Uuid::now_v7(), format!("sk1-sha256:{}", "a".repeat(64))],
                    |r| r.get::<_, String>(3),
                )?
                .collect::<Result<_, _>>()
            })
            .await
            .unwrap();
        let plan_text = plan.join("\n");

        assert!(
            plan_text.contains("idx_material_series_strategy_key"),
            "the rule-and-key lookup planned without its index:\n{plan_text}"
        );
        assert!(
            !plan_text.contains("SCAN "),
            "something turned into a scan:\n{plan_text}"
        );

        driver.shutdown().await.unwrap();
    }

    /// The walk hands out every unanswered `(material, rule)` pair, and
    /// a pair leaves it by acquiring a row — whatever that row says.
    ///
    /// Three things are load bearing in the fixture, and each is the
    /// mutation the assertion below it catches:
    ///
    /// - **a JPEG carrying the `vdsl` chunk.** The rule is written
    ///   against PNG, so the pair is one the derivation declines — and it
    ///   is still offered here, because the mime gate is `derive`'s and
    ///   not this statement's. Measured on 2026-08-10 by adding
    ///   `AND m.mime = s.applies_to` to `underived_page_sql`: *left `2`,
    ///   right `4`* — both rules in this fixture are `image/png`, so a
    ///   SQL equality drops **both** of the JPEG's pairs, and the two
    ///   answers it owes are never filed.
    /// - **a material with no metadata object.** `meta_kv IS NULL` is not
    ///   a pair at all: nothing has walked that container, so there is
    ///   nothing for any rule to read.
    /// - **two rules.** With one, "every pair" and "every material" are
    ///   the same sentence and the cross join is untested.
    ///
    /// The last assertion is the invariant the whole slice rests on: with
    /// every answer filed, including the two that are not keys, the page
    /// is empty. Checked by mutation on 2026-08-10 by skipping the
    /// `record` for anything but `Derived` — the walk came back holding
    /// every pair the two rules had declined, and a chain enqueueing on a
    /// non-empty page would have run for as long as the process lived.
    #[tokio::test]
    async fn the_walk_offers_every_pair_once_and_empties_as_the_answers_land() {
        use asterism_core::domain::series::derive;

        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSeriesRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let card_rule = register_second_rule(&repo).await;
        let vdsl_rule = StrategyId::from_uuid(Uuid::parse_str(VDSL_STRATEGY_ID).unwrap());

        let chunk = vdsl_meta_kv("phase8_hires.lua");
        let png = seed_material_carrying(&isle, persona, "image/png", Some(&chunk)).await;
        let jpeg = seed_material_carrying(&isle, persona, "image/jpeg", Some(&chunk)).await;
        let unwalked = seed_material_carrying(&isle, persona, "image/png", None).await;

        let page = repo.scan_underived(None, 50).await.unwrap();
        assert_eq!(
            page.len(),
            4,
            "two materials carrying metadata against two rules: {page:#?}"
        );
        assert!(
            page.iter().all(|pair| pair.asset_id != unwalked),
            "a material whose container was never walked is not a pair yet"
        );
        assert_eq!(
            page.iter().filter(|pair| pair.asset_id == jpeg).count(),
            2,
            "a pair the rule will decline is still a pair the walk owes an answer"
        );

        // What one pair carries: enough to derive, and nothing that
        // would need a file.
        let pair = page
            .iter()
            .find(|pair| pair.asset_id == png && pair.strategy.id == vdsl_rule)
            .expect("the PNG under the seeded rule");
        assert_eq!(pair.ord, 0);
        assert_eq!(pair.mime, Some(MimeType::parse("image/png")));
        assert_eq!(
            pair.meta_kv.keys().collect::<Vec<_>>(),
            vec!["vdsl"],
            "the container's map, taken apart"
        );
        assert_eq!(pair.strategy.include, vec![Path::new(["vdsl", "script"])]);
        assert!(
            page.iter().any(|pair| pair.strategy.id == card_rule),
            "the registered rule is walked beside the seeded one"
        );

        // Answer each pair the way the handler does, then ask again.
        let mut declined = 0usize;
        for pair in &page {
            let key = derive(&pair.strategy, pair.mime.as_ref(), &pair.meta_kv);
            if key == SeriesKey::NotApplicable {
                declined += 1;
            }
            repo.record(&pair.asset_id, pair.ord, &pair.strategy.id, &key, at())
                .await
                .unwrap();
        }
        assert!(
            declined > 0,
            "the fixture says nothing about termination unless some pair was declined"
        );
        assert!(
            repo.scan_underived(None, 50).await.unwrap().is_empty(),
            "every pair was answered, so the walk is empty — this is the only \
             reason it ever ends"
        );

        driver.shutdown().await.unwrap();
    }

    /// A secondary original is a pair of its own, and gets its own key.
    ///
    /// The walk's population is materials, not assets. Every other
    /// fixture in this file and in `jobs/mod.rs` writes `ord = 0`, so
    /// this is the only thing standing between the library and an
    /// `AND m.ord = 0` added to `underived_page_sql` — which reads as
    /// harmless (an asset has one original today), leaves the whole suite
    /// green, and drops every secondary original out of the axis the
    /// moment the RAW-beside-the-JPEG wave lands. The `ord` column is in
    /// `material_series`' primary key precisely because the two
    /// containers are two containers.
    ///
    /// The two materials carry **different** chunks, so "both filed" is
    /// checked as two different keys rather than as two rows: a walk that
    /// handed the primary's metadata to both would satisfy a row count.
    ///
    /// Checked by mutation on 2026-08-10 by adding `AND m.ord = 0` to
    /// `underived_page_sql`: *left `[0]`, right `[0, 1]`* here, and
    /// *left `4`, right `6`* in
    /// `the_cursor_walks_every_pair_across_pages_of_one`. Before this
    /// test the same edit was silent across the whole suite.
    #[tokio::test]
    async fn a_secondary_original_is_its_own_pair_and_gets_its_own_key() {
        use asterism_core::domain::series::derive;

        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSeriesRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let asset = seed_material_carrying(
            &isle,
            persona,
            "image/png",
            Some(&vdsl_meta_kv("phase8_hires.lua")),
        )
        .await;
        attach_secondary_material(
            &isle,
            asset,
            "image/png",
            Some(&vdsl_meta_kv("phase9_portrait.lua")),
        )
        .await;

        let page = repo.scan_underived(None, 50).await.unwrap();
        assert_eq!(
            page.iter().map(|pair| pair.ord).collect::<Vec<_>>(),
            vec![0, 1],
            "one asset, two originals, two pairs under the one seeded rule: {page:#?}"
        );

        for pair in &page {
            let key = derive(&pair.strategy, pair.mime.as_ref(), &pair.meta_kv);
            repo.record(&pair.asset_id, pair.ord, &pair.strategy.id, &key, at())
                .await
                .unwrap();
        }

        let filed: Vec<(i64, Option<String>)> = isle
            .call({
                let aid = *asset.as_uuid();
                move |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT ord, key FROM material_series WHERE asset_id = ?1 ORDER BY ord",
                    )?;
                    stmt.query_map(params![aid], |r| Ok((r.get(0)?, r.get(1)?)))?
                        .collect::<Result<Vec<_>, _>>()
                }
            })
            .await
            .unwrap();
        assert_eq!(filed.len(), 2, "an answer per original: {filed:#?}");
        assert_eq!(filed[0].0, 0);
        assert_eq!(filed[1].0, 1);
        assert!(filed.iter().all(|(_, key)| key.is_some()), "{filed:#?}");
        assert_ne!(
            filed[0].1, filed[1].1,
            "each original was read as itself — the recipes differ, so the keys do"
        );
        assert!(repo.scan_underived(None, 50).await.unwrap().is_empty());

        driver.shutdown().await.unwrap();
    }

    /// The composite cursor walks the pairs one page at a time without
    /// repeating one or stepping over one.
    ///
    /// A page of one is the size that exposes the cursor, and the fixture
    /// makes each of its three parts the discriminator for at least one
    /// step: one asset carries **two** originals and there are two rules,
    /// so the walk steps `(a,0,ruleA) → (a,0,ruleB)` on the rule alone,
    /// `(a,0,ruleB) → (a,1,ruleA)` on the ord alone, and
    /// `(a,1,ruleB) → (b,0,ruleA)` on the asset. A two-part
    /// `(asset_id, ord)` cursor skips the remaining rules of a material a
    /// page ended inside; a cursor that compared only the asset skips the
    /// secondary original.
    #[tokio::test]
    async fn the_cursor_walks_every_pair_across_pages_of_one() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSeriesRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        register_second_rule(&repo).await;
        let chunk = vdsl_meta_kv("phase8_hires.lua");
        let two_originals = seed_material_carrying(&isle, persona, "image/png", Some(&chunk)).await;
        attach_secondary_material(&isle, two_originals, "image/png", Some(&chunk)).await;
        seed_material_carrying(&isle, persona, "image/png", Some(&chunk)).await;

        let whole = repo.scan_underived(None, 50).await.unwrap();
        assert_eq!(whole.len(), 6, "three materials, two rules");
        assert!(
            whole.iter().any(|pair| pair.ord == 1),
            "the fixture says nothing about the ord leg unless a secondary original is in it"
        );

        let mut walked: Vec<(AssetId, u32, StrategyId)> = Vec::new();
        let mut cursor: Option<(AssetId, u32, StrategyId)> = None;
        loop {
            let page = repo
                .scan_underived(cursor.as_ref().map(|(id, ord, rule)| (id, *ord, rule)), 1)
                .await
                .unwrap();
            let Some(pair) = page.into_iter().next() else {
                break;
            };
            let step = (pair.asset_id, pair.ord, pair.strategy.id);
            cursor = Some(step);
            walked.push(step);
            assert!(
                walked.len() <= whole.len(),
                "the cursor is not advancing: {walked:#?}"
            );
        }

        assert_eq!(
            walked,
            whole
                .iter()
                .map(|pair| (pair.asset_id, pair.ord, pair.strategy.id))
                .collect::<Vec<_>>(),
            "pages of one visit the same pairs, in the same order, as one page of six"
        );
        assert!(
            walked
                .windows(2)
                .any(|step| step[0].0 == step[1].0 && step[0].1 != step[1].1),
            "one step has to be discriminated by the ord alone: {walked:#?}"
        );

        driver.shutdown().await.unwrap();
    }

    /// Deleting a rule takes the keys derived under it and leaves every
    /// other rule's rows where they are.
    ///
    /// Both halves matter. The first is what makes an edit expressible at
    /// all — invalidation on this axis *is* deleting a rule's rows, and
    /// the walk then derives them again (V74). The second is what says
    /// the cascade is scoped: a delete that took the table with it would
    /// silently re-derive the whole library, which on a big one is the
    /// difference between a keystroke and a sweep.
    ///
    /// Checked by mutation on 2026-08-10: with the `strategy_id` foreign
    /// key's `ON DELETE CASCADE` removed from V73, this failed at the
    /// delete itself — *"FOREIGN KEY constraint failed"* — which is the
    /// schema saying a rule cannot be deleted while anything derived
    /// under it stands, and "change a rule and watch the groups move"
    /// with it. Restored, it passes.
    #[tokio::test]
    async fn deleting_a_rule_takes_its_own_derived_rows_and_leaves_the_others() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSeriesRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let vdsl_rule = StrategyId::from_uuid(Uuid::parse_str(VDSL_STRATEGY_ID).unwrap());
        let card_rule = register_second_rule(&repo).await;
        let asset = seed_material_carrying(
            &isle,
            persona,
            "image/png",
            Some(&vdsl_meta_kv("phase8_hires.lua")),
        )
        .await;

        repo.record(
            &asset,
            0,
            &vdsl_rule,
            &SeriesKey::Derived(format!("sk1-sha256:{}", "c".repeat(64))),
            at(),
        )
        .await
        .unwrap();
        repo.record(&asset, 0, &card_rule, &SeriesKey::NotApplicable, at())
            .await
            .unwrap();
        assert_eq!(filed(&isle, asset).await.len(), 2, "both rules answered");

        let removed = isle
            .call({
                let id = *vdsl_rule.as_uuid();
                move |conn| conn.execute("DELETE FROM series_strategy WHERE id = ?1", params![id])
            })
            .await
            .unwrap();
        assert_eq!(removed, 1, "the fixture must actually delete the rule");

        let left: Vec<Uuid> = isle
            .call({
                let aid = *asset.as_uuid();
                move |conn| {
                    let mut stmt = conn
                        .prepare("SELECT strategy_id FROM material_series WHERE asset_id = ?1")?;
                    stmt.query_map(params![aid], |r| r.get(0))?
                        .collect::<Result<Vec<_>, _>>()
                }
            })
            .await
            .unwrap();
        assert_eq!(
            left,
            vec![*card_rule.as_uuid()],
            "the deleted rule's key is gone and the other rule's answer is untouched"
        );

        // And the pair the deletion freed is not back in the walk — the
        // rule it belonged to no longer exists, so there is nothing to
        // re-derive it under.
        let page = repo.scan_underived(None, 50).await.unwrap();
        assert!(
            page.is_empty(),
            "one rule left, and its answer is already filed: {page:#?}"
        );

        driver.shutdown().await.unwrap();
    }

    /// An edit rewrites the rule, moves `updated_at`, and leaves the
    /// provenance columns exactly where they were.
    ///
    /// Run against the **seeded** rule, which is the case the assertions
    /// are about: V73's doc tells a pristine seed from one somebody took
    /// over by `system = 1 AND updated_at = created_at`, and a
    /// corrective migration is written against that test. So an edit has
    /// to break the equality (or the migration overwrites somebody's
    /// work) without clearing the flag (or it stops being addressable as
    /// a seed at all). Both halves are asserted, and `created_at` is
    /// pinned to the literal the migration wrote so a write that moved
    /// *both* stamps — which preserves the equality and would satisfy a
    /// laxer assertion — fails here.
    ///
    /// Checked by mutation on 2026-08-10 by dropping `updated_at = ?7`
    /// from the `SET` list: *"an edited rule still reads as a pristine
    /// seed"*. Restored, it passes.
    #[tokio::test]
    async fn an_edit_moves_the_stamp_and_keeps_the_seed_addressable_as_one() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSeriesRepository::new(isle.clone());
        let seeded = StrategyId::from_uuid(Uuid::parse_str(VDSL_STRATEGY_ID).unwrap());

        let before = repo.find_strategy(&seeded).await.unwrap().expect("seeded");
        assert!(before.system);
        assert_eq!(before.created_at, before.updated_at);

        let mut edited = before.strategy.clone();
        edited.name = "VDSL recipe (mine)".to_string();
        edited.include = vec![
            Path::new(["vdsl", "script"]),
            Path::new(["vdsl", "version"]),
        ];
        let later = at() + chrono::Duration::seconds(90);
        repo.update_strategy(&edited, later).await.unwrap();

        let after = repo
            .find_strategy(&seeded)
            .await
            .unwrap()
            .expect("an edit does not remove the row");
        assert_eq!(after.strategy, edited, "every rule field was written");
        assert!(
            after.system,
            "a person editing a seeded rule does not make it stop being one — \
             the flag is provenance, not permission"
        );
        assert_eq!(
            after.created_at, before.created_at,
            "when the migration wrote it is not something a later edit changes"
        );
        assert_eq!(after.updated_at, later);
        assert_ne!(
            after.created_at, after.updated_at,
            "an edited rule must not read as a pristine seed"
        );

        // And an id nothing is registered under is a caller naming
        // nothing, not a silent success.
        assert!(matches!(
            repo.update_strategy(&strategy_named("stranger"), later)
                .await,
            Err(DomainError::NotFound { .. })
        ));
        assert!(matches!(
            repo.delete_strategy(&StrategyId::new()).await,
            Err(DomainError::NotFound { .. })
        ));

        driver.shutdown().await.unwrap();
    }

    /// Clearing one rule's answers takes that rule's rows, leaves every
    /// other rule's, and puts exactly the freed pairs back in the walk.
    ///
    /// The third assertion is the one that makes this invalidation
    /// rather than deletion: the walk's population is "a pair with no
    /// row", so a cleared pair is one the next pass answers — and a pair
    /// whose row was left behind is one no pass can ever re-offer. Two
    /// rules are in the fixture because with one, "this rule's rows" and
    /// "the table" are the same set and a `DELETE FROM material_series`
    /// with no `WHERE` would pass.
    ///
    /// Checked by mutation on 2026-08-11 by dropping the `WHERE` clause:
    /// this failed on the count first — *"one answer was filed under that
    /// rule: left `2`, right `1`"* — which is the same finding read off
    /// the return value rather than off the table, and the reason the
    /// count is returned at all. Restored, it passes.
    #[tokio::test]
    async fn clearing_one_rules_answers_puts_only_its_own_pairs_back_in_the_walk() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSeriesRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let vdsl_rule = StrategyId::from_uuid(Uuid::parse_str(VDSL_STRATEGY_ID).unwrap());
        let card_rule = register_second_rule(&repo).await;
        let asset = seed_material_carrying(
            &isle,
            persona,
            "image/png",
            Some(&vdsl_meta_kv("phase8_hires.lua")),
        )
        .await;

        repo.record(
            &asset,
            0,
            &vdsl_rule,
            &SeriesKey::Derived(format!("sk1-sha256:{}", "d".repeat(64))),
            at(),
        )
        .await
        .unwrap();
        repo.record(&asset, 0, &card_rule, &SeriesKey::NotApplicable, at())
            .await
            .unwrap();
        assert!(
            repo.scan_underived(None, 50).await.unwrap().is_empty(),
            "both pairs are answered, so the walk is empty to begin with"
        );

        let cleared = repo.clear_derived(&vdsl_rule).await.unwrap();
        assert_eq!(cleared, 1, "one answer was filed under that rule");

        let left: Vec<Uuid> = isle
            .call({
                let aid = *asset.as_uuid();
                move |conn| {
                    let mut stmt = conn
                        .prepare("SELECT strategy_id FROM material_series WHERE asset_id = ?1")?;
                    stmt.query_map(params![aid], |r| r.get(0))?
                        .collect::<Result<Vec<_>, _>>()
                }
            })
            .await
            .unwrap();
        assert_eq!(
            left,
            vec![*card_rule.as_uuid()],
            "the other rule's answer was cleared too"
        );

        // The rule itself survived — this is invalidation, not deletion.
        assert!(repo.find_strategy(&vdsl_rule).await.unwrap().is_some());

        let page = repo.scan_underived(None, 50).await.unwrap();
        assert_eq!(
            page.iter()
                .map(|pair| (pair.asset_id, pair.strategy.id))
                .collect::<Vec<_>>(),
            vec![(asset, vdsl_rule)],
            "the cleared pair is back in the walk and nothing else is: {page:#?}"
        );
        // Clearing a rule nothing was derived under is nought rather
        // than an error: an edit to a rule that never matched anything
        // has nothing to invalidate, and it is not a failure.
        assert_eq!(repo.clear_derived(&vdsl_rule).await.unwrap(), 0);

        driver.shutdown().await.unwrap();
    }

    /// The per-strategy `DELETE` — the cascade above, and the
    /// invalidation an edit will run — is served by V74's index rather
    /// than by a scan of the library times the rules.
    ///
    /// The sibling index cannot serve it: `idx_material_series_strategy_key`
    /// is partial on `key IS NOT NULL`, which `strategy_id = ?` does not
    /// imply, and the primary key leads with `asset_id`. So the assertion
    /// names the index rather than merely refusing a scan — with the
    /// partial one picked the delete would silently miss every row that is
    /// not a key, which is most of them.
    ///
    /// Checked by mutation on 2026-08-10: with V74's `CREATE INDEX`
    /// commented out, this failed — *"the per-strategy delete planned
    /// without its index: SCAN material_series"*. Restored, it passes.
    #[tokio::test]
    async fn the_per_strategy_delete_is_served_by_the_strategy_index() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();

        let plan: Vec<String> = isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "EXPLAIN QUERY PLAN DELETE FROM material_series WHERE strategy_id = ?1",
                )?;
                stmt.query_map(params![Uuid::now_v7()], |r| r.get::<_, String>(3))?
                    .collect::<Result<_, _>>()
            })
            .await
            .unwrap();
        let plan_text = plan.join("\n");

        assert!(
            plan_text.contains("idx_material_series_strategy (strategy_id=?)"),
            "the per-strategy delete planned without its index:\n{plan_text}"
        );
        assert!(
            !plan_text.contains("SCAN "),
            "something turned into a scan:\n{plan_text}"
        );

        driver.shutdown().await.unwrap();
    }

    /// The walk's own question of `material_series` — "is this pair
    /// answered" — is a point lookup on the primary key.
    ///
    /// `material_series` is `WITHOUT ROWID`, so the primary key is the
    /// table: there is no index to add here, and that is why V74 adds one
    /// for the delete and not for this. The plan is measured over the
    /// statement the adapter runs, from the same builder, so a rewrite
    /// that turned the subquery into a scan of the derived table — a
    /// `LEFT JOIN` on a partial key, an `IN (SELECT …)` — fails here.
    ///
    /// The second assertion is about the page rather than the lookup:
    /// the ordering `(m.asset_id, m.ord, s.id)` falls out of the two
    /// primary keys, so no page sorts. It would still be *correct* with a
    /// temp b-tree and it would sort the whole cross join on every page,
    /// which is the shape that makes a cursor pointless.
    #[tokio::test]
    async fn the_series_walk_asks_material_series_by_primary_key() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();

        for cursor in [false, true] {
            let plan: Vec<String> = isle
                .call(move |conn| {
                    let mut stmt = conn.prepare(&format!(
                        "EXPLAIN QUERY PLAN {}",
                        underived_page_sql(cursor)
                    ))?;
                    let read = |r: &rusqlite::Row<'_>| r.get::<_, String>(3);
                    if cursor {
                        stmt.query_map(params![50i64, Uuid::now_v7(), 0i64, Uuid::now_v7()], read)?
                            .collect::<Result<_, _>>()
                    } else {
                        stmt.query_map(params![50i64], read)?
                            .collect::<Result<_, _>>()
                    }
                })
                .await
                .unwrap();
            let plan_text = plan.join("\n");

            assert!(
                plan_text.contains(
                    "SEARCH ms USING PRIMARY KEY (asset_id=? AND ord=? AND strategy_id=?)"
                ),
                "the walk asked the derived table some other way (cursor={cursor}):\n{plan_text}"
            );
            assert!(
                !plan_text.contains("SCAN ms"),
                "the pair lookup turned into a scan of the derived table \
                 (cursor={cursor}):\n{plan_text}"
            );
            assert!(
                !plan_text.contains("TEMP B-TREE"),
                "the page sorted the whole cross join (cursor={cursor}):\n{plan_text}"
            );
        }

        driver.shutdown().await.unwrap();
    }
}
