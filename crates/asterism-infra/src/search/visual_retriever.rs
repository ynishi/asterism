//! The retriever that answers `Similar` from stored vectors (#112).
//!
//! A composite over the text retriever: `Text` delegates unchanged,
//! `Similar` becomes a brute-force cosine scan over the persona's
//! stored feature vectors under the bound model. Brute force on
//! purpose — at personal-library scale the whole scan is a few
//! megabytes of f32, and an ANN structure earns its complexity only
//! when the P2-5 measurements say the scan misses a latency target.
//!
//! Degradation is layered the way the rest of the feature degrades:
//! no bound encoder means `Similar` declines exactly as the text-only
//! build declines it; a bound encoder with no stored vector for the
//! query asset returns the empty set — "not encoded yet" is an honest
//! nothing, not an error.

use std::sync::{Arc, OnceLock};

use asterism_core::domain::repository::{
    AssetRepository, AssetRetriever, Candidate, Evidence, RETRIEVAL_K_CEILING, RetrievalIntent,
    RetrievalQuery, Retrieved, VisualFeatureRepository,
};
use asterism_core::domain::value::AssetId;
use asterism_core::domain::visual::{VisualEncoder, VisualFeatureKind, cosine_normalized};
use asterism_core::error::DomainError;
use async_trait::async_trait;

use crate::sqlite::repo::{SqliteAssetRepository, SqliteVisualFeatureRepository};

/// Composite retriever: text unchanged, `Similar` from stored vectors.
pub struct VisualAwareRetriever {
    text: Arc<dyn AssetRetriever>,
    visual: SqliteVisualFeatureRepository,
    assets: SqliteAssetRepository,
    encoder: Arc<OnceLock<Arc<dyn VisualEncoder>>>,
}

impl VisualAwareRetriever {
    /// Wraps the text retriever with the visual route.
    pub fn new(
        text: Arc<dyn AssetRetriever>,
        visual: SqliteVisualFeatureRepository,
        assets: SqliteAssetRepository,
        encoder: Arc<OnceLock<Arc<dyn VisualEncoder>>>,
    ) -> Self {
        Self {
            text,
            visual,
            assets,
            encoder,
        }
    }

    async fn similar(
        &self,
        asset_id: &AssetId,
        q: &RetrievalQuery,
    ) -> Result<Retrieved, DomainError> {
        let empty = Retrieved {
            candidates: Vec::new(),
            truncated: false,
        };
        // No bound model: the route does not exist in this process,
        // the same answer the text-only adapter gives.
        let Some(encoder) = self.encoder.get() else {
            return Err(DomainError::Validation(
                "similar-asset retrieval has no backing index in this build".into(),
            ));
        };
        let identity = encoder.identity().clone();
        let Some(feature) = self
            .visual
            .feature_of(asset_id, 0, &identity, VisualFeatureKind::Semantic)
            .await?
        else {
            return Ok(empty);
        };
        // The scan is persona-scoped; an explicit scope wins, otherwise
        // the query asset's own persona is the library being asked.
        let persona = match &q.scope {
            Some(persona) => *persona,
            None => match self.assets.find(asset_id).await? {
                Some(asset) => asset.persona_id,
                None => return Ok(empty),
            },
        };
        let vectors = self
            .visual
            .vectors_of_persona(&persona, &identity, VisualFeatureKind::Semantic)
            .await?;
        let k = q.k.clamp(1, RETRIEVAL_K_CEILING) as usize;
        let mut scored: Vec<(AssetId, f32)> = vectors
            .into_iter()
            .filter(|(id, _)| id != asset_id)
            .map(|(id, v)| (id, cosine_normalized(&feature.vector, &v)))
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let truncated = scored.len() > k;
        scored.truncate(k);
        Ok(Retrieved {
            candidates: scored
                .into_iter()
                .map(|(asset_id, score)| Candidate {
                    asset_id,
                    persona_id: persona,
                    score,
                    evidence: Evidence::None,
                })
                .collect(),
            truncated,
        })
    }
}

