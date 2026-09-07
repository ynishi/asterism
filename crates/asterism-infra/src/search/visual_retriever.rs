//! The retriever that answers from stored vectors: `Similar` from an
//! asset's pixels (#112), and the tail of `Text` from what assets say
//! about themselves (#32).
//!
//! A composite over the text retriever. `Similar` becomes a
//! brute-force cosine scan over stored feature vectors under the bound
//! model. `Text` runs full text first and appends what the meaning
//! layer proposes for assets full text did not name — the two are not
//! competing rankings, they are an instrument and a suggestion in that
//! order.
//!
//! Brute force on purpose — an ANN structure earns its complexity only
//! when a measurement says the scan misses a latency target. What is
//! scanned is no longer always one persona's vectors, so the sizing to
//! keep in view is the whole library's: #32's own note puts 50k assets
//! at 512 dimensions of `f32` near 100 MB, and this reads them per
//! query.
//!
//! # Both routes scan what the query asked about
//!
//! The query's scope is the population, and the two routes reach it
//! differently only because they start differently. `Similar` starts
//! from an asset, so an unscoped query falls back to that asset's own
//! persona — the library being asked about is the one the subject
//! lives in. `Text` starts from words and has no such anchor, so an
//! unscoped query scans every persona, which is the population full
//! text answered for the same query.
//!
//! Requiring a scope on the text route instead is what kept the
//! meaning layer switched off in the app's default state, where no
//! persona is selected until somebody selects one.
//!
//! Degradation is layered the way the rest of the feature degrades. No
//! bound encoder means `Similar` declines exactly as the text-only
//! build declines it, and `Text` answers with full text alone — which
//! is what it answered with before this layer existed, so a build or a
//! profile without a model is not a build with a broken search. A bound
//! encoder with no stored vector for the query asset returns the empty
//! set: "not encoded yet" is an honest nothing, not an error.

use std::sync::{Arc, OnceLock};

use asterism_core::domain::repository::{
    AssetRepository, AssetRetriever, Candidate, Evidence, RETRIEVAL_K_CEILING, RetrievalIntent,
    RetrievalQuery, Retrieved, Route, VisualFeatureRepository,
};
use asterism_core::domain::value::{AssetId, PersonaId};
use asterism_core::domain::visual::{VisualEncoder, VisualFeatureKind, cosine_normalized};
use asterism_core::error::DomainError;
use async_trait::async_trait;

use crate::sqlite::repo::{SqliteAssetRepository, SqliteVisualFeatureRepository};

/// Composite retriever: text with the meaning layer appended to it,
/// `Similar` from stored vectors.
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
            .vectors_in_scope(Some(&persona), &identity, VisualFeatureKind::Semantic)
            .await?;
        let k = q.k.clamp(1, RETRIEVAL_K_CEILING) as usize;
        let mut scored: Vec<(AssetId, f32)> = vectors
            .into_iter()
            .filter(|(id, _, _)| id != asset_id)
            .map(|(id, _, v)| (id, cosine_normalized(&feature.vector, &v)))
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
                    route: Route::Neighbour,
                    // Nothing to say beyond the score: the neighbour
                    // route's whole statement is "these pixels are
                    // near those", which the score already is.
                    evidence: Evidence::None,
                })
                .collect(),
            truncated,
        })
    }
}

/// How close a query has to sit before this layer proposes an asset.
///
/// **Not a ranking threshold.** The measurement says no threshold holds
/// both precision and recall here — the closest they came was 0.585
/// and 0.633 at a threshold of 0.79, and either side of that one of
/// them collapses — which is why this layer proposes in rank order and
/// lets metadata dispose, the arrangement #32 asks for.
///
/// What this floor is for is the honest miss: the point below which a
/// query the library cannot answer returns nothing instead of padding.
/// On the fixture set a query about nothing in the library sat at most
/// 0.722 from anything in it [measured: `text_recall_eval`, 24 scenes,
/// seed 42], so the floor sits **above** that number rather than at
/// it. At 0.72 the comparison is `>=`, and the very row the
/// measurement was cited to exclude would be proposed.
///
/// A number from generated scenes, so it is the shape of the answer
/// rather than the answer. The twenty real queries #32 asks for are
/// what would move it.
const MEANING_FLOOR: f32 = 0.73;

