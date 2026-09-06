//! # asterism-vision — model *use* for visual features (#112)
//!
//! This crate is the use side of the visual-feature split: what ships in
//! the app. Model *preparation* — qualification, conversion, packaging,
//! and one day training — is a provider-side tool outside the app's
//! dependency graph; the two meet only at the model-package data
//! contract (weights plus a manifest naming `model_id`, digest,
//! dimensions, preprocessing version, and license).
//!
//! Two answers to "are these the same picture" live here, and they are
//! deliberately not one. [`perceptual`] reduces the pixels themselves
//! to a fingerprint that survives a resize; it needs no model, links in
//! every build, and answers only whether one image is a copy of
//! another. [`encoder`] runs a packaged image/text model through ONNX
//! Runtime behind the `onnx` feature and answers the looser question of
//! what an image resembles — which is why a threshold that serves one
//! of them serves neither the other's question nor the exact digests in
//! `asterism-core`.
//!
//! [`fixtures`] is what both are graded against: deterministic scenes
//! with derived ground truth, in memory, at test time — deliberately
//! not a corpus, because nothing outside the system consumes it.

#![warn(missing_docs)]

#[cfg(feature = "onnx")]
pub mod encoder;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod package;
pub mod perceptual;