#[async_trait]
impl AssetRetriever for VisualAwareRetriever {
    async fn retrieve(&self, q: &RetrievalQuery) -> Result<Retrieved, DomainError> {
        match &q.intent {
            RetrievalIntent::Text(_) => self.text.retrieve(q).await,
            RetrievalIntent::Similar(asset_id) => self.similar(&asset_id.clone(), q).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::value::PersonaId;
    use asterism_core::domain::visual::{ModelIdentity, VisualFeature};
    use rusqlite::params;
    use uuid::Uuid;

    struct UnitEncoder {
        identity: ModelIdentity,
    }

    impl VisualEncoder for UnitEncoder {
        fn identity(&self) -> &ModelIdentity {
            &self.identity
        }
        fn encode_image(&self, _: &[u8], _: u32, _: u32) -> Result<Vec<f32>, DomainError> {
            unreachable!("the retriever never encodes")
        }
        fn encode_text(&self, _: &str) -> Result<Vec<f32>, DomainError> {
            unreachable!("the retriever never encodes")
        }
    }

    /// A refusing text retriever: these tests must never route there.
    struct NoText;

    #[async_trait]
    impl AssetRetriever for NoText {
        async fn retrieve(&self, _q: &RetrievalQuery) -> Result<Retrieved, DomainError> {
            Err(DomainError::Validation("text route not under test".into()))
        }
    }

    fn identity() -> ModelIdentity {
        ModelIdentity {
            model_id: "test-model".into(),
            dim: 3,
            preprocess_ver: 1,
        }
    }

    async fn seed(isle: &rusqlite_isle::AsyncIsle) -> (PersonaId, Vec<AssetId>) {
        let persona = Uuid::now_v7();
        let ids: Vec<Uuid> = (0..3).map(|_| Uuid::now_v7()).collect();
        let ids_for_sql = ids.clone();
        isle.call(move |conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p', 'P', 0, 0)",
                params![persona],
            )?;
            for (n, id) in ids_for_sql.iter().enumerate() {
                let locator =
                    serde_json::json!({ "kind": "file", "path": format!("/pics/{n}.png") })
                        .to_string();
                tx.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                        modality, occurred_at, created_at, updated_at)
                     VALUES (?1, ?2, 'fs', ?3, 'image', 0, 0, 0)",
                    params![id, persona, locator],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
        (
            PersonaId::from_uuid(persona),
            ids.into_iter().map(AssetId::from_uuid).collect(),
        )
    }

    #[tokio::test]
    async fn similar_ranks_by_cosine_and_declines_without_a_model() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let visual = SqliteVisualFeatureRepository::new(isle.clone());
        let assets = SqliteAssetRepository::new(isle.clone());
        let (_persona, ids) = seed(&isle).await;

        for (id, vector) in [
            (ids[0], vec![1.0, 0.0, 0.0]),
            (ids[1], vec![0.9486833, 0.31622776, 0.0]),
            (ids[2], vec![0.0, 0.0, 1.0]),
        ] {
            visual
                .set_visual_feature(
                    VisualFeature::new(id, 0, identity(), VisualFeatureKind::Semantic, vector, 0)
                        .unwrap(),
                )
                .await
                .unwrap();
        }

        let cell: Arc<OnceLock<Arc<dyn VisualEncoder>>> = Arc::new(OnceLock::new());
        let retriever = VisualAwareRetriever::new(
            Arc::new(NoText),
            visual.clone(),
            assets.clone(),
            cell.clone(),
        );
        let query = RetrievalQuery {
            intent: RetrievalIntent::Similar(ids[0]),
            scope: None,
            k: 10,
        };

        // Unbound cell: the route declines like the text-only build.
        assert!(retriever.retrieve(&query).await.is_err());

        cell.set(Arc::new(UnitEncoder {
            identity: identity(),
        }))
        .map_err(|_| ())
        .unwrap();
        let out = retriever.retrieve(&query).await.unwrap();
        assert_eq!(out.candidates.len(), 2);
        // The near-parallel vector outranks the orthogonal one, and
        // the query asset itself is excluded.
        assert_eq!(out.candidates[0].asset_id, ids[1]);
        assert!(out.candidates[0].score > 0.9);
        assert_eq!(out.candidates[1].asset_id, ids[2]);
        assert!(out.candidates[1].score < 0.1);

        // An asset with no stored vector answers with nothing.
        let unencoded = RetrievalQuery {
            intent: RetrievalIntent::Similar(AssetId::new()),
            scope: None,
            k: 10,
        };
        assert!(
            retriever
                .retrieve(&unencoded)
                .await
                .unwrap()
                .candidates
                .is_empty()
        );

        driver.shutdown().await.unwrap();
    }
}