impl VisualAwareRetriever {
    /// Full text first, then what the meaning layer proposes for
    /// assets full text did not name (#32).
    ///
    /// The order is the claim: what was written down is answered by the
    /// instrument that indexes what was written down, and this layer
    /// only ever appends. An asset full text already found is not
    /// re-proposed — it is in the answer, and a second entry would say
    /// nothing except that two routes agreed.
    async fn text_and_meaning(
        &self,
        text: &str,
        q: &RetrievalQuery,
    ) -> Result<Retrieved, DomainError> {
        let found = self.text.retrieve(q).await?;
        // One way this route is simply absent, and it leaves full text
        // as the whole answer rather than failing: no bound model, in a
        // build that has no encoder.
        //
        // The scan follows the query's scope rather than requiring one.
        // A search that names no persona is asking the whole store, and
        // full text answers it that way; a meaning layer that went
        // silent there would be off in the app's own default state,
        // which is where it was until this was fixed.
        let Some(encoder) = self.encoder.get() else {
            return Ok(found);
        };
        let identity = encoder.identity().clone();
        // Through the crate's one encode gate, not inline: this runs on
        // every search now, the tower blocks for long enough to stall
        // the runtime thread it lands on, and a query encoding beside a
        // backfill job is exactly what the permit exists to prevent.
        let query_vector = crate::encode::text(encoder.clone(), text.to_string()).await?;
        let vectors = self
            .visual
            .vectors_in_scope(q.scope.as_ref(), &identity, VisualFeatureKind::Words)
            .await?;

        let already: std::collections::HashSet<AssetId> =
            found.candidates.iter().map(|c| c.asset_id).collect();
        let mut proposed: Vec<(AssetId, PersonaId, f32)> = vectors
            .into_iter()
            .filter(|(id, _, _)| !already.contains(id))
            .map(|(id, persona, v)| (id, persona, cosine_normalized(&query_vector, &v)))
            .filter(|(_, _, score)| *score >= MEANING_FLOOR)
            .collect();
        proposed.sort_by(|a, b| b.2.total_cmp(&a.2));

        let k = q.k.clamp(1, RETRIEVAL_K_CEILING) as usize;
        let room = k.saturating_sub(found.candidates.len());
        let truncated = found.truncated || proposed.len() > room;
        let mut candidates = found.candidates;
        candidates.extend(
            proposed
                .into_iter()
                .take(room)
                .map(|(asset_id, persona_id, score)| Candidate {
                    asset_id,
                    // The row's own persona, not the query's: under an
                    // unscoped search there is no one persona to attribute
                    // a candidate to.
                    persona_id,
                    score,
                    route: Route::Meaning,
                    // What a reader is told beyond the route. The text
                    // itself is not carried here and does not need to
                    // be: `derive_words` is a function of the asset, so
                    // the words this matched on are recomposable from
                    // the row at any later moment.
                    evidence: Evidence::Rationale("the asset's own words".into()),
                }),
        );
        Ok(Retrieved {
            candidates,
            truncated,
        })
    }
}

