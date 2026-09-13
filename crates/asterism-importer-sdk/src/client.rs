//! Thin HTTP client for the asterism-server API.
//!
//! Wraps the endpoints an importer actually needs: the two that land
//! records (`POST /asterism/assets/add` and `/add-batch`), and the two
//! that keep its position between runs, behind
//! [`HttpSyncStore`].

use anyhow::{Context, anyhow};
use asterism_contract::command::{
    AddAssetBatchCommand, AddAssetBatchResult, AddAssetCommand, ReadImportStateCommand,
    WriteImportStateCommand,
};
use asterism_contract::dto::{AssetDto, ImportStateDto};
use async_trait::async_trait;

use crate::port::{SourceError, SyncState};
use crate::store::{StateKey, SyncStore};

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

/// The [`SyncStore`] an adapter that pushes over HTTP uses.
///
/// The whole of the transport decision, in one type. An adapter runs
/// itself, posts its records to a server, and keeps its position in the
/// same place over the same connection — so nothing has to start the
/// adapter, and nothing has to read its output, for it to be
/// incremental.
///
/// Wraps an [`ApiClient`] rather than building a second HTTP stack:
/// the base URL, the client and the error shape are all already
/// decided, and a second one would be a second thing to configure and a
/// second thing to get wrong.
pub struct HttpSyncStore {
    client: ApiClient,
}

impl HttpSyncStore {
    /// Binds a store to the same server the records go to.
    pub fn new(client: ApiClient) -> Self {
        Self { client }
    }
}

/// Failures are classified the way the rest of an import classifies
/// them, because the caller reading them is the same caller.
///
/// A request that did not complete is [`Transient`](SourceError::Transient):
/// the server is not running yet, the socket went away, and the same
/// call in a moment may well work. A request that completed and was
/// refused is [`Config`](SourceError::Config): a persona that does not
/// exist, a body this build spelled wrong, and repeating it changes
/// nothing until somebody does.
fn unreachable(what: &str, err: reqwest::Error) -> SourceError {
    SourceError::Transient(format!("{what}: {err}"))
}

#[async_trait]
impl SyncStore for HttpSyncStore {
    async fn read(&self, key: &StateKey) -> Result<Option<SyncState>, SourceError> {
        let command = ReadImportStateCommand {
            persona_id: key.persona_id.clone(),
            partition: key.partition.clone(),
        };
        let resp = self
            .client
            .inner
            .post(self.client.url("/asterism/import/state/read"))
            .json(&command)
            .send()
            .await
            .map_err(|e| unreachable("reading the resumption point", e))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(SourceError::Config(format!(
                "reading the resumption point: HTTP {status}: {body}"
            )));
        }
        // `null` is the first-run answer and decodes to `None`, which is
        // why this is one decode and not a status check.
        let dto = resp
            .json::<Option<ImportStateDto>>()
            .await
            .map_err(|e| SourceError::Source(format!("resumption point did not decode: {e}")))?;
        let Some(dto) = dto else {
            return Ok(None);
        };
        let offset = serde_json::from_str(&dto.offset_json).map_err(|e| {
            SourceError::Source(format!(
                "the stored resumption point is not JSON, which only this \
                 importer could have written: {e}"
            ))
        })?;
        Ok(Some(SyncState::new(dto.partition, offset)))
    }

    async fn write(&self, key: &StateKey, state: &SyncState) -> Result<(), SourceError> {
        // Serialised here rather than passed as a value, so what is
        // stored is the text this run produced — and what comes back is
        // the same text, key order included. The workspace builds
        // `serde_json` with `preserve_order`, which makes that a real
        // property rather than an accident.
        let offset_json = serde_json::to_string(&state.offset).map_err(|e| {
            SourceError::Source(format!("this scanner's own offset did not serialise: {e}"))
        })?;
        let command = WriteImportStateCommand {
            persona_id: key.persona_id.clone(),
            partition: key.partition.clone(),
            offset_json,
        };
        let resp = self
            .client
            .inner
            .post(self.client.url("/asterism/import/state/write"))
            .json(&command)
            .send()
            .await
            .map_err(|e| unreachable("storing the resumption point", e))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(SourceError::Config(format!(
                "storing the resumption point: HTTP {status}: {body}"
            )));
        }
        Ok(())
    }
}
