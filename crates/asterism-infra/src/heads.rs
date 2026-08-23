//! Trained-head storage (#132 phase 2): the artifact a training run
//! writes, and the one pointer promotion moves.
//!
//! Layout under [`crate::paths::heads_dir`]:
//!
//! ```text
//! heads/
//!   head-v1/head.json     one training run's artifact, eval included
//!   head-v2/head.json
//!   current               the promoted label, or absent — zero-shot
//! ```
//!
//! An artifact is immutable once written: a retrain is a new label,
//! never an overwrite, so `current` can move backwards as well as
//! forwards and every eval stays inspectable. The artifact stores
//! **only the trained rows** — tags below the training floor keep
//! their zero-shot behaviour implicitly — which is why a head is
//! kilobytes, not megabytes, at realistic ruling counts.
//!
//! Promotion is the caller's verdict, not this module's: writing an
//! artifact records what training produced (wins and losses alike, a
//! loss being exactly the evidence that zero-shot should stand);
//! [`promote`] moves the pointer and is called only on a strict
//! held-out win.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use asterism_core::domain::tag_head::{HeadEval, TrainedRow};
use asterism_core::domain::visual::ModelIdentity;
use serde::{Deserialize, Serialize};

/// The artifact schema this module writes and reads.
pub const HEAD_ARTIFACT_SCHEMA: &str = "asterism-tag-head-v1";

/// File name of the artifact inside a head's directory.
pub const HEAD_FILE: &str = "head.json";

/// File name of the promotion pointer inside the heads root.
pub const CURRENT_FILE: &str = "current";

/// One training run, persisted whole: identity, rows, and the eval
/// that judged it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagHeadArtifact {
    /// Always [`HEAD_ARTIFACT_SCHEMA`].
    pub schema: String,
    /// The head's label — the [`TagHeadRef`] value suggestions and
    /// stamps will carry when this head scores.
    ///
    /// [`TagHeadRef`]: asterism_core::domain::visual::TagHeadRef
    pub head: String,
    /// The encoder identity the training vectors were cached under —
    /// a head is meaningless against any other encoder.
    pub model_id: String,
    /// The encoder's vector dimensionality (every row's width).
    pub dim: u32,
    /// The encoder's preprocessing revision.
    pub preprocess_ver: u32,
    /// Trained rows, keyed by tag id (hyphenated UUID). Absent tags
    /// keep zero-shot.
    pub rows: BTreeMap<String, TrainedRow>,
    /// The held-out verdict that decided promotion.
    pub eval: HeadEval,
    /// Rulings consumed by this run, both classes, all tags.
    pub rulings_used: usize,
    /// When the run finished (epoch ms).
    pub trained_at_ms: i64,
}

/// The next unused label under `heads_root`, counting up from
/// `head-v1`. Labels are ordinal, not content-addressed: two runs over
/// identical rulings would produce identical rows, and a person
/// reading `heads/` should see the history of attempts, not a dedupe.
pub fn next_head_label(heads_root: &Path) -> Result<String> {
    let mut highest = 0u32;
    for entry in std::fs::read_dir(heads_root)
        .with_context(|| format!("cannot read {}", heads_root.display()))?
    {
        let name = entry?.file_name();
        if let Some(n) = name
            .to_str()
            .and_then(|s| s.strip_prefix("head-v"))
            .and_then(|s| s.parse::<u32>().ok())
        {
            highest = highest.max(n);
        }
    }
    Ok(format!("head-v{}", highest + 1))
}

