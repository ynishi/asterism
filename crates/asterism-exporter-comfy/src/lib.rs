//! # asterism-exporter-comfy
//!
//! Asterism `Exporter` against a running ComfyUI — one dispatch goes
//! upload → prompt → poll → fetch → reify without anybody touching
//! ComfyUI's filesystem by hand.
//!
//! - Implements [`asterism_dispatch_sdk::Exporter`] over the ComfyUI
//!   HTTP API: `POST /upload/image` for the inputs, `POST /prompt` to
//!   queue the graph, `GET /history/{prompt_id}` to wait, `GET /view`
//!   to read the produced files back out.
//! - The Comfy workflow JSON is passed through `params.workflow` in
//!   the API format the frontend's *Export (API)* writes. Before it is
//!   submitted it is rendered through the shared `{{...}}` template
//!   ([`asterism_exporter_common::template`]), so a prompt, a seed or a
//!   batch count is a placeholder in the graph and a value in the
//!   params rather than a literal to edit in the node.
//! - Every input the graph names lands in ComfyUI's `input/` through
//!   the upload route, under a directory of this dispatch's own, and
//!   the `<subfolder>/<name>` the backend answers with is what the
//!   `LoadImage` node is given — ComfyUI resolves that pair against
//!   `input/`, and a bare name would name a file in another directory.
//!   Stock ComfyUI refuses a path outside `input/`, so the upload is
//!   the only way an image gets in.
//! - Every image a node emitted (`outputs.<node>.images[]`; other
//!   output kinds are left where they are) is fetched and written under
//!   the profile's custody root
//!   ([`asterism_exporter_common::CustodyPaths`]), and that path is the
//!   reified asset's locator. ComfyUI's own `output/`
//!   is not a place a locator can point: `temp/` is wiped on restart,
//!   file names are a counter that is reused once a file is deleted,
//!   and where the directory is at all is a flag on ComfyUI's command
//!   line that this process cannot see.
//!
//! Deferred: WebSocket progress, a saved-workflow registry, fanning a
//! snapshot's members through one graph each. The `Exporter` trait
//! leaves room for all three without touching this crate's surface.
//!
//! ## Params contract
//!
//! `CreateDispatchCommand.params_json` for this exporter deserialises
//! into [`ComfyDispatchParams`]:
//!
//! ```json
//! {
//!   "endpoint": "http://127.0.0.1:8188",
//!   "workflow": { /* ComfyUI prompt graph, API format */ },
//!   "input_slot": { "10": 0 },
//!   "prompt": "golden hour, same person",
//!   "count": 4,
//!   "poll_interval_ms": 2000
//! }
//! ```
//!
//! - `endpoint` — Comfy base URL (no trailing slash).
//! - `workflow` — the graph as the Comfy frontend would submit it. Any
//!   string leaf may be a template: `{{params.prompt}}`,
//!   `{{params.count}}`, `{{input[0].cover}}`, `{{dispatch_id}}`. A
//!   leaf that is *one* placeholder and nothing else keeps the type of
//!   the value it names, so `"amount": "{{params.count}}"` reaches
//!   the backend as the integer `4`.
//! - `input_slot` — which nodes take an image from the snapshot, as
//!   `{ node_id: input_index }`; the index is the position in the
//!   snapshot's member list. A bare `"10"` is accepted and means
//!   `{ "10": 0 }`. Optional: a txt2img graph names no node and uploads
//!   nothing.
//! - `seed` — read by `{{params.seed?}}`; left out, a random one is
//!   drawn per dispatch and written into the params the template sees,
//!   so a re-dispatch is a new sample and the attempt record shows
//!   which one.
//! - `poll_interval_ms` — echoed into the progress hint's message
//!   while the prompt is queued. The runner's own cadence is the job
//!   queue's; nothing reads this number back.
//!
//! Anything else in the blob is the caller's — `{{params.<key>}}`
//! reaches it. That is the same rule the http adapter has.
//!
//! ## What the produced PNG carries
//!
//! The submit sets `extra_data.extra_pnginfo.asterism` to the dispatch
//! id and the prompt id, and ComfyUI writes every key of
//! `extra_pnginfo` as a tEXt chunk of that name beside its own
//! `prompt`. A file that reaches the library by some other route —
//! dragged out of ComfyUI's output directory — still says which
//! dispatch made it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use asterism_contract::dto::AssetCardDto;
use asterism_dispatch_sdk::{
    AttemptRecord, Derived, DispatchContext, DispatchState, Exporter, ExporterError, Handle,
    ProgressHint,
};
use asterism_exporter_common::{CommonExportAdapter, CustodyPaths, TemplateAdapter, TemplateEnv};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Slug the registry uses for this exporter.
pub const SLUG: &str = "comfy";
/// The conventional action name for the graph the shipped example
/// describes. The exporter accepts any action — it is a label on the
/// derived assets, not a switch inside the graph.
pub const ACTION_IMG2IMG: &str = "img2img";

