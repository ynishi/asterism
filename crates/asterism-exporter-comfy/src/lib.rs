//! # asterism-exporter-comfy
//!
//! First-slice Asterism `Exporter` — dispatches jobs to a running
//! ComfyUI HTTP backend and harvests generated images.
//!
//! Scope of this crate at first-slice:
//!
//! - Implements the [`asterism_dispatch_sdk::Exporter`] trait against
//!   the ComfyUI HTTP prompt-queue API (`POST /prompt`, `GET
//!   /history/{prompt_id}`, output files served under `/view`).
//! - Supports the single action `"img2img"`. The Comfy workflow JSON
//!   is passed through `params.workflow` verbatim; the exporter's
//!   only job is to substitute the input image references and read
//!   the produced files back out.
//! - Output images land under a caller-configurable dir (`params.output_dir`
//!   defaults to `$ASTERISM_HOME/dispatch/<dispatch_id>/`) so the reified
//!   `Asset` locator points at a stable on-disk path.
//!
//! Everything more ambitious (workflow template registry, txt2img /
//! upscale actions, streaming previews, WebSocket progress) is
//! deferred — the `Exporter` trait leaves room for those without
//! touching this crate's surface.
//!
//! ## Params contract
//!
//! `CreateDispatchCommand.params_json` for this exporter deserialises
//! into [`ComfyDispatchParams`]:
//!
//! ```json
//! {
//!   "endpoint": "http://127.0.0.1:8188",
//!   "workflow": { /* ComfyUI prompt graph JSON */ },
//!   "output_dir": "/optional/absolute/path",
//!   "input_slot": "load_image_node_id",
//!   "poll_interval_ms": 2000
//! }
//! ```
//!
//! - `endpoint` — Comfy base URL (no trailing slash).
//! - `workflow` — the exact prompt graph the Comfy UI would submit.
//!   The exporter walks it looking for `input_slot` and rewrites that
//!   node's `image` field to the Selection's first input.
//! - `output_dir` — absolute directory to write the harvested files
//!   to. `None` = fall back to `$ASTERISM_HOME/dispatch/<id>/` on the
//!   caller side.
//! - `input_slot` — the id of the workflow node whose `image` input
//!   should be substituted with the Selection's first asset locator.
//! - `poll_interval_ms` — how often the runner will poll; the value
//!   is echoed back into the progress hint so the UI can display a
//!   correct spinner cadence.

use std::time::Duration;

use asterism_contract::dto::AssetCardDto;
use asterism_dispatch_sdk::{
    Derived, DispatchContext, DispatchState, Exporter, ExporterError, Handle, ProgressHint,
};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Slug the registry uses for this exporter.
pub const SLUG: &str = "comfy";
/// The single action supported at first-slice.
pub const ACTION_IMG2IMG: &str = "img2img";

/// Public name for this exporter's params schema in the
/// `asterism-server schema` CLI (`exporter:comfy:params`).
pub const SCHEMA_NAME: &str = "exporter:comfy:params";

/// Canonical example JSON for [`ComfyDispatchParams`] — streamed by
/// `asterism-server schema print exporter:comfy:params`.
pub fn params_example_json() -> &'static str {
    include_str!("../schema/comfy_params.example.json")
}

/// Params schema parsed from `params_json`.
#[derive(Debug, Clone, Deserialize)]
pub struct ComfyDispatchParams {
    /// Comfy base URL (e.g. `http://127.0.0.1:8188`, no trailing slash).
    pub endpoint: String,
    /// Verbatim ComfyUI prompt graph JSON. The exporter mutates
    /// `input_slot`'s image field before submission.
    pub workflow: Value,
    /// Absolute directory the harvested files should be written to
    /// (or, more accurately, the directory the exporter reads
    /// Comfy's output from). Interpreted by the caller-side runtime;
    /// the exporter only echoes it back so downstream Assets can
    /// carry a stable locator.
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Workflow node id whose `image` field should be swapped for the
    /// Selection's first input asset.
    pub input_slot: String,
    /// How often the caller should poll (ms). Echoed on the progress
    /// hint.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_poll_interval_ms() -> u64 {
    2000
}