#[async_trait]
impl AssetRetriever for VisualAwareRetriever {
    async fn retrieve(&self, q: &RetrievalQuery) -> Result<Retrieved, DomainError> {
        match &q.intent {
            RetrievalIntent::Text(text) => self.text_and_meaning(&text.clone(), q).await,
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

    /// An encoder that returns the vector it was built with, whatever
    /// it is asked to encode — so a test states the query's position in
    /// the space directly instead of through a real tokenizer.
    struct FixedText {
        identity: ModelIdentity,
        vector: Vec<f32>,
    }

    impl VisualEncoder for FixedText {
        fn identity(&self) -> &ModelIdentity {
            &self.identity
        }
        fn encode_image(&self, _: &[u8], _: u32, _: u32) -> Result<Vec<f32>, DomainError> {
            unreachable!("the words route never encodes an image")
        }
        fn encode_text(&self, _: &str) -> Result<Vec<f32>, DomainError> {
            Ok(self.vector.clone())
        }
    }

    /// A text retriever answering with exactly what it was given.
    struct FoundText(Vec<Candidate>);

    #[async_trait]
    impl AssetRetriever for FoundText {
        async fn retrieve(&self, _q: &RetrievalQuery) -> Result<Retrieved, DomainError> {
            Ok(Retrieved {
                candidates: self.0.clone(),
                truncated: false,
            })
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

    /// Full text answers first and the meaning layer only appends —
    /// and an asset full text already named is not proposed twice.
    #[tokio::test]
    async fn meaning_appends_to_full_text_and_never_repeats_it() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let visual = SqliteVisualFeatureRepository::new(isle.clone());
        let assets = SqliteAssetRepository::new(isle.clone());
        let (persona, ids) = seed(&isle).await;

        // Two assets sit exactly where the query does, one sits away
        // from it. Full text names the first of the two.
        for (id, vector) in [
            (ids[0], vec![1.0, 0.0, 0.0]),
            (ids[1], vec![1.0, 0.0, 0.0]),
            (ids[2], vec![0.0, 1.0, 0.0]),
        ] {
            visual
                .set_visual_feature(
                    VisualFeature::new(id, 0, identity(), VisualFeatureKind::Words, vector, 0, 0)
                        .unwrap(),
                )
                .await
                .unwrap();
        }

        let from_text = vec![Candidate {
            asset_id: ids[0],
            persona_id: persona,
            score: 9.0,
            route: Route::FullText,
            evidence: Evidence::Snippet("the words that were written down".into()),
        }];
        let cell: Arc<OnceLock<Arc<dyn VisualEncoder>>> = Arc::new(OnceLock::new());
        let retriever = VisualAwareRetriever::new(
            Arc::new(FoundText(from_text)),
            visual.clone(),
            assets.clone(),
            cell.clone(),
        );
        let query = RetrievalQuery {
            intent: RetrievalIntent::Text("anything".into()),
            scope: Some(persona),
            k: 10,
        };

        // Unbound cell: full text is the whole answer, which is what a
        // build without this layer gives rather than an error.
        let out = retriever.retrieve(&query).await.unwrap();
        assert_eq!(out.candidates.len(), 1);
        assert_eq!(out.candidates[0].asset_id, ids[0]);

        cell.set(Arc::new(FixedText {
            identity: identity(),
            vector: vec![1.0, 0.0, 0.0],
        }))
        .map_err(|_| ())
        .unwrap();

        let out = retriever.retrieve(&query).await.unwrap();
        assert_eq!(
            out.candidates
                .iter()
                .map(|c| c.asset_id)
                .collect::<Vec<_>>(),
            vec![ids[0], ids[1]],
            "what full text found comes first, and only what it missed is added"
        );
        assert!(
            matches!(out.candidates[0].evidence, Evidence::Snippet(_)),
            "full text keeps its own evidence"
        );
        assert!(
            matches!(out.candidates[1].evidence, Evidence::Rationale(_)),
            "and the appended one says which route reached it"
        );

        driver.shutdown().await.unwrap();
    }

    /// A query the library cannot answer returns what full text
    /// returned, and not one padded row.
    #[tokio::test]
    async fn a_query_below_the_floor_proposes_nothing() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let visual = SqliteVisualFeatureRepository::new(isle.clone());
        let assets = SqliteAssetRepository::new(isle.clone());
        let (persona, ids) = seed(&isle).await;

        visual
            .set_visual_feature(
                VisualFeature::new(
                    ids[0],
                    0,
                    identity(),
                    VisualFeatureKind::Words,
                    vec![0.0, 1.0, 0.0],
                    0,
                    0,
                )
                .unwrap(),
            )
            .await
            .unwrap();

        let cell: Arc<OnceLock<Arc<dyn VisualEncoder>>> = Arc::new(OnceLock::new());
        cell.set(Arc::new(FixedText {
            identity: identity(),
            // Orthogonal to the only stored vector: a cosine of 0,
            // which is as far below the floor as this space goes.
            vector: vec![1.0, 0.0, 0.0],
        }))
        .map_err(|_| ())
        .unwrap();
        let retriever = VisualAwareRetriever::new(
            Arc::new(FoundText(Vec::new())),
            visual.clone(),
            assets.clone(),
            cell.clone(),
        );

        let out = retriever
            .retrieve(&RetrievalQuery {
                intent: RetrievalIntent::Text("nothing here answers this".into()),
                scope: Some(persona),
                k: 10,
            })
            .await
            .unwrap();
        assert!(
            out.candidates.is_empty(),
            "a miss says so rather than padding: {:?}",
            out.candidates
        );

        driver.shutdown().await.unwrap();
    }

    /// A query naming no persona is asking the whole store, and the
    /// meaning route answers it there too.
    ///
    /// This is the app's own default state — nothing is selected in the
    /// persona strip until somebody selects it — so a route that went
    /// silent without a scope would be a route the app never reached.
    /// It did, until this test was inverted.
    ///
    /// The candidate's persona comes off the row rather than off the
    /// query, which under no scope is the only place it can come from.
    #[tokio::test]
    async fn a_query_with_no_persona_still_reaches_the_meaning_route() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let visual = SqliteVisualFeatureRepository::new(isle.clone());
        let assets = SqliteAssetRepository::new(isle.clone());
        let (persona, ids) = seed(&isle).await;

        visual
            .set_visual_feature(
                VisualFeature::new(
                    ids[0],
                    0,
                    identity(),
                    VisualFeatureKind::Words,
                    vec![1.0, 0.0, 0.0],
                    0,
                    0,
                )
                .unwrap(),
            )
            .await
            .unwrap();

        let cell: Arc<OnceLock<Arc<dyn VisualEncoder>>> = Arc::new(OnceLock::new());
        cell.set(Arc::new(FixedText {
            identity: identity(),
            vector: vec![1.0, 0.0, 0.0],
        }))
        .map_err(|_| ())
        .unwrap();
        let retriever = VisualAwareRetriever::new(
            Arc::new(FoundText(Vec::new())),
            visual.clone(),
            assets.clone(),
            cell.clone(),
        );

        let out = retriever
            .retrieve(&RetrievalQuery {
                intent: RetrievalIntent::Text("a query with no scope".into()),
                scope: None,
                k: 10,
            })
            .await
            .unwrap();
        assert_eq!(
            out.candidates.len(),
            1,
            "the vector sits exactly where the query does: {:?}",
            out.candidates
        );
        assert_eq!(out.candidates[0].asset_id, ids[0]);
        assert_eq!(
            out.candidates[0].persona_id, persona,
            "the candidate names the persona whose asset it is"
        );
        assert!(
            matches!(out.candidates[0].evidence, Evidence::Rationale(_)),
            "the meaning route says it was the one that reached this row"
        );

        driver.shutdown().await.unwrap();
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
                    VisualFeature::new(
                        id,
                        0,
                        identity(),
                        VisualFeatureKind::Semantic,
                        vector,
                        0,
                        0,
                    )
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