/// Writes one run's artifact under its label. Refuses an existing
/// label: artifacts are immutable, and a collision means the caller
/// skipped [`next_head_label`].
pub fn write_artifact(heads_root: &Path, artifact: &TagHeadArtifact) -> Result<PathBuf> {
    let dir = heads_root.join(&artifact.head);
    if dir.exists() {
        bail!(
            "head {} already exists; artifacts are immutable — a retrain is a new label",
            artifact.head
        );
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let path = dir.join(HEAD_FILE);
    std::fs::write(&path, serde_json::to_string_pretty(artifact)?)
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}

/// Points `current` at a label — the promotion. The pointer is the
/// whole mechanism: the scoring side (the follow-up branch of #132)
/// will read it once at startup, bind-once like the encoder. Until
/// that side lands, the pointer records the verdict and nothing
/// consumes it — the zero-shot pass keeps scoring.
pub fn promote(heads_root: &Path, label: &str) -> Result<()> {
    if !heads_root.join(label).join(HEAD_FILE).exists() {
        bail!("cannot promote {label}: no artifact under that label");
    }
    let path = heads_root.join(CURRENT_FILE);
    std::fs::write(&path, label).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// The promoted label, if any. Absent (or dangling) means zero-shot.
pub fn current_label(heads_root: &Path) -> Result<Option<String>> {
    let path = heads_root.join(CURRENT_FILE);
    let label = match std::fs::read_to_string(&path) {
        Ok(label) => label.trim().to_string(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    if label.is_empty() {
        return Ok(None);
    }
    Ok(Some(label))
}

/// Reads one label's artifact, verifying the schema tag and that the
/// artifact answers to the label it sits under.
pub fn load_artifact(heads_root: &Path, label: &str) -> Result<TagHeadArtifact> {
    let path = heads_root.join(label).join(HEAD_FILE);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let artifact: TagHeadArtifact = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a head artifact", path.display()))?;
    if artifact.schema != HEAD_ARTIFACT_SCHEMA {
        bail!(
            "{} carries schema {:?}, not {HEAD_ARTIFACT_SCHEMA:?}",
            path.display(),
            artifact.schema
        );
    }
    if artifact.head != label {
        bail!(
            "{} says it is {:?} but sits under {label:?}",
            path.display(),
            artifact.head
        );
    }
    Ok(artifact)
}

/// Builds an artifact from a run's outputs, stamping the schema and
/// the identity fields from the encoder's.
pub fn artifact_for(
    head: String,
    identity: &ModelIdentity,
    rows: BTreeMap<String, TrainedRow>,
    eval: HeadEval,
    rulings_used: usize,
    trained_at_ms: i64,
) -> TagHeadArtifact {
    TagHeadArtifact {
        schema: HEAD_ARTIFACT_SCHEMA.to_string(),
        head,
        model_id: identity.model_id.clone(),
        dim: identity.dim,
        preprocess_ver: identity.preprocess_ver,
        rows,
        eval,
        rulings_used,
        trained_at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> ModelIdentity {
        ModelIdentity {
            model_id: "test-model".into(),
            dim: 3,
            preprocess_ver: 1,
        }
    }

    fn artifact(label: &str) -> TagHeadArtifact {
        let mut rows = BTreeMap::new();
        rows.insert(
            "00000000-0000-0000-0000-000000000001".to_string(),
            TrainedRow {
                weights: vec![0.1, -0.2, 0.3],
                bias: 0.05,
            },
        );
        artifact_for(
            label.to_string(),
            &identity(),
            rows,
            HeadEval {
                held_out: 4,
                candidate_correct: 3,
                baseline_correct: 2,
            },
            16,
            7,
        )
    }

    #[test]
    fn labels_count_up_artifacts_round_trip_and_stay_immutable() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(next_head_label(root.path()).unwrap(), "head-v1");

        let a = artifact("head-v1");
        write_artifact(root.path(), &a).unwrap();
        assert_eq!(load_artifact(root.path(), "head-v1").unwrap(), a);
        assert_eq!(next_head_label(root.path()).unwrap(), "head-v2");

        // Immutable: the same label cannot be written twice.
        assert!(write_artifact(root.path(), &a).is_err());
        // A stray directory does not derail the count.
        std::fs::create_dir(root.path().join("not-a-head")).unwrap();
        assert_eq!(next_head_label(root.path()).unwrap(), "head-v2");
    }

    #[test]
    fn promotion_moves_one_pointer_and_never_invents_a_head() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(current_label(root.path()).unwrap(), None);
        assert!(
            promote(root.path(), "head-v1").is_err(),
            "no artifact, no promotion"
        );

        write_artifact(root.path(), &artifact("head-v1")).unwrap();
        promote(root.path(), "head-v1").unwrap();
        assert_eq!(
            current_label(root.path()).unwrap(),
            Some("head-v1".to_string())
        );

        // The pointer moves backwards as well as forwards: a rollback
        // is a promotion of an older label.
        write_artifact(root.path(), &artifact("head-v2")).unwrap();
        promote(root.path(), "head-v2").unwrap();
        promote(root.path(), "head-v1").unwrap();
        assert_eq!(
            current_label(root.path()).unwrap(),
            Some("head-v1".to_string())
        );
    }

    #[test]
    fn a_mislabeled_or_misschemaed_artifact_is_refused_on_read() {
        let root = tempfile::tempdir().unwrap();
        let mut wrong = artifact("head-v1");
        wrong.head = "head-v9".to_string();
        // Written under v1 while claiming v9: the write path takes the
        // artifact's word for the directory, so build the mismatch by
        // hand the way a stray copy would.
        std::fs::create_dir_all(root.path().join("head-v1")).unwrap();
        std::fs::write(
            root.path().join("head-v1").join(HEAD_FILE),
            serde_json::to_string(&wrong).unwrap(),
        )
        .unwrap();
        assert!(load_artifact(root.path(), "head-v1").is_err());

        let mut off_schema = artifact("head-v2");
        off_schema.schema = "asterism-tag-head-v0".to_string();
        std::fs::create_dir_all(root.path().join("head-v2")).unwrap();
        std::fs::write(
            root.path().join("head-v2").join(HEAD_FILE),
            serde_json::to_string(&off_schema).unwrap(),
        )
        .unwrap();
        assert!(load_artifact(root.path(), "head-v2").is_err());
    }
}
