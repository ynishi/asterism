//! Visual-feature vocabulary and the encoder port (#112).
//!
//! Everything a model produces is **derived state with an identity**:
//! a vector is meaningless without knowing which model, at which
//! preprocessing revision, produced it. [`ModelIdentity`] is that
//! identity, and it travels with every stored vector, every suggestion,
//! and every synthetic edge a model produces — so replacing a model
//! invalidates exactly its own output and nothing a person asserted.
//!
//! The core knows the *shape* of the work — identities, vectors, the
//! encoder port — and none of the machinery. ONNX Runtime, weights,
//! preprocessing kernels live behind [`VisualEncoder`] in the model-use
//! crate; this module must never grow a model dependency (that is
//! acceptance item 4 of #112, and the reason the port takes a raw RGB
//! buffer rather than an image type).

use crate::domain::value::{AssetId, TagId};
use crate::error::DomainError;

/// The derivation identity of everything one model configuration
/// produces.
///
/// Two of the three fields are versioning: `preprocess_ver` moves when
/// the resize / normalization recipe changes even though the weights
/// did not, because a vector encoded under a different recipe is not
/// comparable and must be invalidated with the same certainty as a
/// model swap. The dimension is carried — not derived — so a stored
/// blob whose length disagrees with its declared identity is detectably
/// corrupt rather than silently wrong.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelIdentity {
    /// Stable id of the model package (for example
    /// `"siglip2-base-patch16-256"`). Part of every derived row's key.
    pub model_id: String,
    /// Vector dimensionality this identity produces.
    pub dim: u32,
    /// Revision of the preprocessing recipe (resize mode, normalization
    /// constants) the vectors were encoded under.
    pub preprocess_ver: u32,
}

/// What kind of feature a stored vector is.
///
/// One kind exists today. The enum exists so that a later image-only
/// feature (a DINOv2-class vector, a learned perceptual code) can share
/// the storage without a migration — the kind is part of the row key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisualFeatureKind {
    /// A joint image/text embedding: comparable with encoded text
    /// (tag names, captions) and with other images.
    Semantic,
}

impl VisualFeatureKind {
    /// Slug shared by the DB layer and DTOs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
        }
    }

    /// Parses a slug (unknown values yield a validation error).
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "semantic" => Ok(Self::Semantic),
            other => Err(DomainError::Validation(format!(
                "unknown visual feature kind: {other:?}"
            ))),
        }
    }
}

/// One stored feature vector and its full derivation identity.
#[derive(Debug, Clone, PartialEq)]
pub struct VisualFeature {
    /// Asset the vector describes.
    pub asset_id: AssetId,
    /// Which material of the asset was encoded (`0` = primary).
    pub ord: u32,
    /// Who derived it, under which recipe.
    pub identity: ModelIdentity,
    /// What kind of feature it is.
    pub kind: VisualFeatureKind,
    /// The vector, L2-normalized at write time so similarity is a dot
    /// product. Its length must equal `identity.dim`.
    pub vector: Vec<f32>,
    /// When extraction ran (epoch ms).
    pub extracted_at_ms: i64,
}

impl VisualFeature {
    /// Rejects a vector whose length disagrees with its identity —
    /// the corruption [`ModelIdentity::dim`] exists to make loud.
    pub fn new(
        asset_id: AssetId,
        ord: u32,
        identity: ModelIdentity,
        kind: VisualFeatureKind,
        vector: Vec<f32>,
        extracted_at_ms: i64,
    ) -> Result<Self, DomainError> {
        if vector.len() != identity.dim as usize {
            return Err(DomainError::Validation(format!(
                "vector length {} disagrees with declared dim {} of {}",
                vector.len(),
                identity.dim,
                identity.model_id
            )));
        }
        Ok(Self {
            asset_id,
            ord,
            identity,
            kind,
            vector,
            extracted_at_ms,
        })
    }
}