/// Payload persisted on `DispatchJob.handle` for later poll / harvest
/// calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ComfyHandlePayload {
    /// ComfyUI's opaque prompt id (returned by `POST /prompt`).
    prompt_id: String,
    /// Base URL to hit on subsequent polls.
    endpoint: String,
    /// Directory where harvested files live (mirrors params).
    output_dir: Option<String>,
    /// Poll interval echoed back so the SDK's `ProgressHint` can
    /// carry it.
    poll_interval_ms: u64,
}

/// HTTP-backed Exporter against a ComfyUI backend.
///
/// Cheap to `Clone` — the underlying reqwest client uses connection
/// pooling.
#[derive(Debug, Clone)]
pub struct ComfyHttpExporter {
    http: reqwest::Client,
}

impl Default for ComfyHttpExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ComfyHttpExporter {
    /// Builds an exporter with a default reqwest client.
    pub fn new() -> Self {
        Self::with_client(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client build"),
        )
    }

    /// Uses a caller-supplied reqwest client (integration tests
    /// substitute a mocked one).
    pub fn with_client(http: reqwest::Client) -> Self {
        Self { http }
    }
}

fn parse_params(ctx: &DispatchContext<'_>) -> Result<ComfyDispatchParams, ExporterError> {
    serde_json::from_value(ctx.params.clone())
        .map_err(|e| ExporterError::BackendRejected(format!("invalid comfy params: {e}")))
}

fn parse_handle_payload(handle: &Handle) -> Result<ComfyHandlePayload, ExporterError> {
    if handle.kind != SLUG {
        return Err(ExporterError::HandleMismatch {
            exporter_slug: SLUG.into(),
            handle_kind: handle.kind.clone(),
        });
    }
    serde_json::from_value(handle.payload.clone())
        .map_err(|e| ExporterError::BackendRejected(format!("corrupt comfy handle: {e}")))
}

fn rewrite_workflow_input(
    workflow: &mut Value,
    input_slot: &str,
    input_asset: &AssetCardDto,
) -> Result<(), ExporterError> {
    // Comfy's workflow shape: top-level object keyed by node id, each
    // value a `{ inputs, class_type }` pair. We patch
    // `<slot>.inputs.image` to the source_locator of the first
    // Selection member — the caller is responsible for making sure
    // Comfy can actually read that path (usually a bind mount into
    // Comfy's `input/` dir).
    let obj = workflow
        .as_object_mut()
        .ok_or_else(|| ExporterError::BackendRejected("workflow must be a JSON object".into()))?;
    let node = obj.get_mut(input_slot).ok_or_else(|| {
        ExporterError::BackendRejected(format!("input_slot {input_slot:?} not found in workflow"))
    })?;
    let inputs = node
        .get_mut("inputs")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| {
            ExporterError::BackendRejected(format!(
                "workflow node {input_slot:?} has no inputs object"
            ))
        })?;
    inputs.insert(
        "image".into(),
        Value::String(input_asset.source_locator.clone()),
    );
    Ok(())
}

#[async_trait]
impl Exporter for ComfyHttpExporter {
    fn slug(&self) -> &str {
        SLUG
    }

    fn accepts(&self, action: &str) -> bool {
        action == ACTION_IMG2IMG
    }

