//! Thin HTTP client for the asterism-server API.
//!
//! Only wraps the two endpoints an importer actually needs:
//! `POST /asterism/assets/add` (single) and
//! `POST /asterism/assets/add-batch`.

use anyhow::{Context, anyhow};
use asterism_contract::command::{AddAssetBatchCommand, AddAssetBatchResult, AddAssetCommand};
use asterism_contract::dto::AssetDto;

/// HTTP client bound to a running `asterism-server`.
#[derive(Debug, Clone)]
pub struct ApiClient {
    base_url: String,
    inner: reqwest::Client,
}

impl ApiClient {
    /// Wraps an HTTP client around `base_url` (for example
    /// `http://127.0.0.1:8989`; no trailing slash).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            inner: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Server health probe. Returns `Ok(())` iff the endpoint answers
    /// with 2xx.
    pub async fn health(&self) -> anyhow::Result<()> {
        let resp = self
            .inner
            .get(self.url("/asterism/health"))
            .send()
            .await
            .context("health request failed")?;
        if !resp.status().is_success() {
            return Err(anyhow!("health: HTTP {}", resp.status()));
        }
        Ok(())
    }

    /// Ingests a single asset.
    pub async fn add_asset(&self, command: AddAssetCommand) -> anyhow::Result<AssetDto> {
        let resp = self
            .inner
            .post(self.url("/asterism/assets/add"))
            .json(&command)
            .send()
            .await
            .context("add_asset request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("add_asset: HTTP {status}: {body}"));
        }
        resp.json::<AssetDto>()
            .await
            .context("add_asset response decode failed")
    }

    /// Uploads a pre-generated thumbnail (raw bytes, typically JPEG)
    /// for `asset_id` at `size_px`. The server stores it in the
    /// `thumb_cache` SQLite BLOB table; subsequent grid renders serve
    /// it via `GET /asterism/assets/{id}/thumbs/{size_px}`.
    pub async fn upload_thumb(
        &self,
        asset_id: &str,
        size_px: u32,
        bytes: Vec<u8>,
    ) -> anyhow::Result<()> {
        let path = format!("/asterism/assets/{asset_id}/thumbs/{size_px}");
        let resp = self
            .inner
            .put(self.url(&path))
            .header("content-type", "image/jpeg")
            .body(bytes)
            .send()
            .await
            .context("upload_thumb request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("upload_thumb: HTTP {status}: {body}"));
        }
        Ok(())
    }

    /// Ingests a batch of assets in one call. Per-item failures are
    /// reflected in the returned [`AddAssetBatchResult`] rather than
    /// raised as an error.
    pub async fn add_asset_batch(
        &self,
        command: AddAssetBatchCommand,
    ) -> anyhow::Result<AddAssetBatchResult> {
        let resp = self
            .inner
            .post(self.url("/asterism/assets/add-batch"))
            .json(&command)
            .send()
            .await
            .context("add_asset_batch request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("add_asset_batch: HTTP {status}: {body}"));
        }
        resp.json::<AddAssetBatchResult>()
            .await
            .context("add_asset_batch response decode failed")
    }
}