/// Where one tag suggestion stands between the model and the person.
///
/// The machine writes only the first state, and only where no row
/// exists; the other two are a person's ruling and are never
/// overwritten by a rerun. A rejection is scoped to the model that
/// earned it — a materially different model may re-suggest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagSuggestionDisposition {
    /// The model proposed it; nobody has ruled.
    Suggested,
    /// A person took it — the tag link in `asset_tag` is the durable
    /// half, this row is the audit trail.
    Accepted,
    /// A person refused it; the row stays so this model cannot ask
    /// again.
    Rejected,
}

impl TagSuggestionDisposition {
    /// Slug shared by the DB layer and DTOs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Suggested => "suggested",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    /// Parses a slug (unknown values yield a validation error).
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "suggested" => Ok(Self::Suggested),
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            other => Err(DomainError::Validation(format!(
                "unknown tag suggestion disposition: {other:?}"
            ))),
        }
    }
}

/// One scored tag suggestion with its full derivation identity.
#[derive(Debug, Clone, PartialEq)]
pub struct TagEvidence {
    /// Asset the suggestion is about.
    pub asset_id: AssetId,
    /// The proposed channel tag.
    pub tag_id: TagId,
    /// Which model proposed it.
    pub model_id: String,
    /// Cosine similarity that cleared the floor.
    pub score: f32,
    /// Where the suggestion stands.
    pub disposition: TagSuggestionDisposition,
    /// When the model proposed it (epoch ms).
    pub suggested_at_ms: i64,
    /// When a person ruled, if they have.
    pub resolved_at_ms: Option<i64>,
}

/// Cosine similarity of two L2-normalized vectors: the dot product.
///
/// Lives in the domain because the score's *meaning* (a suggestion
/// strength that must clear a floor before anything is written) is a
/// domain rule, even though heavy scans run in adapters.
pub fn cosine_normalized(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Encoder port: pixels (or text) in, one normalized vector out.
///
/// Synchronous on purpose — encoding is CPU/accelerator-bound compute,
/// and the job layer decides how to schedule it. Implementations load a
/// model *package* (weights + manifest) prepared by the provider-side
/// tooling; the core never sees where the bytes came from.
pub trait VisualEncoder: Send + Sync {
    /// The identity every vector this encoder produces carries.
    fn identity(&self) -> &ModelIdentity;

    /// Encodes one decoded image, given as a tightly-packed RGB8 buffer
    /// (`width * height * 3` bytes). Returns an L2-normalized vector of
    /// exactly `identity().dim` elements.
    fn encode_image(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<f32>, DomainError>;

    /// Encodes a text (a tag name, a query) into the same space.
    fn encode_text(&self, text: &str) -> Result<Vec<f32>, DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> ModelIdentity {
        ModelIdentity {
            model_id: "test-model".into(),
            dim: 4,
            preprocess_ver: 1,
        }
    }

    #[test]
    fn a_vector_must_match_its_declared_dim() {
        let ok = VisualFeature::new(
            AssetId::new(),
            0,
            identity(),
            VisualFeatureKind::Semantic,
            vec![0.5; 4],
            0,
        );
        assert!(ok.is_ok());
        let wrong = VisualFeature::new(
            AssetId::new(),
            0,
            identity(),
            VisualFeatureKind::Semantic,
            vec![0.5; 3],
            0,
        );
        assert!(wrong.is_err(), "a length mismatch must be loud");
    }

    #[test]
    fn feature_kind_round_trips_through_its_slug() {
        assert_eq!(VisualFeatureKind::Semantic.as_str(), "semantic");
        assert_eq!(
            VisualFeatureKind::parse("semantic").unwrap(),
            VisualFeatureKind::Semantic
        );
        assert!(VisualFeatureKind::parse("visual").is_err());
    }

    #[test]
    fn cosine_of_normalized_vectors_behaves() {
        let a = [1.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0];
        assert_eq!(cosine_normalized(&a, &a), 1.0);
        assert_eq!(cosine_normalized(&a, &b), 0.0);
        let opposite = [-1.0, 0.0, 0.0];
        assert_eq!(cosine_normalized(&a, &opposite), -1.0);
    }
}