/// Public name for this exporter's params schema in the
/// `asterism-server schema` CLI (`exporter:comfy:params`).
pub const SCHEMA_NAME: &str = "exporter:comfy:params";

/// The subfolder under ComfyUI's `input/` every upload goes to, above
/// a directory per dispatch ([`upload_subfolder`]).
///
/// ComfyUI never prunes `input/`, so a name of our own keeps what this
/// process wrote apart from what the user dropped there by hand.
pub const UPLOAD_SUBFOLDER: &str = "asterism";

/// Where one dispatch's inputs go: `asterism/<dispatch_id>`.
///
/// A directory per dispatch, because the upload is named after the
/// asset's own file and two assets can share a basename — this
/// exporter's own outputs do, since custody names them by their
/// position in the harvest, so re-dispatching two dispatches' first
/// images as a reference and a mask would send `000-asterism_00001_.png`
/// twice. With `overwrite=true` the second upload would land on the
/// first and both nodes would load one image, with nothing to report.
/// It is not only a within-dispatch problem: `LoadImage` reads the
/// file when the prompt executes, so an upload from a later dispatch
/// could change what an already-queued one loads.
pub fn upload_subfolder(dispatch_id: &str) -> String {
    // The id is a UUID from our own ledger, so it is already one path
    // segment; ComfyUI validates the joined path stays under `input/`
    // regardless, and answers 400 if it does not.
    format!("{UPLOAD_SUBFOLDER}/{dispatch_id}")
}

/// Canonical example JSON for [`ComfyDispatchParams`] — streamed by
/// `asterism-server schema print exporter:comfy:params`.
pub fn params_example_json() -> &'static str {
    include_str!("../schema/comfy_params.example.json")
}

/// Which workflow nodes take an image from the snapshot.
///
/// Two spellings for one shape. The map is the shape: node id to the
/// index of the snapshot member that node loads. The bare string is
/// the spelling every params blob stored before the map existed uses,
/// and it means the first member — a stored profile is re-read on
/// every re-dispatch, so the old spelling has to keep resolving.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum InputSlots {
    /// One node, fed the first snapshot member.
    First(String),
    /// Node id → snapshot member index.
    Many(BTreeMap<String, usize>),
}

impl Default for InputSlots {
    fn default() -> Self {
        Self::Many(BTreeMap::new())
    }
}

impl InputSlots {
    /// The map, whichever spelling was written. An empty string is no
    /// slot at all — it is what the dispatch form's skeleton says
    /// before anyone types a node id, and a txt2img graph leaves it so.
    pub fn entries(&self) -> BTreeMap<String, usize> {
        match self {
            Self::First(node) if node.trim().is_empty() => BTreeMap::new(),
            Self::First(node) => BTreeMap::from([(node.clone(), 0)]),
            Self::Many(map) => map.clone(),
        }
    }
}

/// Params schema parsed from `params_json`.
#[derive(Debug, Clone, Deserialize)]
pub struct ComfyDispatchParams {
    /// Comfy base URL (e.g. `http://127.0.0.1:8188`, no trailing slash).
    pub endpoint: String,
    /// The ComfyUI prompt graph, API format, with `{{...}}` leaves
    /// where the caller wants a value substituted.
    pub workflow: Value,
    /// Which nodes' `image` input is fed from the snapshot, and by
    /// which member. See [`InputSlots`].
    #[serde(default)]
    pub input_slot: InputSlots,
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
    /// ComfyUI's prompt id — the one it answered `POST /prompt` with,
    /// which is the one it keys history by.
    prompt_id: String,
    /// Base URL to hit on subsequent polls.
    endpoint: String,
    /// Poll interval echoed into the progress hint's message.
    poll_interval_ms: u64,
    /// The seed the graph was sent with, when the graph took one from
    /// the params (`{{params.seed}}` somewhere in it); `None` for a
    /// graph that carries its own literal. The value a reader of the
    /// derived asset wants beside it.
    #[serde(default)]
    seed: Option<Value>,
}

/// HTTP-backed Exporter against a ComfyUI backend.
///
/// Cheap to `Clone` — the underlying reqwest client uses connection
/// pooling.
#[derive(Debug, Clone)]
pub struct ComfyHttpExporter {
    http: reqwest::Client,
    /// Where fetched outputs are written. Bound at construction by the
    /// composition root, never read out of the params — see
    /// [`CustodyPaths::new`] for why.
    custody: CustodyPaths,
    grammar: CommonExportAdapter,
}