    async fn dispatch(&self, ctx: DispatchContext<'_>) -> Result<Handle, ExporterError> {
        if !self.accepts(ctx.action) {
            return Err(ExporterError::UnsupportedAction {
                exporter_slug: SLUG.into(),
                action: ctx.action.into(),
            });
        }
        let first_input = ctx.inputs.first().ok_or_else(|| {
            ExporterError::BackendRejected("comfy img2img needs at least one input".into())
        })?;
        let mut params = parse_params(&ctx)?;
        rewrite_workflow_input(&mut params.workflow, &params.input_slot, first_input)?;
        // ComfyUI's client id is arbitrary; using the dispatch id
        // keeps the WS backchannel (a later addition) natural.
        let body = serde_json::json!({
            "prompt": params.workflow,
            "client_id": ctx.dispatch_id,
        });
        let url = format!("{}/prompt", trim_trailing_slash(&params.endpoint));
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ExporterError::Other(anyhow::anyhow!("comfy POST /prompt: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ExporterError::BackendRejected(format!(
                "comfy POST /prompt HTTP {status}: {text}"
            )));
        }
        let body: Value = resp.json().await.map_err(|e| {
            ExporterError::BackendRejected(format!("comfy /prompt response not JSON: {e}"))
        })?;
        let prompt_id = body
            .get("prompt_id")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| {
                ExporterError::BackendRejected("comfy /prompt response missing prompt_id".into())
            })?;
        let payload = ComfyHandlePayload {
            prompt_id,
            endpoint: params.endpoint,
            output_dir: params.output_dir,
            poll_interval_ms: params.poll_interval_ms,
        };
        Ok(Handle::new(SLUG, serde_json::to_value(payload).unwrap()))
    }

    async fn poll(
        &self,
        _ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<DispatchState, ExporterError> {
        let payload = parse_handle_payload(handle)?;
        let url = format!(
            "{}/history/{}",
            trim_trailing_slash(&payload.endpoint),
            payload.prompt_id
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ExporterError::Other(anyhow::anyhow!("comfy GET /history: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ExporterError::BackendRejected(format!(
                "comfy GET /history HTTP {status}: {text}"
            )));
        }
        let body: Value = resp.json().await.map_err(|e| {
            ExporterError::BackendRejected(format!("comfy /history response not JSON: {e}"))
        })?;
        let entry = body.get(&payload.prompt_id);
        match entry {
            None => Ok(DispatchState::Running(ProgressHint {
                current: None,
                total: None,
                message: Some(format!(
                    "waiting for comfy prompt {}; next poll in {} ms",
                    payload.prompt_id, payload.poll_interval_ms
                )),
            })),
            Some(entry) => {
                // ComfyUI marks completion via `status.completed = true`
                // (or presence of `outputs`); a `status.error` field
                // signals failure.
                if let Some(err) = entry
                    .get("status")
                    .and_then(|s| s.get("error"))
                    .and_then(|e| e.as_str())
                {
                    return Ok(DispatchState::Failed {
                        message: err.into(),
                    });
                }
                let completed = entry
                    .get("status")
                    .and_then(|s| s.get("completed"))
                    .and_then(|c| c.as_bool())
                    .unwrap_or(false)
                    || entry.get("outputs").is_some();
                if completed {
                    Ok(DispatchState::Done)
                } else {
                    Ok(DispatchState::Running(ProgressHint {
                        current: None,
                        total: None,
                        message: Some("comfy still generating".into()),
                    }))
                }
            }
        }
    }

    async fn harvest(
        &self,
        ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<Vec<Derived>, ExporterError> {
        let payload = parse_handle_payload(handle)?;
        let url = format!(
            "{}/history/{}",
            trim_trailing_slash(&payload.endpoint),
            payload.prompt_id
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ExporterError::Other(anyhow::anyhow!("comfy GET /history: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ExporterError::BackendRejected(format!(
                "comfy GET /history HTTP {status}: {text}"
            )));
        }
        let body: Value = resp.json().await.map_err(|e| {
            ExporterError::BackendRejected(format!("comfy /history response not JSON: {e}"))
        })?;
        let entry = body.get(&payload.prompt_id).ok_or_else(|| {
            ExporterError::BackendRejected(format!(
                "comfy history has no entry for prompt_id {}",
                payload.prompt_id
            ))
        })?;
        // Walk `outputs.<node_id>.images[]`. Each image is
        // `{ filename, subfolder, type }`. We build the locator as
        // `<output_dir>/<subfolder>/<filename>` when output_dir is
        // supplied, otherwise as a Comfy `/view` URL — that URL is
        // still a valid locator string and downstream tooling can
        // decide whether to mirror the file locally.
        let outputs = entry
            .get("outputs")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let now = Utc::now();
        let dispatch_id = ctx.dispatch_id.to_string();
        let mut out: Vec<Derived> = Vec::new();
        for (node_id, node_val) in outputs {
            let Some(images) = node_val.get("images").and_then(|v| v.as_array()) else {
                continue;
            };
            for (idx, img) in images.iter().enumerate() {
                let filename = img
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("output.png");
                let subfolder = img
                    .get("subfolder")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let locator = match &payload.output_dir {
                    Some(dir) => {
                        if subfolder.is_empty() {
                            format!("{dir}/{filename}")
                        } else {
                            format!("{dir}/{subfolder}/{filename}")
                        }
                    }
                    None => {
                        let mut q = format!(
                            "{}/view?filename={filename}",
                            trim_trailing_slash(&payload.endpoint)
                        );
                        if !subfolder.is_empty() {
                            q.push_str(&format!("&subfolder={subfolder}"));
                        }
                        q
                    }
                };
                out.push(Derived {
                    modality: "image".into(),
                    locator,
                    occurred_at: now,
                    cover_hint: None,
                    register_note: None,
                    labels: vec![
                        format!("comfy:{}", ACTION_IMG2IMG),
                        format!("comfy_node:{}", node_id),
                    ],
                    file_size_bytes: None,
                    duration_ms: None,
                    extra: serde_json::json!({
                        "comfy": {
                            "prompt_id": payload.prompt_id,
                            "node_id": node_id,
                            "image_index": idx,
                            "filename": filename,
                            "subfolder": subfolder,
                        },
                        "dispatch_id": dispatch_id,
                    }),
                    batch_hint: None,
                });
            }
        }
        Ok(out)
    }
}

