//! SQLite adapters for `TagEvidenceRepository` and
//! `TagVectorRepository` (#112, P3).
//!
//! The evidence adapter's one load-bearing statement is the upsert in
//! [`suggest_under_head`]: the primary key `(asset, tag, model)` plus
//! the update's `WHERE disposition = 'suggested' AND head <> excluded`
//! is what keeps a rerun of the suggestion job from touching a
//! person's ruling — or a same-head rerun from moving a score — while
//! letting a *new* head replace an unruled suggestion (#132). The
//! guarantee is structural, not a handler branch.
//!
//! [`suggest_under_head`]: SqliteTagEvidenceRepository::suggest_under_head

use asterism_core::domain::repository::{TagEvidenceRepository, TagVectorRepository};
use asterism_core::domain::value::{AssetId, TagId};
use asterism_core::domain::visual::{
    ModelIdentity, TagEvidence, TagHeadRef, TagSuggestionDisposition,
};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::fault::StoreFault;
use crate::sqlite::map::infra_err;
use crate::sqlite::repo::visual::{blob_to_vector, vector_to_blob};

/// What ruling on a suggestion found, inside the one call that tried.
///
/// The update matching nothing has two causes and they are not the
/// same answer, so the query that tells them apart runs where it can
/// still see the transaction rather than being reconstructed after it.
enum Ruled {
    /// The suggestion was open and now is not.
    Done,
    /// It is here and somebody already ruled on it.
    Already,
    /// There is no such suggestion.
    Absent,
}

/// SQLite adapter for `TagEvidenceRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqliteTagEvidenceRepository {
    isle: AsyncIsle,
}

impl SqliteTagEvidenceRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

#[async_trait]
impl TagEvidenceRepository for SqliteTagEvidenceRepository {
    async fn suggest_under_head(
        &self,
        asset_id: &AssetId,
        tag_id: &TagId,
        model_id: &str,
        head: &TagHeadRef,
        score: f32,
        at_ms: i64,
    ) -> Result<bool, DomainError> {
        let asset = *asset_id.as_uuid();
        let tag = *tag_id.as_uuid();
        let model_id = model_id.to_string();
        let head = head.as_str().to_string();
        self.isle
            .call(move |conn| {
                // The update arm fires only on an unruled row from a
                // different head: a ruling never matches (disposition
                // moved), and a same-head rerun never matches (head
                // equal), so `changes()` is exactly "this pass said
                // something new".
                let n = conn.execute(
                    "INSERT INTO tag_evidence
                       (asset_id, tag_id, model_id, score, disposition, suggested_at, head)
                     VALUES (?1, ?2, ?3, ?4, 'suggested', ?5, ?6)
                     ON CONFLICT (asset_id, tag_id, model_id) DO UPDATE SET
                       score = excluded.score,
                       suggested_at = excluded.suggested_at,
                       head = excluded.head
                     WHERE tag_evidence.disposition = 'suggested'
                       AND tag_evidence.head <> excluded.head",
                    params![asset, tag, model_id, score as f64, at_ms, head],
                )?;
                Ok(n == 1)
            })
            .await
            .map_err(infra_err)
    }

    async fn retire_stale_suggestions(
        &self,
        asset_id: &AssetId,
        model_id: &str,
        keep: &TagHeadRef,
    ) -> Result<u64, DomainError> {
        let asset = *asset_id.as_uuid();
        let model_id = model_id.to_string();
        let keep = keep.as_str().to_string();
        self.isle
            .call(move |conn| {
                let n = conn.execute(
                    "DELETE FROM tag_evidence
                      WHERE asset_id = ?1 AND model_id = ?2
                        AND disposition = 'suggested' AND head <> ?3",
                    params![asset, model_id, keep],
                )?;
                Ok(n as u64)
            })
            .await
            .map_err(infra_err)
    }

