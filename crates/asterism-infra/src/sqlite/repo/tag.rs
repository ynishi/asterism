//! SQLite adapter for the `TagRepository` port (backed by rusqlite-isle).

use asterism_core::domain::repository::TagRepository;
use asterism_core::domain::tag::{Tag, TagCount, TagMergeOutcome};
use asterism_core::domain::value::{AssetId, ChannelAxis, PersonaId, TagId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::infra_err;

/// Primitive row built inside the isle closure.
struct TagRow {
    id: Uuid,
    name: String,
    axis: Option<String>,
}

impl TagRow {
    const COLUMNS: &'static str = "id, name, axis";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            axis: row.get(2)?,
        })
    }

    fn into_domain(self) -> Result<Tag, DomainError> {
        Ok(Tag {
            id: TagId::from_uuid(self.id),
            name: self.name,
            axis: self.axis.as_deref().map(ChannelAxis::parse).transpose()?,
        })
    }
}

/// Why a tag write refused, decided *inside* the transaction so the
/// check and the write cannot be separated by a concurrent writer.
///
/// Carried out of the isle closure as an `Ok(Err(..))` because the
/// closure's own error type is `rusqlite::Error`: these are refusals,
/// not storage failures, and each maps to a different status at the
/// transport boundary.
///
/// The messages state the storage fact only. Telling the caller what
/// to do instead is the application layer's business — it is the one
/// that knows which command was invoked.
enum Refusal {
    /// The addressed tag id is not in the table.
    Missing(Uuid),
    /// Another tag already carries the requested name.
    NameTaken(String),
}

impl Refusal {
    fn into_domain(self) -> DomainError {
        match self {
            Self::Missing(id) => DomainError::not_found("tag", TagId::from_uuid(id)),
            Self::NameTaken(name) => {
                DomainError::clashes(format!("another tag is already named {name:?}"))
            }
        }
    }
}

/// SQLite adapter for `TagRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqliteTagRepository {
    isle: AsyncIsle,
}

impl SqliteTagRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

#[async_trait]
impl TagRepository for SqliteTagRepository {
    async fn find_or_create(&self, name: &str) -> Result<Tag, DomainError> {
        // Reject an empty name and mint a fresh id.
        let tag = Tag::new(name)?;
        let new_id = *tag.id.as_uuid();
        let tag_name = tag.name.clone();
        let row = self
            .isle
            .call(move |conn| {
                // Idempotent get-or-create: INSERT OR IGNORE followed by
                // a SELECT relies on the UNIQUE(name) constraint to keep
                // the existing row.
                conn.execute(
                    "INSERT OR IGNORE INTO tag (id, name, axis) VALUES (?1, ?2, NULL)",
                    params![new_id, tag_name],
                )?;
                conn.query_row(
                    &format!("SELECT {} FROM tag WHERE name = ?1", TagRow::COLUMNS),
                    params![tag_name],
                    TagRow::from_row,
                )
            })
            .await
            .map_err(infra_err)?;
        row.into_domain()
    }

