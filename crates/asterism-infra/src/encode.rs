//! The one gate every model encode in this crate passes through.
//!
//! Inference is CPU-bound and there is a person in front of the host,
//! so two things are owed at every call site: the work leaves the async
//! runtime's threads, and only one of them runs at a time. Both are
//! here rather than repeated at the call sites, because the second is
//! only true if there is exactly one permit — two semaphores of one
//! would allow precisely the pair of concurrent encodes the permit
//! exists to prevent.
//!
//! Its own module rather than [`crate::vision`], which would otherwise
//! be the natural home: that module is behind the `vision` feature, and
//! the callers are not. The job layer and the retriever both hold the
//! encoder through the core port, and a build with no model bound still
//! compiles them.
//!
//! Two callers today, and they are the pair that makes the permit
//! matter: the backfill walk encoding an asset's words, and search
//! encoding the query somebody just typed.

use std::sync::Arc;

use asterism_core::domain::visual::VisualEncoder;
use asterism_core::error::DomainError;

/// One encode at a time: decode plus inference is CPU-bound the way
/// thumbnail decodes are, and the same host-responsiveness argument
/// applies (see `THUMB_DECODE_SLOTS` in the job layer, which is the
/// same argument for the same reason).
pub(crate) static ENCODE_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// Runs the text tower under the permit, off the runtime's threads.
///
/// The `String` rather than a `&str` is what `spawn_blocking` requires
/// — the closure outlives the call — and the `Arc` clone is the same
/// requirement for the encoder.
pub(crate) async fn text(
    encoder: Arc<dyn VisualEncoder>,
    text: String,
) -> Result<Vec<f32>, DomainError> {
    let _permit = ENCODE_SLOTS
        .acquire()
        .await
        .expect("semaphore never closed");
    tokio::task::spawn_blocking(move || encoder.encode_text(&text))
        .await
        .map_err(|e| DomainError::Validation(format!("encode task failed: {e}")))?
}