impl ComfyHttpExporter {
    /// Builds an exporter with a default reqwest client that takes
    /// custody of produced files under `custody_root` (the application
    /// directory).
    pub fn new(custody_root: PathBuf) -> Self {
        Self::with_client(
            custody_root,
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest client build"),
        )
    }

    /// Uses a caller-supplied reqwest client (integration tests
    /// substitute one pointed at a fake).
    pub fn with_client(custody_root: PathBuf, http: reqwest::Client) -> Self {
        Self {
            http,
            custody: CustodyPaths::new(custody_root),
            grammar: CommonExportAdapter,
        }
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

/// The params the template resolves against: the caller's blob, plus a
/// `seed` when the caller left it out.
///
/// Drawn here rather than left to the backend because the backend does
/// not draw one — `control_after_generate` is a frontend widget, and a
/// graph submitted twice with the same literal seed is answered from
/// cache with no new image. Written into the params the template sees,
/// so `{{params.seed?}}` resolves to it and the attempt record shows
/// the graph as sent.
fn params_with_seed(raw: &Value) -> Value {
    let mut params = raw.clone();
    if let Some(obj) = params.as_object_mut()
        && obj.get("seed").is_none_or(|s| s.is_null())
    {
        // 53 bits: the largest integer every JSON reader on the way —
        // the ledger's own, the backend's — carries without rounding.
        let seed: u64 = rand::random::<u64>() >> 11;
        obj.insert("seed".into(), Value::from(seed));
    }
    params
}

/// A leaf that is exactly one placeholder, and whether it was optional.
fn sole_placeholder(s: &str) -> Option<(&str, bool)> {
    let inner = s.strip_prefix("{{")?.strip_suffix("}}")?.trim();
    if inner.is_empty() || inner.contains("{{") || inner.contains("}}") {
        return None;
    }
    Some(match inner.strip_suffix('?') {
        Some(key) => (key.trim_end(), true),
        None => (inner, false),
    })
}

/// Whether any leaf of the graph names the params' seed.
fn workflow_reads_seed(workflow: &Value) -> bool {
    match workflow {
        Value::String(s) => s.contains("{{params.seed"),
        Value::Array(arr) => arr.iter().any(workflow_reads_seed),
        Value::Object(obj) => obj.values().any(workflow_reads_seed),
        _ => false,
    }
}

/// Renders the graph's string leaves through the template.
///
/// The one departure from [`TemplateAdapter::render_json`]: a leaf that
/// is a single placeholder takes the *value* the placeholder names, so
/// a number stays a number. ComfyUI validates `INT` and `FLOAT` inputs
/// by type before it runs anything, and the `prompt` chunk it writes
/// into the PNG is the graph as submitted — a seed sent as `"123"`
/// would be a string in both places.
fn render_workflow(
    grammar: &CommonExportAdapter,
    value: &Value,
    env: &TemplateEnv<'_>,
) -> Result<Value, ExporterError> {
    match value {
        Value::String(s) => match sole_placeholder(s) {
            Some((key, optional)) => match env.value(key) {
                // A key that is there but null is what it is for
                // `render`: nothing, spelled as the empty string.
                Some(Value::Null) => Ok(Value::String(String::new())),
                Some(v) => Ok(v),
                None if optional => Ok(Value::String(String::new())),
                None => Err(ExporterError::BackendRejected(format!(
                    "template placeholder {{{{{key}}}}} did not resolve"
                ))),
            },
            None => Ok(Value::String(grammar.render(s, env)?)),
        },
        Value::Array(arr) => arr
            .iter()
            .map(|v| render_workflow(grammar, v, env))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(obj) => {
            let mut out = serde_json::Map::with_capacity(obj.len());
            for (k, v) in obj {
                out.insert(k.clone(), render_workflow(grammar, v, env)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

/// Points one node's `image` input at a file ComfyUI can see.
fn set_workflow_image(
    workflow: &mut Value,
    node_id: &str,
    image_name: &str,
) -> Result<(), ExporterError> {
    // Comfy's workflow shape: top-level object keyed by node id, each
    // value a `{ inputs, class_type }` pair.
    let obj = workflow
        .as_object_mut()
        .ok_or_else(|| ExporterError::BackendRejected("workflow must be a JSON object".into()))?;
    let node = obj.get_mut(node_id).ok_or_else(|| {
        ExporterError::BackendRejected(format!("input_slot {node_id:?} not found in workflow"))
    })?;
    let inputs = node
        .get_mut("inputs")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| {
            ExporterError::BackendRejected(format!(
                "workflow node {node_id:?} has no inputs object"
            ))
        })?;
    inputs.insert("image".into(), Value::String(image_name.into()));
    Ok(())
}

/// What `POST /upload/image` answered: the name the file has inside
/// ComfyUI, which is what a `LoadImage` node is given.
///
/// Read from the response rather than rebuilt from the request. The
/// upload asks to overwrite, so today the name comes back as sent —
/// but the subfolder the backend settled on and any renaming a version
/// applies are in its answer, not in what was asked.
#[derive(Debug, Clone, Deserialize)]
struct UploadResponse {
    name: String,
    #[serde(default)]
    subfolder: String,
}

impl UploadResponse {
    /// The `image` input value: `subfolder/name`, or the bare name.
    fn image_ref(&self) -> String {
        if self.subfolder.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.subfolder, self.name)
        }
    }
}

/// One upload, as the attempt record remembers it.
#[derive(Debug, Clone, Serialize)]
struct UploadRecord {
    node_id: String,
    input_index: usize,
    source_locator: String,
    image: String,
}

fn upload_error(what: &str, err: impl std::fmt::Display) -> ExporterError {
    ExporterError::Other(anyhow::anyhow!("comfy POST /upload/image: {what}: {err}"))
}

impl ComfyHttpExporter {
    /// Puts one snapshot member into ComfyUI's `input/` and returns the
    /// reference the graph should use for it.
    async fn upload_input(
        &self,
        endpoint: &str,
        subfolder: &str,
        input: &AssetCardDto,
    ) -> Result<UploadResponse, ExporterError> {
        let path = PathBuf::from(&input.source_locator);
        let bytes = tokio::fs::read(&path).await.map_err(|e| {
            ExporterError::BackendRejected(format!(
                "input asset {} is not a readable file at {}: {e}",
                input.id, input.source_locator
            ))
        })?;
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("{}.png", input.id));
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str(input.mime.as_deref().unwrap_or("application/octet-stream"))
            .map_err(|e| upload_error("mime", e))?;
        let form = reqwest::multipart::Form::new()
            .part("image", part)
            .text("overwrite", "true")
            .text("type", "input")
            .text("subfolder", subfolder.to_string());
        let url = format!("{}/upload/image", trim_trailing_slash(endpoint));
        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| upload_error("send", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ExporterError::BackendRejected(format!(
                "comfy POST /upload/image HTTP {status}: {text}"
            )));
        }
        resp.json::<UploadResponse>().await.map_err(|e| {
            ExporterError::BackendRejected(format!("comfy /upload/image response not JSON: {e}"))
        })
    }

    /// One `GET /history/{prompt_id}`, as both poll and harvest read it.
    async fn history_entry(
        &self,
        payload: &ComfyHandlePayload,
    ) -> Result<Option<Value>, ExporterError> {
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
        Ok(body.get(&payload.prompt_id).cloned())
    }
}

/// The URL ComfyUI serves one produced file from.
fn view_url(endpoint: &str, filename: &str, subfolder: &str, kind: &str) -> String {
    let mut url = format!(
        "{}/view?filename={}",
        trim_trailing_slash(endpoint),
        urlencode(filename)
    );
    if !subfolder.is_empty() {
        url.push_str(&format!("&subfolder={}", urlencode(subfolder)));
    }
    url.push_str(&format!("&type={}", urlencode(kind)));
    url
}

/// Percent-encodes a query value. File names ComfyUI mints are plain,
/// but `filename_prefix` is the caller's and may carry anything.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// How a finished history entry ended, read the way ComfyUI writes it.
///
/// `status.status_str` is `"success"` or `"error"`, `status.completed`
/// says whether every output node ran, and `status.messages` is the
/// event log — an `execution_error` entry carries the exception. An
/// entry with no `status` block at all is a ComfyUI from before the
/// block existed, and for that one the presence of `outputs` is the
/// only signal there is.
enum HistoryVerdict {
    Done,
    Failed(String),
    Running,
}

fn read_verdict(entry: &Value) -> HistoryVerdict {
    let Some(status) = entry.get("status") else {
        return if entry.get("outputs").is_some() {
            HistoryVerdict::Done
        } else {
            HistoryVerdict::Running
        };
    };
    let status_str = status.get("status_str").and_then(|s| s.as_str());
    if status_str == Some("error") {
        return HistoryVerdict::Failed(execution_error_message(status));
    }
    let completed = status
        .get("completed")
        .and_then(|c| c.as_bool())
        .unwrap_or(false);
    if completed || status_str == Some("success") {
        HistoryVerdict::Done
    } else {
        HistoryVerdict::Running
    }
}

/// The exception text out of `status.messages`, or the best fallback
/// the entry offers.
fn execution_error_message(status: &Value) -> String {
    let from_messages = status
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|msgs| {
            msgs.iter().find_map(|m| {
                let kind = m.get(0)?.as_str()?;
                if kind != "execution_error" {
                    return None;
                }
                let data = m.get(1)?;
                let message = data.get("exception_message")?.as_str()?;
                Some(match data.get("node_type").and_then(|n| n.as_str()) {
                    Some(node_type) => format!("{node_type}: {message}"),
                    None => message.to_string(),
                })
            })
        });
    from_messages
        .or_else(|| {
            status
                .get("error")
                .and_then(|e| e.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| "comfy reported an error".into())
}

#[async_trait]
impl Exporter for ComfyHttpExporter {
    fn slug(&self) -> &str {
        SLUG
    }

    fn accepts(&self, action: &str) -> bool {
        // The graph decides what the run does; the action is the label
        // the derived assets carry. Any name is a name.
        !action.trim().is_empty()
    }

    async fn dispatch(&self, ctx: DispatchContext<'_>) -> Result<Handle, ExporterError> {
        if !self.accepts(ctx.action) {
            return Err(ExporterError::UnsupportedAction {
                exporter_slug: SLUG.into(),
                action: ctx.action.into(),
            });
        }
        let params = parse_params(&ctx)?;
        let template_params = params_with_seed(ctx.params);
        let env = TemplateEnv::pre_handle(&ctx, &template_params);
        let mut workflow = render_workflow(&self.grammar, &params.workflow, &env)?;
        // Template first, uploads second: the name the backend answers
        // with is data, and must not be read as a template.
        let subfolder = upload_subfolder(ctx.dispatch_id);
        let mut uploads: Vec<UploadRecord> = Vec::new();
        for (node_id, index) in params.input_slot.entries() {
            let input = ctx.inputs.get(index).ok_or_else(|| {
                ExporterError::BackendRejected(format!(
                    "input_slot {node_id:?} asks for snapshot member {index}, but the snapshot has {}",
                    ctx.inputs.len()
                ))
            })?;
            let uploaded = self
                .upload_input(&params.endpoint, &subfolder, input)
                .await?;
            let image = uploaded.image_ref();
            set_workflow_image(&mut workflow, &node_id, &image)?;
            uploads.push(UploadRecord {
                node_id,
                input_index: index,
                source_locator: input.source_locator.clone(),
                image,
            });
        }
        // Only a graph that took its seed from the params has a seed to
        // report; a frontend export carries a literal in the sampler,
        // and the drawn one never left this function.
        let seed = if workflow_reads_seed(&params.workflow) {
            template_params.get("seed").cloned()
        } else {
            None
        };
        // ComfyUI's client id is arbitrary; using the dispatch id keeps
        // the WS backchannel (a later addition) natural. The prompt id
        // is ours to choose as well, and choosing it means the PNG can
        // carry it before the backend has answered.
        let prompt_id = uuid::Uuid::now_v7().to_string();
        let body = serde_json::json!({
            "prompt": workflow,
            "client_id": ctx.dispatch_id,
            "prompt_id": prompt_id,
            "extra_data": {
                "extra_pnginfo": {
                    "asterism": {
                        "dispatch_id": ctx.dispatch_id,
                        "prompt_id": prompt_id,
                    }
                }
            },
        });
        let url = format!("{}/prompt", trim_trailing_slash(&params.endpoint));
        let sent = self.http.post(&url).json(&body).send().await;
        // Recorded on both arms: the refused submit is the case a
        // reader has the most questions about, and it is the one that
        // leaves no handle behind.
        let (status, answer) = match &sent {
            Ok(resp) => (Some(resp.status().as_u16()), None),
            Err(err) => (None, Some(err.to_string())),
        };
        let mut attempt = serde_json::json!({
            "uploads": uploads,
            "submit": { "url": url, "body": body, "status": status, "error": answer },
        });
        let resp = match sent {
            Ok(resp) => resp,
            Err(e) => {
                ctx.attempt.record(AttemptRecord::new(SLUG, attempt));
                return Err(ExporterError::Other(anyhow::anyhow!(
                    "comfy POST /prompt: {e}"
                )));
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            attempt["submit"]["response"] = Value::String(text.clone());
            ctx.attempt.record(AttemptRecord::new(SLUG, attempt));
            return Err(ExporterError::BackendRejected(format!(
                "comfy POST /prompt HTTP {status}: {text}"
            )));
        }
        let answer: Value = resp.json().await.map_err(|e| {
            ExporterError::BackendRejected(format!("comfy /prompt response not JSON: {e}"))
        })?;
        attempt["submit"]["response"] = answer.clone();
        ctx.attempt.record(AttemptRecord::new(SLUG, attempt));
        // The backend's word wins over ours: history is keyed by what it
        // answered, and a backend that ignored the supplied id would
        // otherwise be polled for a prompt it never queued.
        let prompt_id = answer
            .get("prompt_id")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| {
                ExporterError::BackendRejected("comfy /prompt response missing prompt_id".into())
            })?;
        let payload = ComfyHandlePayload {
            prompt_id,
            endpoint: params.endpoint,
            poll_interval_ms: params.poll_interval_ms,
            seed,
        };
        Ok(Handle::new(SLUG, serde_json::to_value(payload).unwrap()))
    }

    async fn poll(
        &self,
        _ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<DispatchState, ExporterError> {
        let payload = parse_handle_payload(handle)?;
        match self.history_entry(&payload).await? {
            None => Ok(DispatchState::Running(ProgressHint {
                current: None,
                total: None,
                message: Some(format!(
                    "waiting for comfy prompt {}; next poll in {} ms",
                    payload.prompt_id, payload.poll_interval_ms
                )),
            })),
            Some(entry) => Ok(match read_verdict(&entry) {
                HistoryVerdict::Done => DispatchState::Done,
                HistoryVerdict::Failed(message) => DispatchState::Failed { message },
                HistoryVerdict::Running => DispatchState::Running(ProgressHint {
                    current: None,
                    total: None,
                    message: Some("comfy still generating".into()),
                }),
            }),
        }
    }

    async fn harvest(
        &self,
        ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<Vec<Derived>, ExporterError> {
        let payload = parse_handle_payload(handle)?;
        let entry = self.history_entry(&payload).await?.ok_or_else(|| {
            ExporterError::BackendRejected(format!(
                "comfy history has no entry for prompt_id {}",
                payload.prompt_id
            ))
        })?;
        // Walk `outputs.<node_id>.images[]`. Each image is
        // `{ filename, subfolder, type }`; each is fetched through
        // `/view` and written into custody, and the custody path is the
        // locator. The `/view` parameters ride along as provenance.
        let outputs = entry
            .get("outputs")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let now = Utc::now();
        let dispatch_id = ctx.dispatch_id.to_string();
        let mut out: Vec<Derived> = Vec::new();
        // One counter across every node: the custody name is
        // `<nnn>-<filename>`, and two nodes may well emit the same
        // filename.
        let mut index = 0usize;
        for (node_id, node_val) in outputs {
            let Some(images) = node_val.get("images").and_then(|v| v.as_array()) else {
                continue;
            };
            for (image_index, img) in images.iter().enumerate() {
                let filename = img
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("output.png");
                let subfolder = img
                    .get("subfolder")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let kind = img.get("type").and_then(|v| v.as_str()).unwrap_or("output");
                let url = view_url(&payload.endpoint, filename, subfolder, kind);
                let resp =
                    self.http.get(&url).send().await.map_err(|e| {
                        ExporterError::Other(anyhow::anyhow!("comfy GET /view: {e}"))
                    })?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    return Err(ExporterError::BackendRejected(format!(
                        "comfy GET /view HTTP {status} for {filename}"
                    )));
                }
                let bytes = resp.bytes().await.map_err(|e| {
                    ExporterError::Other(anyhow::anyhow!("comfy GET /view body: {e}"))
                })?;
                let written = self
                    .custody
                    .write(ctx.dispatch_id, index, filename, &bytes)
                    .await?;
                index += 1;
                out.push(Derived {
                    modality: "image".into(),
                    locator: written.to_string_lossy().into_owned(),
                    occurred_at: now,
                    cover_hint: None,
                    register_note: None,
                    labels: vec![
                        format!("comfy:{}", ctx.action),
                        format!("comfy_node:{}", node_id),
                    ],
                    file_size_bytes: Some(bytes.len() as u64),
                    duration_ms: None,
                    extra: serde_json::json!({
                        "comfy": {
                            "prompt_id": payload.prompt_id,
                            "node_id": node_id,
                            "image_index": image_index,
                            "filename": filename,
                            "subfolder": subfolder,
                            "type": kind,
                            "view_url": url,
                            "seed": payload.seed,
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
            occurred_source: "unknown".into(),
            time_zone: None,
            cover: Some("a plate, off-white".into()),
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
            found_by: None,
            author_kind: None,
            author_subject: None,
            operator_ai: None,
        }
    }

    fn ctx<'a>(inputs: &'a [AssetCardDto], params: &'a Value) -> DispatchContext<'a> {
        DispatchContext {
            inputs,
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            persona_id: "persona-1",
            action: "img2img",
            params,
            attempt: &asterism_dispatch_sdk::DISCARD_ATTEMPTS,
        }
    }

    fn exporter() -> ComfyHttpExporter {
        ComfyHttpExporter::new(std::env::temp_dir().join("asterism-comfy-unit"))
    }

    #[test]
    fn slug_is_stable_and_any_named_action_is_accepted() {
        let exp = exporter();
        assert_eq!(exp.slug(), SLUG);
        assert!(exp.accepts(ACTION_IMG2IMG));
        assert!(exp.accepts("txt2img"));
        assert!(exp.accepts("upscale"));
        assert!(!exp.accepts(""));
        assert!(!exp.accepts("   "));
    }

    #[test]
    fn sets_the_named_node_image_input() {
        let mut workflow = serde_json::json!({
            "load_image": {
                "class_type": "LoadImage",
                "inputs": { "image": "placeholder" }
            }
        });
        set_workflow_image(&mut workflow, "load_image", "asterism/photo.png").unwrap();
        assert_eq!(
            workflow["load_image"]["inputs"]["image"].as_str(),
            Some("asterism/photo.png")
        );
        let missing = set_workflow_image(&mut workflow, "nope", "x").unwrap_err();
        assert!(matches!(missing, ExporterError::BackendRejected(_)));
    }

    /// Both spellings of `input_slot` resolve to the same map, and the
    /// old bare-string one still means "first member" — a stored
    /// params blob is re-read on every re-dispatch.
    #[test]
    fn input_slot_accepts_the_old_string_and_the_map() {
        let old: ComfyDispatchParams = serde_json::from_value(serde_json::json!({
            "endpoint": "http://x", "workflow": {}, "input_slot": "10"
        }))
        .unwrap();
        assert_eq!(
            old.input_slot.entries(),
            BTreeMap::from([("10".to_string(), 0)])
        );

        let new: ComfyDispatchParams = serde_json::from_value(serde_json::json!({
            "endpoint": "http://x", "workflow": {}, "input_slot": { "10": 0, "11": 1 }
        }))
        .unwrap();
        assert_eq!(
            new.input_slot.entries(),
            BTreeMap::from([("10".to_string(), 0), ("11".to_string(), 1)])
        );

        let none: ComfyDispatchParams = serde_json::from_value(serde_json::json!({
            "endpoint": "http://x", "workflow": {}
        }))
        .unwrap();
        assert!(
            none.input_slot.entries().is_empty(),
            "a txt2img graph names no node"
        );

        let blank: ComfyDispatchParams = serde_json::from_value(serde_json::json!({
            "endpoint": "http://x", "workflow": {}, "input_slot": ""
        }))
        .unwrap();
        assert!(
            blank.input_slot.entries().is_empty(),
            "the form's untouched skeleton is the same as naming nothing"
        );
    }

    /// A placeholder that is the whole leaf keeps its value's type; one
    /// embedded in text is text; an optional one that is missing is the
    /// empty string; a required one that is missing is a rejection.
    #[test]
    fn workflow_leaves_render_with_their_types() {
        let inputs = [card_fixture("/tmp/photo.png")];
        let params = serde_json::json!({
            "endpoint": "http://x",
            "prompt": "golden hour",
            "count": 4,
            "negative": null,
            "workflow": {}
        });
        let ctx = ctx(&inputs, &params);
        let env = TemplateEnv::pre_handle(&ctx, &params);
        let workflow = serde_json::json!({
            "6": { "inputs": { "text": "{{params.prompt}}, {{input[0].cover}}" } },
            "12": { "inputs": { "amount": "{{params.count}}" } },
            "9": { "inputs": { "filename_prefix": "asterism/{{dispatch_id}}" } },
            "7": { "inputs": { "text": "{{params.negative}}" } },
            "3": { "inputs": { "seed": "{{params.seed?}}", "steps": 30 } }
        });
        let rendered = render_workflow(&CommonExportAdapter, &workflow, &env).unwrap();
        assert_eq!(
            rendered["6"]["inputs"]["text"],
            "golden hour, a plate, off-white"
        );
        assert_eq!(
            rendered["12"]["inputs"]["amount"], 4,
            "a number stays a number"
        );
        assert_eq!(
            rendered["9"]["inputs"]["filename_prefix"],
            "asterism/disp-1"
        );
        assert_eq!(
            rendered["3"]["inputs"]["seed"], "",
            "optional and absent: empty"
        );
        assert_eq!(
            rendered["7"]["inputs"]["text"], "",
            "present but null: empty, as `render` has it"
        );
        assert_eq!(
            rendered["3"]["inputs"]["steps"], 30,
            "a literal rides through"
        );

        let required = serde_json::json!({ "3": { "inputs": { "seed": "{{params.seed}}" } } });
        let err = render_workflow(&CommonExportAdapter, &required, &env).unwrap_err();
        assert!(
            matches!(&err, ExporterError::BackendRejected(m) if m.contains("params.seed")),
            "{err:?}"
        );
    }

    /// A graph that carries its own literal seed never took the drawn
    /// one, so there is none to report beside its outputs.
    #[test]
    fn only_a_graph_that_reads_the_params_seed_has_one_to_report() {
        let literal = serde_json::json!({ "3": { "inputs": { "seed": 12345 } } });
        assert!(!workflow_reads_seed(&literal));
        let optional = serde_json::json!({ "3": { "inputs": { "seed": "{{params.seed?}}" } } });
        assert!(workflow_reads_seed(&optional));
        let nested = serde_json::json!({ "3": { "inputs": { "seed": ["{{params.seed}}"] } } });
        assert!(workflow_reads_seed(&nested));
    }

    #[test]
    fn a_missing_seed_is_drawn_and_a_given_one_is_kept() {
        let given = params_with_seed(&serde_json::json!({ "seed": 7 }));
        assert_eq!(given["seed"], 7);
        let drawn = params_with_seed(&serde_json::json!({ "prompt": "x" }));
        let seed = drawn["seed"].as_u64().expect("a drawn seed is an integer");
        assert!(seed < (1u64 << 53), "fits every JSON reader on the way");
        let null = params_with_seed(&serde_json::json!({ "seed": null }));
        assert!(
            null["seed"].is_u64(),
            "an explicit null is the same as absent"
        );
    }

    #[test]
    fn history_verdict_reads_the_status_block_the_backend_writes() {
        let ok = serde_json::json!({
            "status": { "status_str": "success", "completed": true, "messages": [] },
            "outputs": {}
        });
        assert!(matches!(read_verdict(&ok), HistoryVerdict::Done));

        // An error with partial outputs is still an error.
        let failed = serde_json::json!({
            "status": {
                "status_str": "error", "completed": false,
                "messages": [
                    ["execution_start", { "prompt_id": "p" }],
                    ["execution_error", {
                        "node_id": "3", "node_type": "KSampler",
                        "exception_message": "CUDA out of memory"
                    }]
                ]
            },
            "outputs": { "9": { "images": [] } }
        });
        match read_verdict(&failed) {
            HistoryVerdict::Failed(m) => assert_eq!(m, "KSampler: CUDA out of memory"),
            _ => panic!("an error status is a failure"),
        }

        let legacy = serde_json::json!({ "outputs": { "9": {} } });
        assert!(matches!(read_verdict(&legacy), HistoryVerdict::Done));
        let nothing = serde_json::json!({ "prompt": [] });
        assert!(matches!(read_verdict(&nothing), HistoryVerdict::Running));
    }

    #[test]
    fn view_url_carries_every_parameter_the_backend_keys_on() {
        assert_eq!(
            view_url("http://h:1/", "a b.png", "", "output"),
            "http://h:1/view?filename=a%20b.png&type=output"
        );
        assert_eq!(
            view_url("http://h:1", "x.png", "batch", "temp"),
            "http://h:1/view?filename=x.png&subfolder=batch&type=temp"
        );
    }

    /// The shipped example is what `asterism-server schema print
    /// exporter:comfy:params` hands a caller, so it has to survive the
    /// same `parse_params` the exporter runs — and its `input_slot`
    /// has to name a node the documented workflow actually contains,
    /// or a caller copying it verbatim would be rejected at dispatch.
    #[test]
    fn params_example_deserialises_into_the_current_struct() {
        let params: ComfyDispatchParams = serde_json::from_str(params_example_json())
            .expect("schema/comfy_params.example.json must parse as ComfyDispatchParams");
        assert_eq!(params.endpoint, "http://127.0.0.1:8188");
        assert_eq!(
            params.input_slot.entries(),
            BTreeMap::from([("10".to_string(), 0)])
        );
        assert_eq!(params.poll_interval_ms, 2000);

        let raw: Value = serde_json::from_str(params_example_json()).unwrap();
        let inputs = [card_fixture("/tmp/photo.png")];
        let ctx = ctx(&inputs, &raw);
        let with_seed = params_with_seed(&raw);
        let env = TemplateEnv::pre_handle(&ctx, &with_seed);
        let mut workflow = render_workflow(&CommonExportAdapter, &params.workflow, &env)
            .expect("every placeholder the example uses resolves against the example params");
        assert!(
            workflow["3"]["inputs"]["seed"].is_u64(),
            "the seed the example leaves blank is drawn"
        );
        assert_eq!(workflow["6"]["inputs"]["text"], raw["prompt"]);
        for (node, _) in params.input_slot.entries() {
            set_workflow_image(&mut workflow, &node, "asterism/photo.png")
                .expect("example input_slot must exist in the example workflow");
        }
        assert_eq!(
            workflow["10"]["inputs"]["image"].as_str(),
            Some("asterism/photo.png")
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
