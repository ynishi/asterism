//! Trained-head storage (#132 phase 2): the artifact a training run
//! writes, and the one pointer promotion moves.
//!
//! Layout under [`crate::paths::heads_dir`]:
//!
//! ```text
//! heads/
//!   head-v1-9f80e2c1/head.json   one training run's artifact, eval included
//!   head-v2-1a2b3c4d/head.json
//!   current                      the promoted label, or absent — zero-shot
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
//! [`promote`] only moves the pointer. The training caller promotes
//! on a strict held-out win; a pull ([`install_pulled`]) promotes on
//! the person's explicit act — the two verdicts that may move the one
//! pointer.

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

/// The next ordinal label stem under `heads_root`, counting up from
/// `head-v1`. A run appends a content discriminator to the stem
/// (`head-v3-1a2b3c4d`) before writing, so labels are unique across
/// *stores*, not only within one — a pulled head (#132 phase 3)
/// lands under its publisher's label, and two members' third
/// training runs must not claim the same name. The ordinal half
/// stays because a person reading `heads/` should see the history of
/// attempts in order; the discriminator half exists purely so that
/// order never collides.
pub fn next_head_label(heads_root: &Path) -> Result<String> {
    let mut highest = 0u32;
    for entry in std::fs::read_dir(heads_root)
        .with_context(|| format!("cannot read {}", heads_root.display()))?
    {
        let name = entry?.file_name();
        if let Some(n) = name
            .to_str()
            .and_then(|s| s.strip_prefix("head-v"))
            .map(|s| s.split('-').next().unwrap_or(s))
            .and_then(|s| s.parse::<u32>().ok())
        {
            highest = highest.max(n);
        }
    }
    Ok(format!("head-v{}", highest + 1))
}