    async fn of_asset(
        &self,
        asset_id: &AssetId,
        model_id: &str,
    ) -> Result<Vec<TagEvidence>, DomainError> {
        let asset = *asset_id.as_uuid();
        let model = model_id.to_string();
        type Row = (Uuid, f64, String, i64, Option<i64>, String);
        let rows: Vec<Row> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT tag_id, score, disposition, suggested_at, resolved_at, head
                       FROM tag_evidence
                      WHERE asset_id = ?1 AND model_id = ?2
                      ORDER BY score DESC",
                )?;
                let rows = stmt
                    .query_map(params![asset, model], |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(
                |(tag, score, disposition, suggested_at, resolved_at, head)| {
                    Ok(TagEvidence {
                        asset_id: *asset_id,
                        tag_id: TagId::from_uuid(tag),
                        model_id: model_id.to_string(),
                        // A blank head, like a disposition outside the
                        // three slugs below, is a row that could not have
                        // been written.
                        head: TagHeadRef::new(head).map_err(|_| {
                            StoreFault::CorruptRow(
                                "a stored tag_evidence row carries a blank head ref".to_string(),
                            )
                        })?,
                        score: score as f32,
                        // A stored disposition outside the three slugs is a
                        // row that could not have been written, not a thing
                        // the caller asked for.
                        disposition: TagSuggestionDisposition::parse(&disposition).map_err(
                            |_| {
                                StoreFault::CorruptRow(format!(
                                    "a stored tag_evidence row names a disposition this model \
                             does not have: {disposition:?}"
                                ))
                            },
                        )?,
                        suggested_at_ms: suggested_at,
                        resolved_at_ms: resolved_at,
                    })
                },
            )
            .collect()
    }

    async fn resolve(
        &self,
        asset_id: &AssetId,
        tag_id: &TagId,
        model_id: &str,
        disposition: TagSuggestionDisposition,
        at_ms: i64,
    ) -> Result<(), DomainError> {
        if disposition == TagSuggestionDisposition::Suggested {
            return Err(DomainError::Validation(
                "a ruling must be accepted or rejected".into(),
            ));
        }
        let asset = *asset_id.as_uuid();
        let tag = *tag_id.as_uuid();
        let named = model_id.to_string();
        let model_id = model_id.to_string();
        let slug = disposition.as_str();
        // The update alone cannot say why it matched nothing, and the
        // two reasons want opposite answers: a suggestion nobody made
        // is a `404`, one already ruled on is a `409` that will never
        // become anything else. Asking inside the same call rather
        // than reporting "absent or already ruled" and leaving a
        // caller to guess which — the extra read costs one statement
        // and only on the path that is already failing.
        let ruled = self
            .isle
            .call(move |conn| {
                let n = conn.execute(
                    "UPDATE tag_evidence
                        SET disposition = ?1, resolved_at = ?2
                      WHERE asset_id = ?3 AND tag_id = ?4 AND model_id = ?5
                        AND disposition = 'suggested'",
                    params![slug, at_ms, asset, tag, model_id],
                )?;
                if n > 0 {
                    return Ok(Ruled::Done);
                }
                let held: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM tag_evidence
                      WHERE asset_id = ?1 AND tag_id = ?2 AND model_id = ?3",
                    params![asset, tag, model_id],
                    |row| row.get(0),
                )?;
                Ok(if held > 0 {
                    Ruled::Already
                } else {
                    Ruled::Absent
                })
            })
            .await
            .map_err(infra_err)?;

        match ruled {
            Ruled::Done => Ok(()),
            Ruled::Already => Err(StoreFault::AlreadyDecided(
                "that suggestion has already been ruled on".into(),
            )
            .into()),
            Ruled::Absent => {
                Err(StoreFault::absent("tag suggestion", format!("{asset}/{tag}/{named}")).into())
            }
        }
    }

    async fn clear_derived(&self, model_id: &str) -> Result<u64, DomainError> {
        let model_id = model_id.to_string();
        self.isle
            .call(move |conn| {
                let n = conn.execute(
                    "DELETE FROM tag_evidence WHERE model_id = ?1",
                    params![model_id],
                )?;
                Ok(n as u64)
            })
            .await
            .map_err(infra_err)
    }
}

/// SQLite adapter for `TagVectorRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqliteTagVectorRepository {
    isle: AsyncIsle,
}

impl SqliteTagVectorRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

