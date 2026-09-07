//! SQLite adapter for the `VisualFeatureRepository` port (#112).
//!
//! Vectors are stored as little-endian `f32` blobs beside their full
//! derivation identity; a row exists only once extraction has answered
//! (`computed` with a vector, `failed` with a reason), so the walk's
//! `NOT EXISTS` predicate offers each image material exactly once per
//! model. `preprocess_ver` sits outside the primary key on purpose: a
//! recipe bump *overwrites* the same model's row rather than growing a
//! second generation, and reads filter on it so a stale-recipe vector
//! is never served as current.

use asterism_core::domain::repository::{VisualFeatureRepository, VisualScanCandidate};
use asterism_core::domain::source_locator::SourceLocator;
use asterism_core::domain::value::{AssetId, MimeType, PersonaId};
use asterism_core::domain::visual::{ModelIdentity, TagHeadRef, VisualFeature, VisualFeatureKind};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::infra_err;

/// SQLite adapter for `VisualFeatureRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqliteVisualFeatureRepository {
    isle: AsyncIsle,
}

impl SqliteVisualFeatureRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

pub(crate) fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(vector.len() * 4);
    for v in vector {
        blob.extend_from_slice(&v.to_le_bytes());
    }
    blob
}

pub(crate) fn blob_to_vector(blob: &[u8]) -> Result<Vec<f32>, DomainError> {
    let (chunks, remainder) = blob.as_chunks::<4>();
    if !remainder.is_empty() {
        return Err(DomainError::Validation(format!(
            "stored vector blob length {} is not a multiple of 4",
            blob.len()
        )));
    }
    Ok(chunks.iter().map(|c| f32::from_le_bytes(*c)).collect())
}