/// Appends the content discriminator to an ordinal stem: eight hex
/// characters of the rows' digest, so the label is a function of what
/// the head actually scores with.
pub fn discriminated_label(stem: &str, rows: &BTreeMap<String, TrainedRow>) -> Result<String> {
    let serialized = serde_json::to_string(rows)?;
    let digest = asterism_core::domain::content_hash::of_bytes(serialized.as_bytes());
    let hex = digest
        .strip_prefix("sha256:")
        .unwrap_or(&digest)
        .chars()
        .take(8)
        .collect::<String>();
    Ok(format!("{stem}-{hex}"))
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
/// whole mechanism: the scoring side reads it once at startup
/// ([`bind_current`]), bind-once like the encoder, so a promotion
/// applies on the next launch.
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

/// Resolves the promoted head against the bound encoder — the scoring
/// side's one read, at startup.
///
/// `Ok(None)` is the ordinary case: no pointer, zero-shot scores. An
/// error is a pointer that exists but cannot be honoured — a dangling
/// label, a corrupt artifact, an identity that is not the bound
/// encoder's, a row whose width disagrees, a key that is not a tag id
/// — and the caller's move is to warn and score zero-shot: a head
/// that cannot be verified must not score, and startup must not fail
/// over a file the person can delete.
pub fn bind_current(
    heads_root: &Path,
    identity: &ModelIdentity,
) -> Result<Option<asterism_core::domain::tag_head::BoundTagHead>> {
    let Some(label) = current_label(heads_root)? else {
        return Ok(None);
    };
    let artifact = load_artifact(heads_root, &label)?;
    verify_artifact(&artifact, identity)?;
    let head = asterism_core::domain::visual::TagHeadRef::new(label.clone())
        .map_err(|e| anyhow::anyhow!("{label:?} is not a usable head label: {e}"))?;
    let mut rows = BTreeMap::new();
    for (key, row) in artifact.rows {
        let tag = uuid::Uuid::parse_str(&key).expect("verify_artifact checked every key");
        rows.insert(asterism_core::domain::value::TagId::from_uuid(tag), row);
    }
    Ok(Some(asterism_core::domain::tag_head::BoundTagHead {
        head,
        rows,
    }))
}

/// The checks a head must pass before it may score against `identity`
/// — one function, because the startup bind and a pulled install
/// (#132 phase 3) must refuse for exactly the same reasons: an
/// identity that is not the bound encoder's, a row whose width
/// disagrees with it, a key that is not a tag id.
pub fn verify_artifact(artifact: &TagHeadArtifact, identity: &ModelIdentity) -> Result<()> {
    let label = &artifact.head;
    // The label becomes a directory name under the heads store: a
    // separator, a dot-dot, or an absolute path (which `Path::join`
    // substitutes wholesale) would let a pulled artifact write — and
    // point `current` — outside the store. Locally minted labels can
    // never look like this; a pulled one must be refused before any
    // path is built from it.
    if label.is_empty() || label.contains(['/', '\\']) || label == "." || label == ".." {
        bail!("head label {label:?} is not a plain directory name");
    }
    if artifact.model_id != identity.model_id
        || artifact.dim != identity.dim
        || artifact.preprocess_ver != identity.preprocess_ver
    {
        bail!(
            "head {label} was trained under {}/{}d/p{}, the bound encoder is {}/{}d/p{} — \
             a head scores only against the vectors it learned from",
            artifact.model_id,
            artifact.dim,
            artifact.preprocess_ver,
            identity.model_id,
            identity.dim,
            identity.preprocess_ver
        );
    }
    for (key, row) in &artifact.rows {
        uuid::Uuid::parse_str(key)
            .with_context(|| format!("head {label} keys a row by {key:?}, not a tag id"))?;
        if row.weights.len() != identity.dim as usize {
            bail!(
                "head {label}'s row for {key} is {} wide, the encoder produces {}",
                row.weights.len(),
                identity.dim
            );
        }
    }
    Ok(())
}

/// Installs a pulled head artifact and promotes it — the member half
/// of #132 phase 3. Returns the label promotion now points at.
///
/// The artifact is verified against the bound encoder with the same
/// checks the startup bind runs ([`verify_artifact`]): a pulled head
/// that could not bind must not install. The label is the
/// publisher's, kept as-is: labels carry a content discriminator
/// since the pull exists (see [`next_head_label`]), so a collision
/// means one of two things — the identical artifact, in which case
/// the install is a re-promote; or a genuinely different head under
/// the same name, which is refused rather than renamed, because
/// silently renaming would detach the label the team talks about
/// from the bytes that score.
pub fn install_pulled(heads_root: &Path, raw: &str, identity: &ModelIdentity) -> Result<String> {
    let artifact: TagHeadArtifact =
        serde_json::from_str(raw).context("the pulled bytes are not a head artifact")?;
    if artifact.schema != HEAD_ARTIFACT_SCHEMA {
        bail!(
            "the pulled artifact carries schema {:?}, not {HEAD_ARTIFACT_SCHEMA:?}",
            artifact.schema
        );
    }
    verify_artifact(&artifact, identity)?;
    let label = artifact.head.clone();
    if heads_root.join(&label).join(HEAD_FILE).exists() {
        let existing = load_artifact(heads_root, &label)
            .with_context(|| format!("a local head under {label} exists but cannot be read"))?;
        // A label identifies the ROWS — the bytes that score — which
        // is exactly what its content discriminator digests. Same
        // rows under the same label is the same head (the local copy
        // stays, eval and timestamps included; artifacts are
        // immutable) and pulling it again is a re-promote. Different
        // rows under the same label is refused, never renamed.
        if existing.rows != artifact.rows {
            bail!(
                "a different head already holds the label {label} locally; \
                 the publisher labels a new train a new label"
            );
        }
    } else {
        write_artifact(heads_root, &artifact)?;
    }
    promote(heads_root, &label)?;
    Ok(label)
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

    #[test]
    fn bind_current_honours_the_pointer_only_against_its_own_encoder() {
        let root = tempfile::tempdir().unwrap();

        // No pointer: the ordinary zero-shot answer, not an error.
        assert!(bind_current(root.path(), &identity()).unwrap().is_none());

        write_artifact(root.path(), &artifact("head-v1")).unwrap();
        promote(root.path(), "head-v1").unwrap();
        let bound = bind_current(root.path(), &identity()).unwrap().unwrap();
        assert_eq!(bound.head.as_str(), "head-v1");
        assert_eq!(bound.rows.len(), 1);
        let tag = asterism_core::domain::value::TagId::from_uuid(
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        );
        assert!(bound.rows.contains_key(&tag));

        // A different encoder identity refuses the head instead of
        // scoring vectors it never learned from.
        let other = ModelIdentity {
            model_id: "other-model".into(),
            ..identity()
        };
        assert!(bind_current(root.path(), &other).is_err());

        // A dangling pointer is an error too — the caller warns and
        // scores zero-shot, and deleting the pointer heals it.
        std::fs::write(root.path().join(CURRENT_FILE), "head-v9").unwrap();
        assert!(bind_current(root.path(), &identity()).is_err());
    }

    #[test]
    fn bind_current_refuses_rows_it_cannot_key_or_size() {
        let root = tempfile::tempdir().unwrap();
        let mut bad_key = artifact("head-v1");
        bad_key.rows.insert(
            "not-a-uuid".to_string(),
            TrainedRow {
                weights: vec![0.0; 3],
                bias: 0.0,
            },
        );
        write_artifact(root.path(), &bad_key).unwrap();
        promote(root.path(), "head-v1").unwrap();
        assert!(bind_current(root.path(), &identity()).is_err());

        let mut bad_width = artifact("head-v2");
        bad_width.rows.insert(
            "00000000-0000-0000-0000-000000000002".to_string(),
            TrainedRow {
                weights: vec![0.0; 2],
                bias: 0.0,
            },
        );
        write_artifact(root.path(), &bad_width).unwrap();
        promote(root.path(), "head-v2").unwrap();
        assert!(bind_current(root.path(), &identity()).is_err());
    }

    #[test]
    fn a_pulled_artifact_installs_verifies_and_promotes() {
        let root = tempfile::tempdir().unwrap();
        let pulled = artifact("head-v3-1a2b3c4d");
        let raw = serde_json::to_string(&pulled).unwrap();

        let label = install_pulled(root.path(), &raw, &identity()).unwrap();
        assert_eq!(label, "head-v3-1a2b3c4d");
        assert_eq!(current_label(root.path()).unwrap(), Some(label.clone()));
        assert_eq!(load_artifact(root.path(), &label).unwrap(), pulled);

        // Pulling the identical artifact again is a re-promote, not a
        // collision.
        promote(root.path(), &label).unwrap();
        assert_eq!(
            install_pulled(root.path(), &raw, &identity()).unwrap(),
            label
        );

        // A different head under the same label is refused — the
        // label the team talks about must stay attached to the bytes
        // that score.
        let mut different = pulled.clone();
        different
            .rows
            .get_mut("00000000-0000-0000-0000-000000000001")
            .unwrap()
            .bias = 9.0;
        let raw = serde_json::to_string(&different).unwrap();
        assert!(install_pulled(root.path(), &raw, &identity()).is_err());

        // A head trained against another encoder never installs.
        let mut foreign = artifact("head-v9-cafecafe");
        foreign.model_id = "other-model".into();
        let raw = serde_json::to_string(&foreign).unwrap();
        assert!(install_pulled(root.path(), &raw, &identity()).is_err());
        assert!(!root.path().join("head-v9-cafecafe").exists());

        // Same rows, different run metadata: the label identifies the
        // rows, so this is the same head and pulls as a re-promote.
        let mut same_rows = pulled.clone();
        same_rows.trained_at_ms = 999;
        same_rows.eval.held_out = 40;
        let raw = serde_json::to_string(&same_rows).unwrap();
        assert_eq!(
            install_pulled(root.path(), &raw, &identity()).unwrap(),
            "head-v3-1a2b3c4d"
        );
        // The local copy stays as written — artifacts are immutable.
        assert_eq!(
            load_artifact(root.path(), "head-v3-1a2b3c4d").unwrap(),
            pulled
        );

        // A label that reaches outside the store is refused before
        // any path is built from it.
        let mut hostile = artifact("../escape");
        hostile.head = "../escape".into();
        let raw = serde_json::to_string(&hostile).unwrap();
        assert!(install_pulled(root.path(), &raw, &identity()).is_err());
        let absolute = {
            let mut a = artifact("/tmp/evil");
            a.head = "/tmp/evil".into();
            serde_json::to_string(&a).unwrap()
        };
        assert!(install_pulled(root.path(), &absolute, &identity()).is_err());
    }

    #[test]
    fn ordinal_stems_count_past_discriminated_labels() {
        let root = tempfile::tempdir().unwrap();
        write_artifact(root.path(), &artifact("head-v2-deadbeef")).unwrap();
        // The stem parser reads the ordinal before the discriminator,
        // so the next run counts from it instead of restarting at 1.
        assert_eq!(next_head_label(root.path()).unwrap(), "head-v3");

        // The discriminator is a function of the rows: same rows,
        // same label; different rows, different label.
        let rows = artifact("x").rows;
        let a = discriminated_label("head-v3", &rows).unwrap();
        let b = discriminated_label("head-v3", &rows).unwrap();
        assert_eq!(a, b);
        assert!(
            a.starts_with("head-v3-") && a.len() == "head-v3-".len() + 8,
            "{a}"
        );
        let mut other = rows.clone();
        other
            .get_mut("00000000-0000-0000-0000-000000000001")
            .unwrap()
            .bias = 1.0;
        assert_ne!(a, discriminated_label("head-v3", &other).unwrap());
    }
}