fn trim_trailing_slash(s: &str) -> &str {
    s.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card_fixture(source_locator: &str) -> AssetCardDto {
        AssetCardDto {
            id: "8a5b1c9d-7e0f-4a11-a2b3-c4d5e6f70801".into(),
            persona_id: "0a0000e5-4f01-70a1-9b0c-000000000001".into(),
            modality: Some("image".into()),
            mime: Some("image/png".into()),
            media: "image".into(),
            occurred_at_ms: 0,
            cover: None,
            labels: vec![],
            file_size_bytes: None,
            duration_ms: None,
            pixel_count: None,
            source_locator: source_locator.into(),
            group_ids: vec![],
            primary_group_position: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            rating: None,
            palette: None,
            has_note: false,
            has_thread: false,
            role: "item".into(),
            title: None,
            member_count: 0,
            score: None,
            snippet: None,
            author_kind: None,
            author_subject: None,
            operator_ai: None,
        }
    }

    #[test]
    fn slug_and_accepts_are_stable() {
        let exp = ComfyHttpExporter::new();
        assert_eq!(exp.slug(), SLUG);
        assert!(exp.accepts(ACTION_IMG2IMG));
        assert!(!exp.accepts("txt2img"));
        assert!(!exp.accepts(""));
    }

    #[test]
    fn rewrites_workflow_input_slot_image_field() {
        let mut workflow = serde_json::json!({
            "load_image": {
                "class_type": "LoadImage",
                "inputs": { "image": "placeholder" }
            }
        });
        let input = card_fixture("/tmp/photo.png");
        rewrite_workflow_input(&mut workflow, "load_image", &input).unwrap();
        assert_eq!(
            workflow["load_image"]["inputs"]["image"].as_str(),
            Some("/tmp/photo.png")
        );
    }

    /// The shipped example is what `asterism-server schema print
    /// exporter:comfy:params` hands a caller, so it has to survive the
    /// same `parse_params` the exporter runs — and its `input_slot`
    /// has to name a node the documented workflow actually contains,
    /// or a caller copying it verbatim would be rejected at dispatch.
    /// The sibling http example drifted out of its struct unnoticed
    /// (`harvest.item` / `locator_path`); this is the same guard on
    /// this side.
    #[test]
    fn params_example_deserialises_into_the_current_struct() {
        let params: ComfyDispatchParams = serde_json::from_str(params_example_json())
            .expect("schema/comfy_params.example.json must parse as ComfyDispatchParams");
        assert_eq!(params.endpoint, "http://127.0.0.1:8188");
        assert_eq!(params.input_slot, "10");
        assert_eq!(params.poll_interval_ms, 2000);

        let input = card_fixture("/tmp/photo.png");
        let mut workflow = params.workflow.clone();
        rewrite_workflow_input(&mut workflow, &params.input_slot, &input)
            .expect("example input_slot must exist in the example workflow");
        assert_eq!(
            workflow[params.input_slot.as_str()]["inputs"]["image"].as_str(),
            Some("/tmp/photo.png")
        );
    }

    #[test]
    fn handle_kind_mismatch_is_reported() {
        let mismatched = Handle::new("gemini", serde_json::json!({}));
        match parse_handle_payload(&mismatched) {
            Err(ExporterError::HandleMismatch {
                exporter_slug,
                handle_kind,
            }) => {
                assert_eq!(exporter_slug, SLUG);
                assert_eq!(handle_kind, "gemini");
            }
            other => panic!("expected HandleMismatch, got {other:?}"),
        }
    }
}
