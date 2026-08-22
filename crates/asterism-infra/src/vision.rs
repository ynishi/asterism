//! Adapter from `asterism-vision`'s encoder to the core
//! [`VisualEncoder`] port (#112).
//!
//! The port is `&self` (the job engine shares one encoder across
//! handlers), the underlying `ort` session runs on `&mut self`, so the
//! adapter owns the lock. A `Mutex` and not a pool: the job layer
//! already serialises encodes through its own semaphore, so contention
//! here is the exception, not the shape.

use std::sync::Mutex;

use asterism_core::domain::visual::{ModelIdentity, VisualEncoder};
use asterism_core::error::DomainError;
use asterism_vision::encoder::Encoder;
use asterism_vision::package::ModelPackage;

/// A bound model package, exposed through the core port.
pub struct OrtVisualEncoder {
    inner: Mutex<Encoder>,
    identity: ModelIdentity,
}

impl OrtVisualEncoder {
    /// Opens and loads the package at `dir` — digest verification and
    /// session creation in one blocking call, so callers (composition
    /// roots) hold one name instead of two crates.
    pub fn open_dir(dir: &std::path::Path) -> Result<Self, DomainError> {
        let package = ModelPackage::open(dir)
            .map_err(|e| DomainError::Validation(format!("cannot open model package: {e}")))?;
        Self::load(&package)
    }

    /// Loads an opened package into an adapter the encoder cell can
    /// hold. The package's manifest is the identity every vector will
    /// carry.
    pub fn load(package: &ModelPackage) -> Result<Self, DomainError> {
        let encoder = Encoder::load(package)
            .map_err(|e| DomainError::Validation(format!("cannot load model package: {e}")))?;
        let identity = ModelIdentity {
            model_id: encoder.model_id().to_string(),
            dim: encoder.dim(),
            preprocess_ver: encoder.preprocess_ver(),
        };
        Ok(Self {
            inner: Mutex::new(encoder),
            identity,
        })
    }
}

impl VisualEncoder for OrtVisualEncoder {
    fn identity(&self) -> &ModelIdentity {
        &self.identity
    }

    fn encode_image(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<f32>, DomainError> {
        self.inner
            .lock()
            .expect("encoder lock poisoned")
            .encode_image(rgb, width, height)
            .map_err(|e| DomainError::Validation(format!("encode_image failed: {e}")))
    }

    fn encode_text(&self, text: &str) -> Result<Vec<f32>, DomainError> {
        self.inner
            .lock()
            .expect("encoder lock poisoned")
            .encode_text(text)
            .map_err(|e| DomainError::Validation(format!("encode_text failed: {e}")))
    }
}