    async fn list(&self) -> Result<Vec<Tag>, DomainError> {
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM tag ORDER BY name",
                    TagRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map([], TagRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(TagRow::into_domain).collect()
    }

    async fn tag_counts(
        &self,
        persona_id: Option<&PersonaId>,
    ) -> Result<Vec<TagCount>, DomainError> {
        // Persona-scoped: JOIN asset so the count reflects the
        // active persona filter. `INNER JOIN` (via `asset_tag` on
        // both sides) drops tags with zero assets, matching the
        // contract ("sidebar does not list dead channels").
        //
        // Both branches exclude trashed assets. `asset_tag` rows
        // survive trashing by design (V30 keeps the asset row so
        // restore is free), so counting the join table alone would
        // advertise a channel count the grid cannot match — and a tag
        // whose every asset is trashed would keep listing, breaking the
        // "no dead channels" contract. The unscoped branch therefore
        // joins `asset` purely to reach `trashed_at`.
        //
        // Headstones (V49) drop out on the same argument: a folded
        // row's `asset_tag` links also outlive the fold, so counting
        // them would re-inflate a channel by exactly the duplicates
        // somebody just resolved. The keeper carries the tags forward
        // (the fold verb unions them), so nothing is lost
        // by not counting the row it replaced.
        let persona_uuid = persona_id.map(|p| *p.as_uuid());
        let rows: Vec<(TagRow, i64)> = self
            .isle
            .call(move |conn| {
                let (sql, mut params_vec): (String, Vec<rusqlite::types::Value>) =
                    if persona_uuid.is_some() {
                        (
                            "SELECT tag.id, tag.name, tag.axis, \
                                    COUNT(DISTINCT asset.id) AS c \
                             FROM tag \
                             JOIN asset_tag ON asset_tag.tag_id = tag.id \
                             JOIN asset ON asset.id = asset_tag.asset_id \
                             WHERE asset.persona_id = ?1 \
                               AND asset.trashed_at IS NULL \
                               AND asset.folded_into IS NULL \
                             GROUP BY tag.id, tag.name, tag.axis \
                             HAVING c > 0 \
                             ORDER BY c DESC, tag.name ASC"
                                .to_string(),
                            Vec::new(),
                        )
                    } else {
                        (
                            "SELECT tag.id, tag.name, tag.axis, \
                                    COUNT(DISTINCT asset.id) AS c \
                             FROM tag \
                             JOIN asset_tag ON asset_tag.tag_id = tag.id \
                             JOIN asset ON asset.id = asset_tag.asset_id \
                             WHERE asset.trashed_at IS NULL \
                               AND asset.folded_into IS NULL \
                             GROUP BY tag.id, tag.name, tag.axis \
                             HAVING c > 0 \
                             ORDER BY c DESC, tag.name ASC"
                                .to_string(),
                            Vec::new(),
                        )
                    };
                if let Some(uuid) = persona_uuid {
                    params_vec.push(rusqlite::types::Value::Blob(uuid.as_bytes().to_vec()));
                }
                let mut stmt = conn.prepare(&sql)?;
                let iter = stmt
                    .query_map(rusqlite::params_from_iter(params_vec.iter()), |row| {
                        Ok((TagRow::from_row(row)?, row.get::<_, i64>(3)?))
                    })?;
                iter.collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(infra_err)?;

        rows.into_iter()
            .map(|(row, count)| {
                Ok(TagCount {
                    tag: row.into_domain()?,
                    asset_count: count.max(0) as u64,
                })
            })
            .collect()
    }

    async fn tags_of(&self, asset_id: &AssetId) -> Result<Vec<Tag>, DomainError> {
        let uuid = *asset_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT tag.id, tag.name, tag.axis FROM tag
                     JOIN asset_tag ON asset_tag.tag_id = tag.id
                     WHERE asset_tag.asset_id = ?1
                     ORDER BY tag.name",
                )?;
                let rows = stmt
                    .query_map(params![uuid], TagRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(TagRow::into_domain).collect()
    }

    async fn rename(&self, id: &TagId, name: &str) -> Result<Tag, DomainError> {
        let uuid = *id.as_uuid();
        let new_name = name.to_string();
        let row = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let present: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM tag WHERE id = ?1",
                    params![uuid],
                    |r| r.get(0),
                )?;
                if present == 0 {
                    return Ok(Err(Refusal::Missing(uuid)));
                }
                // `id <> ?2` is what makes renaming a tag to the name
                // it already carries a no-op success rather than a
                // self-inflicted conflict. Any *other* holder of the
                // name is refused: the UNIQUE constraint would refuse
                // it anyway, but as a 500-shaped storage error rather
                // than the 409 that tells the caller merge exists.
                let taken: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM tag WHERE name = ?1 AND id <> ?2",
                    params![new_name, uuid],
                    |r| r.get(0),
                )?;
                if taken > 0 {
                    return Ok(Err(Refusal::NameTaken(new_name)));
                }
                tx.execute(
                    "UPDATE tag SET name = ?1 WHERE id = ?2",
                    params![new_name, uuid],
                )?;
                // The name is the input of the cached tag embedding
                // (#112): a renamed tag's vector describes a word that
                // no longer exists, so it leaves in the same
                // transaction and the suggestion job re-encodes lazily.
                tx.execute("DELETE FROM tag_vector WHERE tag_id = ?1", params![uuid])?;
                let row = tx.query_row(
                    &format!("SELECT {} FROM tag WHERE id = ?1", TagRow::COLUMNS),
                    params![uuid],
                    TagRow::from_row,
                )?;
                tx.commit()?;
                Ok(Ok(row))
            })
            .await
            .map_err(infra_err)?
            .map_err(Refusal::into_domain)?;
        row.into_domain()
    }

    async fn delete(&self, id: &TagId) -> Result<u64, DomainError> {
        let uuid = *id.as_uuid();
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let present: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM tag WHERE id = ?1",
                    params![uuid],
                    |r| r.get(0),
                )?;
                if present == 0 {
                    return Ok(Err(Refusal::Missing(uuid)));
                }
                // Counted before the delete: `ON DELETE CASCADE` takes
                // the links with the row, so afterwards there is
                // nothing left to count.
                let links: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM asset_tag WHERE tag_id = ?1",
                    params![uuid],
                    |r| r.get(0),
                )?;
                tx.execute("DELETE FROM tag WHERE id = ?1", params![uuid])?;
                tx.commit()?;
                Ok(Ok(links.max(0) as u64))
            })
            .await
            .map_err(infra_err)?
            .map_err(Refusal::into_domain)
    }

    async fn merge(
        &self,
        source: &TagId,
        target: &TagId,
        dry_run: bool,
    ) -> Result<TagMergeOutcome, DomainError> {
        let source_id = *source.as_uuid();
        let target_id = *target.as_uuid();
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                for id in [source_id, target_id] {
                    let present: i64 =
                        tx.query_row("SELECT COUNT(*) FROM tag WHERE id = ?1", params![id], |r| {
                            r.get(0)
                        })?;
                    if present == 0 {
                        return Ok(Err(Refusal::Missing(id)));
                    }
                }
                // The source's whole link set, which the two reported
                // counts partition.
                let total: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM asset_tag WHERE tag_id = ?1",
                    params![source_id],
                    |r| r.get(0),
                )?;

                if dry_run {
                    // Predict rather than write, then let the
                    // transaction roll back on drop.
                    let already: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM asset_tag s \
                         WHERE s.tag_id = ?1 \
                           AND EXISTS (SELECT 1 FROM asset_tag t \
                                       WHERE t.tag_id = ?2 \
                                         AND t.asset_id = s.asset_id)",
                        params![source_id, target_id],
                        |r| r.get(0),
                    )?;
                    return Ok(Ok(TagMergeOutcome {
                        affected_assets: (total - already).max(0) as u64,
                        already_tagged: already.max(0) as u64,
                        source_removed: false,
                    }));
                }

                // `OR IGNORE` against `PRIMARY KEY (asset_id, tag_id)`
                // is the de-duplication: an asset already carrying the
                // target keeps its single link, and the statement's
                // change count is therefore exactly the number of
                // assets that *moved*. Without it the merge would
                // abort on the first overlapping asset.
                //
                // Reading `asset_tag` while inserting into it is safe
                // here because the two row sets are disjoint by
                // predicate: the SELECT takes `tag_id = source` and
                // every inserted row carries `tag_id = target`
                // (`source != target` is enforced upstream).
                let moved = tx.execute(
                    "INSERT OR IGNORE INTO asset_tag (asset_id, tag_id) \
                     SELECT asset_id, ?1 FROM asset_tag WHERE tag_id = ?2",
                    params![target_id, source_id],
                )? as i64;
                // Dissolve the source; the FK cascade drops whatever
                // links it still held (the moved ones now exist under
                // the target, the overlapping ones were duplicates).
                tx.execute("DELETE FROM tag WHERE id = ?1", params![source_id])?;
                tx.commit()?;
                Ok(Ok(TagMergeOutcome {
                    affected_assets: moved.max(0) as u64,
                    already_tagged: (total - moved).max(0) as u64,
                    source_removed: true,
                }))
            })
            .await
            .map_err(infra_err)?
            .map_err(Refusal::into_domain)
    }

    async fn personas_with_tag(&self, tag: &TagId) -> Result<Vec<PersonaId>, DomainError> {
        let uuid = *tag.as_uuid();
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT asset.persona_id FROM asset_tag \
                     JOIN asset ON asset.id = asset_tag.asset_id \
                     WHERE asset_tag.tag_id = ?1",
                )?;
                stmt.query_map(params![uuid], |row| row.get(0))?
                    .collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(PersonaId::from_uuid).collect())
    }

    async fn link(&self, asset_id: &AssetId, tag_id: &TagId) -> Result<(), DomainError> {
        let asset = *asset_id.as_uuid();
        let tag = *tag_id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO asset_tag (asset_id, tag_id) VALUES (?1, ?2)",
                    params![asset, tag],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn unlink(&self, asset_id: &AssetId, tag_id: &TagId) -> Result<(), DomainError> {
        let asset = *asset_id.as_uuid();
        let tag = *tag_id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM asset_tag WHERE asset_id = ?1 AND tag_id = ?2",
                    params![asset, tag],
                )?;
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

    /// The channel chips count assets, and `asset_tag` links outlive
    /// both a trash stamp and a fold. Counting them raw is how a
    /// sidebar ends up advertising a number the grid cannot produce —
    /// and after a de-duplication pass, the gap is exactly the set of
    /// duplicates somebody just resolved.
    #[tokio::test]
    async fn a_folded_asset_stops_counting_towards_its_channel() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteTagRepository::new(isle.clone());

        let persona = Uuid::now_v7();
        let tag = Uuid::now_v7();
        let keeper = Uuid::now_v7();
        let headstone = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, display_order, archived, created_at, updated_at) \
                 VALUES (?1, 'P', 0, 0, 0, 0)",
                params![persona],
            )?;
            conn.execute(
                "INSERT INTO tag (id, name, axis) VALUES (?1, 'keepers', NULL)",
                params![tag],
            )?;
            for id in [keeper, headstone] {
                conn.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                        modality, occurred_at, created_at, updated_at) \
                     VALUES (?1, ?2, 'fs', ?3, 'tape', 0, 0, 0)",
                    params![id, persona, format!("a-{id}.md")],
                )?;
                conn.execute(
                    "INSERT INTO asset_tag (asset_id, tag_id) VALUES (?1, ?2)",
                    params![id, tag],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();

        let persona_id = PersonaId::from_uuid(persona);
        // Both scopes: the persona-scoped branch and the global one are
        // separate statements, so a filter added to one and not the
        // other would show up here.
        for scope in [Some(&persona_id), None] {
            let counts = repo.tag_counts(scope).await.unwrap();
            assert_eq!(counts.len(), 1);
            assert_eq!(
                counts[0].asset_count,
                2,
                "two tagged assets before the fold (persona scope: {})",
                scope.is_some()
            );
        }

        isle.call(move |conn| {
            let marked = conn.execute(
                "UPDATE asset SET folded_into = ?2 WHERE id = ?1",
                params![headstone, keeper],
            )?;
            assert_eq!(marked, 1, "the fixture must actually stand a headstone");
            Ok(())
        })
        .await
        .unwrap();

        for scope in [Some(&persona_id), None] {
            let counts = repo.tag_counts(scope).await.unwrap();
            assert_eq!(counts.len(), 1, "the channel is still alive");
            assert_eq!(
                counts[0].asset_count,
                1,
                "the link outlives the fold; the count must not (persona scope: {})",
                scope.is_some()
            );
        }

        driver.shutdown().await.unwrap();
    }
}