#[async_trait]
impl TagVectorRepository for SqliteTagVectorRepository {
    async fn vectors(
        &self,
        identity: &ModelIdentity,
    ) -> Result<Vec<(TagId, Vec<f32>)>, DomainError> {
        let ident = identity.clone();
        let rows: Vec<(Uuid, Vec<u8>)> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT tag_id, vector FROM tag_vector
                      WHERE model_id = ?1 AND preprocess_ver = ?2",
                )?;
                let rows = stmt
                    .query_map(params![ident.model_id, ident.preprocess_ver], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(tag, blob)| Ok((TagId::from_uuid(tag), blob_to_vector(&blob)?)))
            .collect()
    }

    async fn set_tag_vector(
        &self,
        tag_id: &TagId,
        identity: &ModelIdentity,
        vector: &[f32],
        at_ms: i64,
    ) -> Result<(), DomainError> {
        let tag = *tag_id.as_uuid();
        let ident = identity.clone();
        let blob = vector_to_blob(vector);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO tag_vector
                       (tag_id, model_id, preprocess_ver, dim, vector, encoded_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        tag,
                        ident.model_id,
                        ident.preprocess_ver,
                        ident.dim,
                        blob,
                        at_ms
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn clear_derived(&self, model_id: &str) -> Result<u64, DomainError> {
        let model_id = model_id.to_string();
        self.isle
            .call(move |conn| {
                let n = conn.execute(
                    "DELETE FROM tag_vector WHERE model_id = ?1",
                    params![model_id],
                )?;
                Ok(n as u64)
            })
            .await
            .map_err(infra_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_core::error::ConflictKind;

    /// The day-one head, spelled short: every pre-#132 test scores
    /// under it.
    fn zs() -> TagHeadRef {
        TagHeadRef::zero_shot()
    }

    async fn seed_asset_and_tags(isle: &AsyncIsle) -> (AssetId, TagId, TagId) {
        let persona = Uuid::now_v7();
        let asset = Uuid::now_v7();
        let tag_a = Uuid::now_v7();
        let tag_b = Uuid::now_v7();
        isle.call(move |conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p', 'P', 0, 0)",
                params![persona],
            )?;
            tx.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', '{\"kind\":\"file\",\"path\":\"/pics/a.png\"}',
                         'image', 0, 0, 0)",
                params![asset, persona],
            )?;
            tx.execute(
                "INSERT INTO tag (id, name) VALUES (?1, 'red circle')",
                params![tag_a],
            )?;
            tx.execute(
                "INSERT INTO tag (id, name) VALUES (?1, 'blue square')",
                params![tag_b],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
        (
            AssetId::from_uuid(asset),
            TagId::from_uuid(tag_a),
            TagId::from_uuid(tag_b),
        )
    }

    #[tokio::test]
    async fn a_rerun_cannot_touch_a_ruling_or_an_earlier_score() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteTagEvidenceRepository::new(isle.clone());
        let (asset, tag_a, tag_b) = seed_asset_and_tags(&isle).await;

        assert!(
            repo.suggest_under_head(&asset, &tag_a, "m", &zs(), 0.31, 1)
                .await
                .unwrap()
        );
        assert!(
            repo.suggest_under_head(&asset, &tag_b, "m", &zs(), 0.22, 1)
                .await
                .unwrap()
        );
        // The person rules one row each way.
        repo.resolve(&asset, &tag_a, "m", TagSuggestionDisposition::Accepted, 2)
            .await
            .unwrap();
        repo.resolve(&asset, &tag_b, "m", TagSuggestionDisposition::Rejected, 2)
            .await
            .unwrap();

        // A rerun proposes again, with different scores: nothing lands.
        assert!(
            !repo
                .suggest_under_head(&asset, &tag_a, "m", &zs(), 0.99, 3)
                .await
                .unwrap()
        );
        assert!(
            !repo
                .suggest_under_head(&asset, &tag_b, "m", &zs(), 0.99, 3)
                .await
                .unwrap()
        );

        let rows = repo.of_asset(&asset, "m").await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].disposition, TagSuggestionDisposition::Accepted);
        assert!(
            (rows[0].score - 0.31).abs() < 1e-6,
            "the ruled score survives the rerun"
        );
        assert_eq!(rows[1].disposition, TagSuggestionDisposition::Rejected);

        // Re-ruling a ruled row is a conflict, not an overwrite.
        let err = repo
            .resolve(&asset, &tag_a, "m", TagSuggestionDisposition::Rejected, 4)
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                DomainError::Conflict {
                    kind: ConflictKind::Settled,
                    ..
                }
            ),
            "{err}"
        );

        // A different model has its own namespace.
        assert!(
            repo.suggest_under_head(&asset, &tag_b, "m2", &zs(), 0.5, 5)
                .await
                .unwrap()
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn clear_derived_scopes_to_one_model_and_tag_vectors_round_trip() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let evidence = SqliteTagEvidenceRepository::new(isle.clone());
        let vectors = SqliteTagVectorRepository::new(isle.clone());
        let (asset, tag_a, _tag_b) = seed_asset_and_tags(&isle).await;

        let identity = ModelIdentity {
            model_id: "m".into(),
            dim: 3,
            preprocess_ver: 1,
        };
        evidence
            .suggest_under_head(&asset, &tag_a, "m", &zs(), 0.4, 1)
            .await
            .unwrap();
        evidence
            .suggest_under_head(&asset, &tag_a, "m2", &zs(), 0.4, 1)
            .await
            .unwrap();
        vectors
            .set_tag_vector(&tag_a, &identity, &[0.0, 1.0, 0.0], 1)
            .await
            .unwrap();

        let cached = vectors.vectors(&identity).await.unwrap();
        assert_eq!(cached, vec![(tag_a, vec![0.0, 1.0, 0.0])]);

        assert_eq!(evidence.clear_derived("m").await.unwrap(), 1);
        assert_eq!(vectors.clear_derived("m").await.unwrap(), 1);
        assert_eq!(evidence.of_asset(&asset, "m").await.unwrap().len(), 0);
        assert_eq!(evidence.of_asset(&asset, "m2").await.unwrap().len(), 1);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_new_head_rescores_unruled_rows_and_never_a_ruling() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteTagEvidenceRepository::new(isle.clone());
        let (asset, tag_a, tag_b) = seed_asset_and_tags(&isle).await;
        let trained = TagHeadRef::new("head-v1").unwrap();

        // Zero-shot proposes both; the person rules one.
        repo.suggest_under_head(&asset, &tag_a, "m", &zs(), 0.31, 1)
            .await
            .unwrap();
        repo.suggest_under_head(&asset, &tag_b, "m", &zs(), 0.22, 1)
            .await
            .unwrap();
        repo.resolve(&asset, &tag_a, "m", TagSuggestionDisposition::Accepted, 2)
            .await
            .unwrap();

        // The trained head re-proposes both: the ruling stands
        // untouched, the unruled row takes the new head's score.
        assert!(
            !repo
                .suggest_under_head(&asset, &tag_a, "m", &trained, 0.9, 3)
                .await
                .unwrap(),
            "a ruling is out of every head's reach"
        );
        assert!(
            repo.suggest_under_head(&asset, &tag_b, "m", &trained, 0.8, 3)
                .await
                .unwrap()
        );
        let rows = repo.of_asset(&asset, "m").await.unwrap();
        let b = rows.iter().find(|r| r.tag_id == tag_b).unwrap();
        assert_eq!(b.head, trained);
        assert!((b.score - 0.8).abs() < 1e-6, "the new head's score wins");
        let a = rows.iter().find(|r| r.tag_id == tag_a).unwrap();
        assert_eq!(a.head, zs(), "the ruled row keeps the head that ruled");

        // A rerun under the same head moves nothing — first score
        // stands, the pre-#132 guarantee, now per head.
        assert!(
            !repo
                .suggest_under_head(&asset, &tag_b, "m", &trained, 0.99, 4)
                .await
                .unwrap()
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn retiring_stale_suggestions_spares_rulings_and_the_current_head() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteTagEvidenceRepository::new(isle.clone());
        let (asset, tag_a, tag_b) = seed_asset_and_tags(&isle).await;
        let trained = TagHeadRef::new("head-v1").unwrap();

        // Zero-shot proposed both; the person ruled one; the trained
        // head's pass re-affirmed neither (both fell below its floor).
        repo.suggest_under_head(&asset, &tag_a, "m", &zs(), 0.31, 1)
            .await
            .unwrap();
        repo.suggest_under_head(&asset, &tag_b, "m", &zs(), 0.22, 1)
            .await
            .unwrap();
        repo.resolve(&asset, &tag_a, "m", TagSuggestionDisposition::Rejected, 2)
            .await
            .unwrap();

        // The stale zero-shot suggestion leaves; the ruling stays.
        assert_eq!(
            repo.retire_stale_suggestions(&asset, "m", &trained)
                .await
                .unwrap(),
            1
        );
        let rows = repo.of_asset(&asset, "m").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tag_id, tag_a);
        assert_eq!(rows[0].disposition, TagSuggestionDisposition::Rejected);

        // Idempotent once nothing stale remains.
        assert_eq!(
            repo.retire_stale_suggestions(&asset, "m", &trained)
                .await
                .unwrap(),
            0
        );

        driver.shutdown().await.unwrap();
    }
}
