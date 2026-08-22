//! Evaluation fixtures for the visual pipeline: known relationships,
//! generated in-process.
//!
//! `PUBLIC_DEVELOPMENT.md` rules personal library images out of the
//! repository, so the material that grades pHash and the encoder is
//! generated: deterministic scenes whose spec *is* the ground truth.
//! This is deliberately not a corpus — no directory layout, no manifest
//! file, no CLI. Tests and benches call [`scene::render`] and
//! [`relations::RelationStream`] directly and grade in memory; the only
//! identity a measurement needs to cite is the seed.
//!
//! [`scene`] renders one spec; [`relations`] streams specs together with
//! the relatives the pipeline must find (look-alike, semantic sibling,
//! hard negative) and the material it must refuse (unrelated noise and
//! queries). File-level variants (exact copy, resize, recompress, crop)
//! are transforms in [`scene`], applied by the evaluation at call time.

#[cfg(feature = "onnx")]
pub mod eval;
pub mod relations;
pub mod scene;
