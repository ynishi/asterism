//! `ThumbService` — pre-generated thumbnail cache use cases.
//!
//! Thin wrapper over the `ThumbRepository` port. The service exists so
//! the wire adapters (Tauri command, HTTP handler) share the same
//! entry-point instead of each poking at the repo directly. Encoding
//! and resize decisions live upstream in the importer — the service
//! only stores and retrieves opaque bytes.

use std::sync::Arc;

use crate::application::mapping::parse_uuid;
use crate::domain::repository::ThumbRepository;
use crate::domain::value::AssetId;
use crate::error::DomainError;

/// Thumbnail cache use-case service. Shared as an `Arc`.
pub struct ThumbService {
    repo: Arc<dyn ThumbRepository>,
}

impl ThumbService {
    /// Constructs the service around a repository port.
    pub fn new(repo: Arc<dyn ThumbRepository>) -> Self {
        Self { repo }
    }

    /// Stores (or replaces) a thumbnail for `asset_id` at `size_px`.
    ///
    /// `data` is opaque bytes — the encoding (JPEG / WebP / PNG) is
    /// whatever the caller produced. The UI treats it as
    /// `image/jpeg` by default; callers that want a different type
    /// need to plumb it separately.
    pub async fn put(
        &self,
        asset_id: &str,
        size_px: u32,
        data: Vec<u8>,
    ) -> Result<(), DomainError> {
        let id = AssetId::from_uuid(parse_uuid(asset_id, "asset_id")?);
        self.repo.upsert(&id, size_px, data).await
    }

    /// Retrieves a cached thumbnail if one exists.
    pub async fn get(&self, asset_id: &str, size_px: u32) -> Result<Option<Vec<u8>>, DomainError> {
        let id = AssetId::from_uuid(parse_uuid(asset_id, "asset_id")?);
        self.repo.get(&id, size_px).await
    }

    /// Retrieves many cached thumbnails at one size, answering in the
    /// order asked — slot `i` belongs to `asset_ids[i]`.
    ///
    /// The grid paints a screenful at a time, so this is the shape its
    /// fetch should have. Asking one at a time cost a round trip per
    /// tile and showed up as p95 10.4 s under repeated scrolling
    /// [measured 2026-08-05, bench-scroll-v3].
    ///
    /// A malformed id fails the whole call rather than turning into a
    /// `None` slot: ids reaching here come from rows this app just
    /// handed out, so a bad one is a defect upstream, and answering
    /// "not cached" would hide it as a permanently blank tile.
    pub async fn get_many(
        &self,
        asset_ids: &[String],
        size_px: u32,
    ) -> Result<Vec<Option<Vec<u8>>>, DomainError> {
        let ids = asset_ids
            .iter()
            .map(|raw| parse_uuid(raw, "asset_id").map(AssetId::from_uuid))
            .collect::<Result<Vec<_>, _>>()?;
        self.repo.get_many(&ids, size_px).await
    }
}
