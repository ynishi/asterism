//! # asterism-vision — model *use* for visual features (#112)
//!
//! This crate is the use side of the visual-feature split: what ships in
//! the app. Model *preparation* — qualification, conversion, packaging,
//! and one day training — is a provider-side tool outside the app's
//! dependency graph; the two meet only at the model-package data
//! contract (weights plus a manifest naming `model_id`, digest,
//! dimensions, preprocessing version, and license).
//!
//! The implementation arrives in phases. Phase 1 lands the perceptual
//! hash; Phase 2 the ONNX-encoder path (load, identity check, clean
//! degradation when no model is placed, derived-state invalidation on
//! replacement). What exists today is the evaluation half those phases
//! are graded against: [`fixtures`] generates deterministic scenes with
//! derived ground truth, in memory, at test time — deliberately not a
//! corpus, because nothing outside the system consumes it.

#![warn(missing_docs)]

#[cfg(feature = "onnx")]
pub mod encoder;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod package;