#[async_trait]
impl VisualFeatureRepository for SqliteVisualFeatureRepository {
    async fn set_visual_feature(&self, feature: VisualFeature) -> Result<(), DomainError> {
        let asset = *feature.asset_id.as_uuid();
        let blob = vector_to_blob(&feature.vector);
        let identity = feature.identity.clone();
        let kind = feature.kind.as_str();
        let (ord, extracted_at) = (feature.ord, feature.extracted_at_ms);
        let composition = feature.composition_ver;
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO visual_feature
                       (asset_id, ord, model_id, feature_kind, preprocess_ver,
                        dim, vector, status, reason, extracted_at, composition_ver)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'computed', NULL, ?8, ?9)",
                    params![
                        asset,
                        ord,
                        identity.model_id,
                        kind,
                        identity.preprocess_ver,
                        identity.dim,
                        blob,
                        extracted_at,
                        composition,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn mark_unextractable(
        &self,
        asset_id: &AssetId,
        ord: u32,
        identity: &ModelIdentity,
        kind: VisualFeatureKind,
        reason: &str,
        composition: i64,
    ) -> Result<(), DomainError> {
        let asset = *asset_id.as_uuid();
        let identity = identity.clone();
        let kind = kind.as_str();
        let reason = reason.to_string();
        let now = chrono::Utc::now().timestamp_millis();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO visual_feature
                       (asset_id, ord, model_id, feature_kind, preprocess_ver,
                        dim, vector, status, reason, extracted_at, composition_ver)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, 'failed', ?6, ?7, ?8)",
                    params![
                        asset,
                        ord,
                        identity.model_id,
                        kind,
                        identity.preprocess_ver,
                        reason,
                        now,
                        composition,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn feature_of(
        &self,
        asset_id: &AssetId,
        ord: u32,
        identity: &ModelIdentity,
        kind: VisualFeatureKind,
    ) -> Result<Option<VisualFeature>, DomainError> {
        let asset = *asset_id.as_uuid();
        let ident = identity.clone();
        let kind_slug = kind.as_str();
        let row: Option<(Vec<u8>, i64, i64)> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT vector, extracted_at, composition_ver FROM visual_feature
                      WHERE asset_id = ?1 AND ord = ?2 AND model_id = ?3
                        AND feature_kind = ?4 AND preprocess_ver = ?5
                        AND status = 'computed'",
                )?;
                let mut rows = stmt.query_map(
                    params![asset, ord, ident.model_id, kind_slug, ident.preprocess_ver],
                    |r| {
                        Ok((
                            r.get::<_, Vec<u8>>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, i64>(2)?,
                        ))
                    },
                )?;
                rows.next().transpose()
            })
            .await
            .map_err(infra_err)?;
        match row {
            None => Ok(None),
            Some((blob, extracted_at, composition_ver)) => {
                let vector = blob_to_vector(&blob)?;
                Ok(Some(VisualFeature::new(
                    *asset_id,
                    ord,
                    identity.clone(),
                    kind,
                    vector,
                    extracted_at,
                    composition_ver,
                )?))
            }
        }
    }

    async fn vectors_in_scope(
        &self,
        scope: Option<&PersonaId>,
        identity: &ModelIdentity,
        kind: VisualFeatureKind,
    ) -> Result<Vec<(AssetId, PersonaId, Vec<f32>)>, DomainError> {
        let persona = scope.map(|p| *p.as_uuid());
        let ident = identity.clone();
        let kind_slug = kind.as_str();
        let rows: Vec<(Uuid, Uuid, Vec<u8>)> = self
            .isle
            .call(move |conn| {
                // Trashed and folded assets are out: a suggestion must
                // not point at a card the grid will not show.
                //
                // The scope predicate is written as a NULL test rather
                // than as two prepared statements: one shape means one
                // place where the visibility rules above are stated,
                // and a library-wide scan that forgot one of them would
                // differ from the scoped scan in what it is willing to
                // propose.
                let mut stmt = conn.prepare(
                    "SELECT vf.asset_id, a.persona_id, vf.vector
                       FROM visual_feature vf
                       JOIN asset a ON a.id = vf.asset_id
                      WHERE (?1 IS NULL OR a.persona_id = ?1)
                        AND vf.model_id = ?2 AND vf.feature_kind = ?3
                        AND vf.preprocess_ver = ?4 AND vf.status = 'computed'
                        AND a.trashed_at IS NULL AND a.folded_into IS NULL",
                )?;
                let rows = stmt
                    .query_map(
                        params![persona, ident.model_id, kind_slug, ident.preprocess_ver],
                        |r| {
                            Ok((
                                r.get::<_, Uuid>(0)?,
                                r.get::<_, Uuid>(1)?,
                                r.get::<_, Vec<u8>>(2)?,
                            ))
                        },
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(id, persona, blob)| {
                Ok((
                    AssetId::from_uuid(id),
                    PersonaId::from_uuid(persona),
                    blob_to_vector(&blob)?,
                ))
            })
            .collect()
    }

    async fn unextracted(
        &self,
        identity: &ModelIdentity,
        kind: VisualFeatureKind,
        limit: u32,
    ) -> Result<Vec<VisualScanCandidate>, DomainError> {
        let ident = identity.clone();
        let kind_slug = kind.as_str();
        let rows: Vec<(Uuid, u32, String, Option<String>)> = self
            .isle
            .call(move |conn| {
                // Absence is the pending state: a `computed` or
                // `failed` row retires the material from this walk, so
                // every row is offered exactly once per model. The
                // mime filter keeps non-images from ever entering —
                // they would only ever earn failure rows.
                let mut stmt = conn.prepare(
                    "SELECT m.asset_id, m.ord, m.locator, m.mime
                       FROM material m
                       JOIN asset a ON a.id = m.asset_id
                      WHERE m.ord = 0
                        AND m.mime LIKE 'image/%'
                        AND a.trashed_at IS NULL AND a.folded_into IS NULL
                        AND NOT EXISTS (
                            SELECT 1 FROM visual_feature vf
                             WHERE vf.asset_id = m.asset_id AND vf.ord = m.ord
                               AND vf.model_id = ?1 AND vf.feature_kind = ?2)
                      ORDER BY m.asset_id
                      LIMIT ?3",
                )?;
                let rows = stmt
                    .query_map(params![ident.model_id, kind_slug, limit as i64], |r| {
                        Ok((
                            r.get::<_, Uuid>(0)?,
                            r.get::<_, u32>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<String>>(3)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(id, ord, locator, mime)| {
                Ok(VisualScanCandidate {
                    asset_id: AssetId::from_uuid(id),
                    ord,
                    // Same boundary parse as the fingerprint walk, so
                    // the batch and per-asset passes cannot hand the
                    // extractor two readings of one artefact.
                    locator: SourceLocator::try_from(locator.as_str())?,
                    mime: mime.as_deref().map(MimeType::parse),
                })
            })
            .collect()
    }

    async fn unworded(
        &self,
        identity: &ModelIdentity,
        composition: i64,
        limit: u32,
    ) -> Result<Vec<AssetId>, DomainError> {
        let ident = identity.clone();
        let kind = VisualFeatureKind::Words.as_str();
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                // Over `asset` rather than `material`, and with no mime
                // filter: what this walk offers is a row whose words
                // nobody has encoded, and a row's words come from what
                // it says about itself rather than from its bytes. The
                // `ord = 0` in the stored key is the convention the
                // table keeps, not a claim about a material.
                //
                // Absence *or* staleness is the pending state, which is
                // one condition because it is one question: is there a
                // vector here composed the way this build composes. A
                // row written by an older reading is as much work as no
                // row at all, and it is the case a predicate testing
                // only for absence can never see.
                //
                // An asset with nothing to say earns a failure row on
                // its first pass, at the current composition, and is
                // offered no second one until the composition moves.
                let mut stmt = conn.prepare(
                    "SELECT a.id
                       FROM asset a
                      WHERE a.trashed_at IS NULL AND a.folded_into IS NULL
                        AND NOT EXISTS (
                            SELECT 1 FROM visual_feature vf
                             WHERE vf.asset_id = a.id AND vf.ord = 0
                               AND vf.model_id = ?1 AND vf.feature_kind = ?2
                               AND vf.composition_ver >= ?3)
                      ORDER BY a.id
                      LIMIT ?4",
                )?;
                let rows = stmt
                    .query_map(
                        params![ident.model_id, kind, composition, limit as i64],
                        |r| r.get::<_, Uuid>(0),
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }

    async fn clear_derived(&self, model_id: &str) -> Result<u64, DomainError> {
        let model_id = model_id.to_string();
        self.isle
            .call(move |conn| {
                let n = conn.execute(
                    "DELETE FROM visual_feature WHERE model_id = ?1",
                    params![model_id],
                )?;
                Ok(n as u64)
            })
            .await
            .map_err(infra_err)
    }

    async fn stamp_tag_suggested(
        &self,
        asset_id: &AssetId,
        ord: u32,
        identity: &ModelIdentity,
        kind: VisualFeatureKind,
        head: &TagHeadRef,
        at_ms: i64,
    ) -> Result<(), DomainError> {
        let asset = *asset_id.as_uuid();
        let ident = identity.clone();
        let kind_slug = kind.as_str();
        let head = head.as_str().to_string();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE visual_feature
                        SET tag_suggested_at = ?1, tag_suggested_head = ?2
                      WHERE asset_id = ?3 AND ord = ?4 AND model_id = ?5
                        AND feature_kind = ?6",
                    params![at_ms, head, asset, ord, ident.model_id, kind_slug],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn unsuggested(
        &self,
        identity: &ModelIdentity,
        kind: VisualFeatureKind,
        head: &TagHeadRef,
        limit: u32,
    ) -> Result<Vec<AssetId>, DomainError> {
        let ident = identity.clone();
        let kind_slug = kind.as_str();
        let head = head.as_str().to_string();
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                // "Not stamped under the *current* head": a NULL stamp
                // and a superseded head's stamp read the same, which
                // is what makes a head swap re-walk the library (#132)
                // through this ordinary page.
                let mut stmt = conn.prepare(
                    "SELECT vf.asset_id FROM visual_feature vf
                      WHERE vf.model_id = ?1 AND vf.feature_kind = ?2
                        AND vf.preprocess_ver = ?3 AND vf.status = 'computed'
                        AND vf.ord = 0
                        AND (vf.tag_suggested_head IS NULL OR vf.tag_suggested_head <> ?4)
                      ORDER BY vf.asset_id
                      LIMIT ?5",
                )?;
                let rows = stmt
                    .query_map(
                        params![
                            ident.model_id,
                            kind_slug,
                            ident.preprocess_ver,
                            head,
                            limit as i64
                        ],
                        |r| r.get::<_, Uuid>(0),
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;

    fn identity() -> ModelIdentity {
        ModelIdentity {
            model_id: "test-model".into(),
            dim: 4,
            preprocess_ver: 1,
        }
    }

    /// Seed one persona and two image assets with primary materials.
    async fn seed_two_images(isle: &AsyncIsle) -> (PersonaId, AssetId, AssetId) {
        let persona = Uuid::now_v7();
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        isle.call(move |conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p', 'P', 0, 0)",
                params![persona],
            )?;
            for (id, name) in [(a, "/pics/a.png"), (b, "/pics/b.png")] {
                // The storage rendering (v63): the tagged object — the
                // walk's promotion parses it, so a bare path would fail
                // the very boundary the walk exists to cross.
                let locator = serde_json::json!({ "kind": "file", "path": name }).to_string();
                tx.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                        modality, occurred_at, created_at, updated_at)
                     VALUES (?1, ?2, 'fs', ?3, 'image', 0, 0, 0)",
                    params![id, persona, locator],
                )?;
                tx.execute(
                    "INSERT INTO material (asset_id, ord, locator, mime, created_at, updated_at)
                     VALUES (?1, 0, ?2, 'image/png', 0, 0)",
                    params![id, locator],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
        (
            PersonaId::from_uuid(persona),
            AssetId::from_uuid(a),
            AssetId::from_uuid(b),
        )
    }

    /// Seed a second persona with one image asset.
    ///
    /// Its own `pack_id`, because the column is unique and the helper
    /// above spends the one it hard-codes — which is why a test wanting
    /// two personas cannot simply call that one twice.
    async fn seed_another_persona(isle: &AsyncIsle) -> (PersonaId, AssetId) {
        let persona = Uuid::now_v7();
        let asset = Uuid::now_v7();
        isle.call(move |conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'q', 'Q', 0, 0)",
                params![persona],
            )?;
            let locator = serde_json::json!({ "kind": "file", "path": "/pics/q.png" }).to_string();
            tx.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', ?3, 'image', 0, 0, 0)",
                params![asset, persona, locator],
            )?;
            tx.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, created_at, updated_at)
                 VALUES (?1, 0, ?2, 'image/png', 0, 0)",
                params![asset, locator],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
        (PersonaId::from_uuid(persona), AssetId::from_uuid(asset))
    }

    fn feature(asset: AssetId, vector: Vec<f32>) -> VisualFeature {
        VisualFeature::new(
            asset,
            0,
            identity(),
            VisualFeatureKind::Semantic,
            vector,
            7,
            0,
        )
        .unwrap()
    }

    /// The words walk's whole point (#32): a vector composed by an
    /// older reading is as much work as no vector at all, and a
    /// predicate testing only for absence cannot see it.
    ///
    /// This is the failure `asset_body.derived_version` was added to
    /// close on the full-text side, after the first derivation walk had
    /// already left every text asset holding a body composed from its
    /// file alone. The same shape, asserted before it can happen here.
    #[tokio::test]
    async fn the_words_walk_offers_a_stale_composition_and_not_a_current_one() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteVisualFeatureRepository::new(isle.clone());
        let (_persona, a, b) = seed_two_images(&isle).await;

        // Both rows exist. One was composed by an older reading.
        for (asset, composition) in [(a, 1), (b, 2)] {
            repo.set_visual_feature(
                VisualFeature::new(
                    asset,
                    0,
                    identity(),
                    VisualFeatureKind::Words,
                    vec![0.0; 4],
                    0,
                    composition,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        }

        assert_eq!(
            repo.unworded(&identity(), 2, 16).await.unwrap(),
            vec![a],
            "the older composition is work and the current one is not"
        );
        assert!(
            repo.unworded(&identity(), 1, 16).await.unwrap().is_empty(),
            "and at the reading that wrote them, neither is"
        );

        driver.shutdown().await.unwrap();
    }

    /// A row with nothing to say is answered once, not offered every
    /// pass — and the answer is stamped, so a wider reading asks it
    /// again rather than trusting a verdict reached without the
    /// sections it now reads.
    #[tokio::test]
    async fn a_wordless_row_is_answered_once_per_composition() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteVisualFeatureRepository::new(isle.clone());
        let (_persona, a, b) = seed_two_images(&isle).await;

        repo.mark_unextractable(
            &a,
            0,
            &identity(),
            VisualFeatureKind::Words,
            "the row says nothing about itself",
            1,
        )
        .await
        .unwrap();
        repo.set_visual_feature(
            VisualFeature::new(
                b,
                0,
                identity(),
                VisualFeatureKind::Words,
                vec![0.0; 4],
                0,
                1,
            )
            .unwrap(),
        )
        .await
        .unwrap();

        assert!(
            repo.unworded(&identity(), 1, 16).await.unwrap().is_empty(),
            "a failure retires the row as firmly as a vector does"
        );
        assert_eq!(
            repo.unworded(&identity(), 2, 16).await.unwrap().len(),
            2,
            "and a wider reading re-opens both, the answered one included"
        );

        driver.shutdown().await.unwrap();
    }

    /// The stamp survives the round trip, which is what the walk's
    /// comparison rests on.
    #[tokio::test]
    async fn the_composition_reads_back_off_the_row() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteVisualFeatureRepository::new(isle.clone());
        let (_persona, a, _b) = seed_two_images(&isle).await;

        repo.set_visual_feature(
            VisualFeature::new(
                a,
                0,
                identity(),
                VisualFeatureKind::Words,
                vec![1.0, 0.0, 0.0, 0.0],
                7,
                3,
            )
            .unwrap(),
        )
        .await
        .unwrap();

        let stored = repo
            .feature_of(&a, 0, &identity(), VisualFeatureKind::Words)
            .await
            .unwrap()
            .expect("the row is there");
        assert_eq!(stored.composition_ver, 3);
        assert_eq!(stored.kind, VisualFeatureKind::Words);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_vector_round_trips_with_its_identity() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteVisualFeatureRepository::new(isle.clone());
        let (_p, a, _b) = seed_two_images(&isle).await;

        let stored = feature(a, vec![0.1, 0.2, 0.3, 0.4]);
        repo.set_visual_feature(stored.clone()).await.unwrap();

        let back = repo
            .feature_of(&a, 0, &identity(), VisualFeatureKind::Semantic)
            .await
            .unwrap()
            .expect("stored vector comes back");
        assert_eq!(back, stored);

        // A different preprocessing revision is a different generation:
        // the stale vector must not be served as current.
        let newer = ModelIdentity {
            preprocess_ver: 2,
            ..identity()
        };
        assert!(
            repo.feature_of(&a, 0, &newer, VisualFeatureKind::Semantic)
                .await
                .unwrap()
                .is_none()
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_walk_offers_a_row_exactly_once() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteVisualFeatureRepository::new(isle.clone());
        let (_p, a, b) = seed_two_images(&isle).await;

        let page = repo
            .unextracted(&identity(), VisualFeatureKind::Semantic, 10)
            .await
            .unwrap();
        assert_eq!(page.len(), 2);
        assert!(page.iter().all(|c| c.ord == 0));

        // A computed row retires one material; a failure record
        // retires the other just as firmly.
        repo.set_visual_feature(feature(a, vec![0.0; 4]))
            .await
            .unwrap();
        repo.mark_unextractable(
            &b,
            0,
            &identity(),
            VisualFeatureKind::Semantic,
            "undecodable",
            0,
        )
        .await
        .unwrap();

        let page = repo
            .unextracted(&identity(), VisualFeatureKind::Semantic, 10)
            .await
            .unwrap();
        assert!(page.is_empty(), "{page:?}");

        // Another model still sees both: the walk is per identity.
        let other = ModelIdentity {
            model_id: "other-model".into(),
            ..identity()
        };
        assert_eq!(
            repo.unextracted(&other, VisualFeatureKind::Semantic, 10)
                .await
                .unwrap()
                .len(),
            2
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_persona_scan_serves_computed_current_vectors_only() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteVisualFeatureRepository::new(isle.clone());
        let (p, a, b) = seed_two_images(&isle).await;

        repo.set_visual_feature(feature(a, vec![1.0, 0.0, 0.0, 0.0]))
            .await
            .unwrap();
        repo.mark_unextractable(
            &b,
            0,
            &identity(),
            VisualFeatureKind::Semantic,
            "unreadable",
            0,
        )
        .await
        .unwrap();

        let vectors = repo
            .vectors_in_scope(Some(&p), &identity(), VisualFeatureKind::Semantic)
            .await
            .unwrap();
        assert_eq!(vectors.len(), 1, "failed rows carry no vector to scan");
        assert_eq!(vectors[0].0, a);
        assert_eq!(vectors[0].1, p, "the scan says whose asset it found");
        assert_eq!(vectors[0].2, vec![1.0, 0.0, 0.0, 0.0]);

        driver.shutdown().await.unwrap();
    }

    /// An unscoped scan reaches every persona, and a scoped one reaches
    /// exactly the persona named.
    #[tokio::test]
    async fn no_scope_scans_every_persona_and_a_scope_scans_one() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteVisualFeatureRepository::new(isle.clone());
        let (mine, a, _b) = seed_two_images(&isle).await;
        let (theirs, c) = seed_another_persona(&isle).await;
        assert_ne!(mine, theirs, "the fixture must seed two personas");

        repo.set_visual_feature(feature(a, vec![1.0, 0.0, 0.0, 0.0]))
            .await
            .unwrap();
        repo.set_visual_feature(feature(c, vec![0.0, 1.0, 0.0, 0.0]))
            .await
            .unwrap();

        let everywhere = repo
            .vectors_in_scope(None, &identity(), VisualFeatureKind::Semantic)
            .await
            .unwrap();
        let mut found: Vec<(AssetId, PersonaId)> =
            everywhere.iter().map(|(id, p, _)| (*id, *p)).collect();
        found.sort_by_key(|(id, _)| *id);
        let mut want = vec![(a, mine), (c, theirs)];
        want.sort_by_key(|(id, _)| *id);
        assert_eq!(found, want, "an unscoped scan crosses personas");

        let only_mine = repo
            .vectors_in_scope(Some(&mine), &identity(), VisualFeatureKind::Semantic)
            .await
            .unwrap();
        assert_eq!(
            only_mine.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
            vec![a],
            "a scoped scan stops at the persona it was given"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn clear_derived_removes_exactly_one_models_output() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteVisualFeatureRepository::new(isle.clone());
        let (_p, a, b) = seed_two_images(&isle).await;

        repo.set_visual_feature(feature(a, vec![0.0; 4]))
            .await
            .unwrap();
        repo.mark_unextractable(&b, 0, &identity(), VisualFeatureKind::Semantic, "x", 0)
            .await
            .unwrap();
        let other = ModelIdentity {
            model_id: "other-model".into(),
            ..identity()
        };
        repo.set_visual_feature(
            VisualFeature::new(
                a,
                0,
                other.clone(),
                VisualFeatureKind::Semantic,
                vec![0.0; 4],
                9,
                0,
            )
            .unwrap(),
        )
        .await
        .unwrap();

        let removed = repo.clear_derived("test-model").await.unwrap();
        assert_eq!(removed, 2, "vectors and failure records both go");
        assert!(
            repo.feature_of(&a, 0, &other, VisualFeatureKind::Semantic)
                .await
                .unwrap()
                .is_some(),
            "the other model's output survives"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_head_swap_reoffers_what_an_earlier_head_stamped() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteVisualFeatureRepository::new(isle.clone());
        let (_p, a, b) = seed_two_images(&isle).await;
        repo.set_visual_feature(feature(a, vec![0.0; 4]))
            .await
            .unwrap();
        repo.set_visual_feature(feature(b, vec![0.0; 4]))
            .await
            .unwrap();
        let zs = TagHeadRef::zero_shot();
        let kind = VisualFeatureKind::Semantic;

        // The zero-shot pass walks both, stamps one.
        assert_eq!(
            repo.unsuggested(&identity(), kind, &zs, 10).await.unwrap(),
            vec_sorted(a, b)
        );
        repo.stamp_tag_suggested(&a, 0, &identity(), kind, &zs, 9)
            .await
            .unwrap();
        assert_eq!(
            repo.unsuggested(&identity(), kind, &zs, 10).await.unwrap(),
            vec![b]
        );

        // A trained head arrives: the zero-shot stamp does not count,
        // so the whole encoded library is re-offered — the #132 re-
        // score path, through the ordinary walk.
        let trained = TagHeadRef::new("head-v1").unwrap();
        assert_eq!(
            repo.unsuggested(&identity(), kind, &trained, 10)
                .await
                .unwrap(),
            vec_sorted(a, b)
        );
        repo.stamp_tag_suggested(&a, 0, &identity(), kind, &trained, 10)
            .await
            .unwrap();
        assert_eq!(
            repo.unsuggested(&identity(), kind, &trained, 10)
                .await
                .unwrap(),
            vec![b]
        );

        driver.shutdown().await.unwrap();
    }

    /// The walk orders by asset id; the two seeded ids are random, so
    /// tests sort the expectation the same way.
    fn vec_sorted(a: AssetId, b: AssetId) -> Vec<AssetId> {
        let mut v = vec![a, b];
        v.sort_by_key(|id| *id.as_uuid());
        v
    }
}
