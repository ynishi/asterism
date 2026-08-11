//! # asterism-exporter-http
//!
//! Schema-driven HTTP exporter. Where [`asterism_exporter_comfy`] hard-
//! codes the ComfyUI protocol, this crate stays backend-agnostic:
//! **the caller supplies the request / poll / harvest shapes as JSON
//! schema in the dispatch params**. One deployable adapter, N backends.
//!
//! Position in the workspace: mirror of `asterism-importer-sqlite`
//! ("query + column map = whole importer") on the OUT side. Where the
//! SQLite importer's caller writes SQL + a column mapping, this
//! exporter's caller writes an HTTP shape + a JSON-path mapping.
//!
//! ## Params schema
//!
//! `CreateDispatchCommand.params_json` deserialises into
//! [`HttpDispatchParams`]. All three phases (`dispatch` /
//! `poll` / `harvest`) are configured up front so the runner can
//! drive the state machine without re-reading params on every tick.
//!
//! ```json
//! {
//!   "endpoint": "http://backend.example.com",
//!
//!   "dispatch": {
//!     "method": "POST",
//!     "path": "/generate",
//!     "headers": { "authorization": "Bearer {{params.extras.api_key}}" },
//!     "body_template": {
//!       "input_url": "{{input[0].source_locator}}",
//!       "prompt":    "{{params.extras.prompt}}",
//!       "client_id": "{{dispatch_id}}"
//!     },
//!     "handle_from": "$.job_id"
//!   },
//!
//!   "poll": {
//!     "method": "GET",
//!     "path":   "/status/{{handle}}",
//!     "done_when":   { "path": "$.status", "equals": "done" },
//!     "failed_when": { "path": "$.status", "equals": "failed",
//!                       "message_path": "$.error" },
//!     "progress_message_path": "$.status_message"
//!   },
//!
//!   "harvest": {
//!     "method": "GET",
//!     "path": "/result/{{handle}}",
//!     "items_path": "$.outputs[*]",
//!     "map": {
//!       "modality":      "image",
//!       "locator":       "{{item.url}}",
//!       "cover_hint":    "{{item.caption?}}",
//!       "labels_static": ["batch:{{dispatch_id}}"]
//!     }
//!   },
//!
//!   "extras": {
//!     "api_key": "put-your-token-here",
//!     "prompt":  "photo studio portrait"
//!   }
//! }
//! ```
//!
//! `extras` is not a field the exporter knows about — the params blob
//! is its own template namespace, so a caller nests its per-backend
//! values anywhere it likes and reaches them with
//! `{{params.<dot.path>}}`. `schema/http_params.example.json` is the
//! runnable version of this same shape (it is what `asterism-server
//! schema print exporter:http:params` streams), and the tests at the
//! bottom of this file are what keep it honest.
//!
//! Note that `handle_from` decides what `{{handle}}` resolves to.
//! With `"$.job_id"` the handle *is* the id string, so the poll path
//! interpolates `{{handle}}`; a `handle_from` of `"$"` keeps the whole
//! response body and the path would read `{{handle.job_id}}` instead.
//!
//! ### Template placeholders
//!
//! Simple `{{...}}` substitution, no arithmetic. Supported roots:
//!
//! - `{{selection_id}}`, `{{dispatch_id}}`, `{{persona_id}}`,
//!   `{{action}}` — dispatch-context ids.
//! - `{{input[N].<field>}}` — indexed input asset field. Supported
//!   fields: `id`, `persona_id`, `source_locator`, `source_kind`,
//!   `modality`, `cover`.
//! - `{{params.<dot.path>}}` — deep dot-access into the params JSON
//!   itself (so the caller can define its own "extra fields" section
//!   in params and reference it from templates).
//! - `{{handle.<dot.path>}}` — deep dot-access into the handle JSON.
//!   Only available in `poll` / `harvest` templates (the exporter
//!   panics on this in `dispatch`, when no handle exists yet).
//! - `{{item.<dot.path>}}` — dot-access into the current
//!   `harvest.items_path` element. Only available inside
//!   `harvest.map`.
//!
//! A trailing `?` on a placeholder (`{{item.caption?}}`) means
//! "resolve to empty string when the path is missing" instead of
//! failing with `BackendRejected`.
//!
//! Params are persisted unedited. The blob handed to
//! `CreateDispatchCommand` is stored whole as
//! `dispatch_job.params_json` and handed back out as
//! `DispatchDto.params_json` on every read of the dispatch — nothing
//! on that path filters, redacts, or drops a field. A credential
//! reached by `{{params.…}}` (the `extras.api_key` above) is
//! therefore readable by anything that can list dispatches; put one
//! there only where that visibility is acceptable.
//!
//! ### JSONPath
//!
//! Minimal subset — enough to steer the state machine and pluck out
//! items:
//!
//! - `$.foo`             — object field.
//! - `$.foo.bar`         — dot chain.
//! - `$.arr[0]`          — array index.
//! - `$.arr[*]`          — array wildcard (only the last segment can
//!   be a wildcard; matches the shape of every documented example).
//!
//! Anything outside this grammar is rejected up front with
//! `BackendRejected`.

use std::collections::BTreeMap;

use asterism_contract::dto::AssetCardDto;
use asterism_dispatch_sdk::{
    Derived, DispatchContext, DispatchState, Exporter, ExporterError, Handle, ProgressHint,
};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Slug the registry uses for this exporter.
pub const SLUG: &str = "http";

/// Public name for this exporter's params schema in the
/// `asterism-server schema` CLI (`exporter:http:params`).
pub const SCHEMA_NAME: &str = "exporter:http:params";

/// Canonical example JSON for [`HttpDispatchParams`] — streamed by
/// `asterism-server schema print exporter:http:params`.
pub fn params_example_json() -> &'static str {
    include_str!("../schema/http_params.example.json")
}

/// Params schema for [`SLUG`] dispatch calls.
#[derive(Debug, Clone, Deserialize)]
pub struct HttpDispatchParams {
    /// Base URL of the backend (no trailing slash needed).
    pub endpoint: String,
    /// How to submit the job.
    pub dispatch: DispatchSchema,
    /// How to check on the job.
    pub poll: PollSchema,
    /// How to collect results once the job is done.
    pub harvest: HarvestSchema,
}

/// Schema for the `POST /prompt`-shaped submit call.
#[derive(Debug, Clone, Deserialize)]
pub struct DispatchSchema {
    /// HTTP method (`"POST"` / `"PUT"` / …). Defaults to `POST`.
    #[serde(default = "default_post")]
    pub method: String,
    /// URL path appended to `endpoint` (`/generate`, `/prompt`, …).
    /// Template substitution applies.
    pub path: String,
    /// Optional header dict. Values are template-substituted.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// JSON body template. Placeholders are recursively substituted
    /// at every string leaf.
    pub body_template: Value,
    /// JSONPath into the response used to build the persisted
    /// [`Handle`]. Extracts a JSON value that survives restarts.
    /// Defaults to the whole response body (`$`).
    #[serde(default = "default_dollar")]
    pub handle_from: String,
}

/// Schema for the `GET /status/{id}`-shaped poll call.
#[derive(Debug, Clone, Deserialize)]
pub struct PollSchema {
    /// HTTP method. Defaults to `GET`.
    #[serde(default = "default_get")]
    pub method: String,
    /// URL path (template-substituted; typically references
    /// `{{handle.…}}`).
    pub path: String,
    /// Optional headers (template-substituted).
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Predicate — the state is Done when this evaluates truthy.
    pub done_when: MatchRule,
    /// Predicate — the state is Failed when this evaluates truthy.
    /// Checked before `done_when` so a `failed` marker wins over a
    /// concurrent `outputs` field.
    pub failed_when: MatchRule,
    /// Optional JSONPath to a human-readable progress message
    /// echoed on the Running state.
    #[serde(default)]
    pub progress_message_path: Option<String>,
}

/// Schema for the `GET /result/{id}`-shaped harvest call.
#[derive(Debug, Clone, Deserialize)]
pub struct HarvestSchema {
    /// HTTP method. Defaults to `GET`.
    #[serde(default = "default_get")]
    pub method: String,
    /// URL path (template-substituted).
    pub path: String,
    /// Optional headers (template-substituted).
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// JSONPath to an array of items in the response. One [`Derived`]
    /// is emitted per element.
    pub items_path: String,
    /// Per-item mapping. Every field is optional except `locator`.
    pub map: HarvestMap,
}

/// How the harvest phase turns each response item into a
/// [`Derived`].
#[derive(Debug, Clone, Deserialize)]
pub struct HarvestMap {
    /// Modality slug template (or static string). Defaults to
    /// `"image"`.
    #[serde(default = "default_image")]
    pub modality: String,
    /// Locator template. **Required** — the resulting Derived has to
    /// point at something the reified Asset can read from.
    pub locator: String,
    /// Optional cover hint template.
    #[serde(default)]
    pub cover_hint: Option<String>,
    /// Optional register note template.
    #[serde(default)]
    pub register_note: Option<String>,
    /// Static label templates (each substituted independently).
    #[serde(default)]
    pub labels_static: Vec<String>,
    /// Optional JSONPath to an array of labels *inside the item*.
    /// Appended to `labels_static` after per-string template
    /// resolution.
    #[serde(default)]
    pub labels_path: Option<String>,
}

/// Predicate against a poll-response value.
///
/// Only equality is supported today. Numeric range checks would
/// widen the grammar without materially expanding coverage.
#[derive(Debug, Clone, Deserialize)]
pub struct MatchRule {
    /// JSONPath into the response document.
    pub path: String,
    /// Value the path must equal for the rule to fire. Strings and
    /// primitives round-trip verbatim.
    pub equals: Value,
    /// Optional JSONPath to a human-readable message extracted when
    /// the rule fires (used to populate the Failed state's message,
    /// or ignored on Done).
    #[serde(default)]
    pub message_path: Option<String>,
}

fn default_post() -> String {
    "POST".into()
}
fn default_get() -> String {
    "GET".into()
}
fn default_dollar() -> String {
    "$".into()
}
fn default_image() -> String {
    "image".into()
}

/// Payload persisted on the returned [`Handle`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HttpHandlePayload {
    /// Value extracted via `dispatch.handle_from` — the exporter uses
    /// this to fill the `{{handle.…}}` placeholder on subsequent
    /// polls.
    handle: Value,
}

/// HTTP-backed schema-driven [`Exporter`].
#[derive(Debug, Clone)]
pub struct HttpExporter {
    http: reqwest::Client,
}

impl Default for HttpExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpExporter {
    /// Builds an exporter with a default `reqwest::Client`.
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest client build"),
        }
    }

    /// Uses a caller-supplied `reqwest::Client` (integration tests
    /// pass a mocked one).
    pub fn with_client(http: reqwest::Client) -> Self {
        Self { http }
    }
}

#[async_trait]
impl Exporter for HttpExporter {
    fn slug(&self) -> &str {
        SLUG
    }

    fn accepts(&self, _action: &str) -> bool {
        // Actions are opaque to a schema-driven exporter — every
        // action fits, as long as the params carry a valid schema.
        // The core still records the action string on the DispatchJob
        // row, so callers keep audit-trail granularity.
        true
    }

    async fn dispatch(&self, ctx: DispatchContext<'_>) -> Result<Handle, ExporterError> {
        let params = parse_params(&ctx)?;
        let base = trim_trailing_slash(&params.endpoint);
        let path = render_template(
            &params.dispatch.path,
            &TemplateEnv::pre_handle(&ctx, ctx.params),
        )?;
        let url = format!("{base}{path}");
        let body = substitute_json_leaves(
            &params.dispatch.body_template,
            &TemplateEnv::pre_handle(&ctx, ctx.params),
        )?;
        let headers = render_headers(
            &params.dispatch.headers,
            &TemplateEnv::pre_handle(&ctx, ctx.params),
        )?;
        let resp = send_request(
            &self.http,
            &params.dispatch.method,
            &url,
            &headers,
            Some(body),
        )
        .await?;
        let handle_value =
            jsonpath_first(&resp, &params.dispatch.handle_from).ok_or_else(|| {
                ExporterError::BackendRejected(format!(
                    "dispatch response missing handle_from path {:?}",
                    params.dispatch.handle_from
                ))
            })?;
        Ok(Handle::new(
            SLUG,
            serde_json::to_value(HttpHandlePayload {
                handle: handle_value,
            })
            .unwrap(),
        ))
    }

    async fn poll(
        &self,
        ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<DispatchState, ExporterError> {
        check_kind(handle)?;
        let params = parse_params(&ctx)?;
        let payload = parse_handle_payload(handle)?;
        let env = TemplateEnv::with_handle(&ctx, ctx.params, &payload.handle);
        let base = trim_trailing_slash(&params.endpoint);
        let path = render_template(&params.poll.path, &env)?;
        let url = format!("{base}{path}");
        let headers = render_headers(&params.poll.headers, &env)?;
        let resp = send_request(&self.http, &params.poll.method, &url, &headers, None).await?;

        if match_rule(&params.poll.failed_when, &resp) {
            let message = params
                .poll
                .failed_when
                .message_path
                .as_deref()
                .and_then(|p| jsonpath_first(&resp, p))
                .and_then(value_to_display_string)
                .unwrap_or_else(|| "backend reported failure".into());
            return Ok(DispatchState::Failed { message });
        }
        if match_rule(&params.poll.done_when, &resp) {
            return Ok(DispatchState::Done);
        }
        let message = params
            .poll
            .progress_message_path
            .as_deref()
            .and_then(|p| jsonpath_first(&resp, p))
            .and_then(value_to_display_string);
        Ok(DispatchState::Running(ProgressHint {
            current: None,
            total: None,
            message,
        }))
    }

    async fn harvest(
        &self,
        ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<Vec<Derived>, ExporterError> {
        check_kind(handle)?;
        let params = parse_params(&ctx)?;
        let payload = parse_handle_payload(handle)?;
        let env = TemplateEnv::with_handle(&ctx, ctx.params, &payload.handle);
        let base = trim_trailing_slash(&params.endpoint);
        let path = render_template(&params.harvest.path, &env)?;
        let url = format!("{base}{path}");
        let headers = render_headers(&params.harvest.headers, &env)?;
        let resp = send_request(&self.http, &params.harvest.method, &url, &headers, None).await?;
        let items = jsonpath_many(&resp, &params.harvest.items_path);
        let now = Utc::now();
        let mut out: Vec<Derived> = Vec::with_capacity(items.len());
        for item in items {
            let item_env = env.with_item(&item);
            let modality = render_template(&params.harvest.map.modality, &item_env)?;
            let locator = render_template(&params.harvest.map.locator, &item_env)?;
            let cover_hint = params
                .harvest
                .map
                .cover_hint
                .as_deref()
                .map(|t| render_template(t, &item_env))
                .transpose()?;
            let register_note = params
                .harvest
                .map
                .register_note
                .as_deref()
                .map(|t| render_template(t, &item_env))
                .transpose()?;
            let mut labels: Vec<String> = Vec::new();
            for tmpl in &params.harvest.map.labels_static {
                labels.push(render_template(tmpl, &item_env)?);
            }
            if let Some(path) = &params.harvest.map.labels_path {
                for v in jsonpath_many(
                    &item,
                    path.trim_start_matches("$.item").trim_start_matches("$"),
                ) {
                    if let Some(s) = value_to_display_string(v) {
                        labels.push(s);
                    }
                }
            }
            out.push(Derived {
                modality,
                locator,
                occurred_at: now,
                cover_hint,
                register_note,
                labels,
                file_size_bytes: None,
                duration_ms: None,
                extra: serde_json::json!({ "http": { "item": item } }),
                batch_hint: None,
            });
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

fn parse_params(ctx: &DispatchContext<'_>) -> Result<HttpDispatchParams, ExporterError> {
    serde_json::from_value(ctx.params.clone())
        .map_err(|e| ExporterError::BackendRejected(format!("invalid http params: {e}")))
}

fn parse_handle_payload(handle: &Handle) -> Result<HttpHandlePayload, ExporterError> {
    serde_json::from_value(handle.payload.clone())
        .map_err(|e| ExporterError::BackendRejected(format!("corrupt http handle: {e}")))
}

fn check_kind(handle: &Handle) -> Result<(), ExporterError> {
    if handle.kind != SLUG {
        return Err(ExporterError::HandleMismatch {
            exporter_slug: SLUG.into(),
            handle_kind: handle.kind.clone(),
        });
    }
    Ok(())
}

fn trim_trailing_slash(s: &str) -> &str {
    s.trim_end_matches('/')
}

async fn send_request(
    http: &reqwest::Client,
    method: &str,
    url: &str,
    headers: &BTreeMap<String, String>,
    body: Option<Value>,
) -> Result<Value, ExporterError> {
    let http_method = match method.to_ascii_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
        "DELETE" => reqwest::Method::DELETE,
        other => {
            return Err(ExporterError::BackendRejected(format!(
                "unsupported HTTP method: {other:?}"
            )));
        }
    };
    let mut req = http.request(http_method, url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| ExporterError::Other(anyhow::anyhow!("http {method} {url}: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ExporterError::BackendRejected(format!(
            "http {method} {url} HTTP {status}: {text}"
        )));
    }
    resp.json::<Value>().await.map_err(|e| {
        ExporterError::BackendRejected(format!("http {method} {url} response not JSON: {e}"))
    })
}

fn render_headers(
    headers: &BTreeMap<String, String>,
    env: &TemplateEnv<'_>,
) -> Result<BTreeMap<String, String>, ExporterError> {
    let mut out = BTreeMap::new();
    for (k, v) in headers {
        out.insert(k.clone(), render_template(v, env)?);
    }
    Ok(out)
}

fn value_to_display_string(v: Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(_) | Value::Object(_) => Some(v.to_string()),
    }
}

fn match_rule(rule: &MatchRule, resp: &Value) -> bool {
    match jsonpath_first(resp, &rule.path) {
        Some(actual) => actual == rule.equals,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Template substitution — {{...}}.
// ---------------------------------------------------------------------------

/// Environment used to resolve `{{...}}` placeholders during
/// template rendering. Kept opaque to callers.
struct TemplateEnv<'a> {
    ctx: &'a DispatchContext<'a>,
    params: &'a Value,
    handle: Option<&'a Value>,
    item: Option<&'a Value>,
}

impl<'a> TemplateEnv<'a> {
    fn pre_handle(ctx: &'a DispatchContext<'a>, params: &'a Value) -> Self {
        Self {
            ctx,
            params,
            handle: None,
            item: None,
        }
    }

    fn with_handle(ctx: &'a DispatchContext<'a>, params: &'a Value, handle: &'a Value) -> Self {
        Self {
            ctx,
            params,
            handle: Some(handle),
            item: None,
        }
    }

    fn with_item<'b: 'a>(&'b self, item: &'a Value) -> TemplateEnv<'a> {
        TemplateEnv {
            ctx: self.ctx,
            params: self.params,
            handle: self.handle,
            item: Some(item),
        }
    }

    fn resolve(&self, key: &str, optional: bool) -> Result<String, ExporterError> {
        let value = self.lookup(key);
        match value {
            Some(v) => Ok(value_to_display_string(v).unwrap_or_default()),
            None if optional => Ok(String::new()),
            None => Err(ExporterError::BackendRejected(format!(
                "template placeholder {{{{{key}}}}} did not resolve"
            ))),
        }
    }

    fn lookup(&self, key: &str) -> Option<Value> {
        // Top-level context ids.
        match key {
            "selection_id" => return Some(Value::String(self.ctx.selection_id.into())),
            "dispatch_id" => return Some(Value::String(self.ctx.dispatch_id.into())),
            "persona_id" => return Some(Value::String(self.ctx.persona_id.into())),
            "action" => return Some(Value::String(self.ctx.action.into())),
            _ => {}
        }
        // input[N].<field>
        if let Some(rest) = key.strip_prefix("input[")
            && let Some(bracket_end) = rest.find(']')
        {
            let (idx_str, tail) = rest.split_at(bracket_end);
            let tail = &tail[1..]; // skip `]`
            let idx: usize = idx_str.parse().ok()?;
            let input = self.ctx.inputs.get(idx)?;
            let field = tail.strip_prefix('.').unwrap_or(tail);
            return input_field(input, field);
        }
        // params.<dot.path>
        if let Some(rest) = key.strip_prefix("params.") {
            return dot_path(self.params, rest);
        }
        if key == "params" {
            return Some(self.params.clone());
        }
        // handle.<dot.path>
        if let Some(rest) = key.strip_prefix("handle.") {
            return self.handle.and_then(|h| dot_path(h, rest));
        }
        if key == "handle" {
            return self.handle.cloned();
        }
        // item.<dot.path>
        if let Some(rest) = key.strip_prefix("item.") {
            return self.item.and_then(|i| dot_path(i, rest));
        }
        if key == "item" {
            return self.item.cloned();
        }
        None
    }
}

fn input_field(input: &AssetCardDto, field: &str) -> Option<Value> {
    match field {
        "" | "id" => Some(Value::String(input.id.clone())),
        "persona_id" => Some(Value::String(input.persona_id.clone())),
        "source_locator" => Some(Value::String(input.source_locator.clone())),
        // Unclassified inputs (asset-model v4) template as "".
        "modality" => Some(Value::String(input.modality.clone().unwrap_or_default())),
        "cover" => input.cover.clone().map(Value::String),
        _ => None,
    }
}

fn dot_path(root: &Value, path: &str) -> Option<Value> {
    let mut cur = root;
    for seg in path.split('.') {
        // Support `arr[0]` inside the dot chain.
        if let Some(open) = seg.find('[') {
            let field = &seg[..open];
            let rest = &seg[open + 1..];
            let close = rest.find(']')?;
            let idx: usize = rest[..close].parse().ok()?;
            let after_bracket = &rest[close + 1..];
            if !field.is_empty() {
                cur = cur.get(field)?;
            }
            cur = cur.get(idx)?;
            if !after_bracket.is_empty() {
                // Additional trailing chain like `[0].name` — recurse
                // on the remainder starting after the `].` we just
                // consumed.
                return dot_path(cur, after_bracket.trim_start_matches('.'));
            }
        } else {
            cur = cur.get(seg)?;
        }
    }
    Some(cur.clone())
}

fn render_template(template: &str, env: &TemplateEnv<'_>) -> Result<String, ExporterError> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    // Byte offset where the current literal run started. Literals are
    // copied as `&str` slices rather than byte-by-byte so multi-byte
    // UTF-8 sequences survive intact. Every byte of a multi-byte
    // sequence is >= 0x80 — lead bytes (0xC2-0xF4) as well as
    // continuation bytes (0x80-0xBF) — so none can equal `b'{'`, and
    // `i` is a char boundary whenever a placeholder opens.
    // `literal_start` is a boundary for the same reason: it is either 0
    // or the offset just past a `}}`, both of which sit outside any
    // multi-byte sequence.
    let mut literal_start = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            out.push_str(&template[literal_start..i]);
            let end = template[i + 2..].find("}}").ok_or_else(|| {
                ExporterError::BackendRejected(format!(
                    "unterminated template placeholder starting at byte {i}"
                ))
            })?;
            let raw = &template[i + 2..i + 2 + end];
            let key = raw.trim();
            let (real_key, optional) = match key.strip_suffix('?') {
                Some(k) => (k.trim(), true),
                None => (key, false),
            };
            out.push_str(&env.resolve(real_key, optional)?);
            i += 2 + end + 2;
            literal_start = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&template[literal_start..]);
    Ok(out)
}

fn substitute_json_leaves(value: &Value, env: &TemplateEnv<'_>) -> Result<Value, ExporterError> {
    match value {
        Value::String(s) => Ok(Value::String(render_template(s, env)?)),
        Value::Array(arr) => arr
            .iter()
            .map(|v| substitute_json_leaves(v, env))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(obj) => {
            let mut out = Map::with_capacity(obj.len());
            for (k, v) in obj {
                out.insert(k.clone(), substitute_json_leaves(v, env)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

// ---------------------------------------------------------------------------
// Minimal JSONPath — $.foo.bar[0] and $.foo.bar[*]
// ---------------------------------------------------------------------------

fn parse_jsonpath(expr: &str) -> Vec<PathSeg> {
    let mut segs: Vec<PathSeg> = Vec::new();
    let expr = expr.trim();
    let expr = expr.strip_prefix('$').unwrap_or(expr);
    let expr = expr.strip_prefix('.').unwrap_or(expr);
    for raw in expr.split('.') {
        if raw.is_empty() {
            continue;
        }
        // Possible forms: `foo`, `foo[0]`, `foo[*]`, `[0]`, `[*]`
        let (field, index_part) = match raw.find('[') {
            None => (raw, ""),
            Some(open) => (&raw[..open], &raw[open..]),
        };
        if !field.is_empty() {
            segs.push(PathSeg::Field(field.to_string()));
        }
        if !index_part.is_empty() {
            let inner = index_part.trim_start_matches('[').trim_end_matches(']');
            if inner == "*" {
                segs.push(PathSeg::Wildcard);
            } else if let Ok(idx) = inner.parse::<usize>() {
                segs.push(PathSeg::Index(idx));
            } else {
                // Unsupported index form — leave as a literal field
                // fallback so grammar errors are visible.
                segs.push(PathSeg::Field(inner.to_string()));
            }
        }
    }
    segs
}

enum PathSeg {
    Field(String),
    Index(usize),
    Wildcard,
}

fn jsonpath_many(root: &Value, expr: &str) -> Vec<Value> {
    let segs = parse_jsonpath(expr);
    let mut frontier: Vec<&Value> = vec![root];
    for seg in &segs {
        let mut next: Vec<&Value> = Vec::new();
        for v in &frontier {
            match seg {
                PathSeg::Field(name) => {
                    if let Some(child) = v.get(name) {
                        next.push(child);
                    }
                }
                PathSeg::Index(idx) => {
                    if let Some(child) = v.get(*idx) {
                        next.push(child);
                    }
                }
                PathSeg::Wildcard => match v {
                    Value::Array(arr) => next.extend(arr.iter()),
                    Value::Object(obj) => next.extend(obj.values()),
                    _ => {}
                },
            }
        }
        frontier = next;
    }
    frontier.into_iter().cloned().collect()
}

fn jsonpath_first(root: &Value, expr: &str) -> Option<Value> {
    jsonpath_many(root, expr).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card_fixture(id: &str, source_locator: &str) -> AssetCardDto {
        AssetCardDto {
            id: id.into(),
            persona_id: "persona-1".into(),
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

    fn stub_ctx<'a>(inputs: &'a [AssetCardDto], params: &'a Value) -> DispatchContext<'a> {
        DispatchContext {
            inputs,
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            persona_id: "persona-1",
            action: "run",
            params,
        }
    }

    #[test]
    fn slug_is_stable_and_action_open() {
        let exp = HttpExporter::new();
        assert_eq!(exp.slug(), SLUG);
        assert!(exp.accepts("run"));
        assert!(exp.accepts("any-slug"));
    }

    #[test]
    fn render_template_substitutes_context_and_params() {
        let params = serde_json::json!({ "prompt": "cats" });
        let input = card_fixture("asset-uuid", "/tmp/photo.png");
        let inputs = vec![input];
        let ctx = DispatchContext {
            inputs: &inputs,
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            persona_id: "persona-1",
            action: "run",
            params: &params,
        };
        let env = TemplateEnv::pre_handle(&ctx, ctx.params);
        assert_eq!(
            render_template(
                "{{selection_id}}/{{input[0].source_locator}}?p={{params.prompt}}",
                &env
            )
            .unwrap(),
            "sel-1//tmp/photo.png?p=cats"
        );
        // Optional placeholder silently resolves to empty string.
        assert_eq!(render_template("{{params.missing?}}", &env).unwrap(), "");
        // Non-optional missing placeholder errors.
        assert!(render_template("{{params.missing}}", &env).is_err());
    }

    #[test]
    fn render_template_keeps_non_ascii_literals_free_of_mojibake() {
        let params = serde_json::json!({ "extras": { "prompt": "夜の街" } });
        let inputs = vec![card_fixture("asset-uuid", "/tmp/写真.png")];
        let ctx = stub_ctx(&inputs, &params);
        let env = TemplateEnv::pre_handle(&ctx, ctx.params);
        // Multi-byte literals sit between the placeholders, and the
        // placeholders themselves open at byte 0 and close at the end —
        // exercising the empty leading slice and the empty trailing
        // slice on top of the multi-byte runs. A byte-wise copy would
        // widen each literal byte into its own Latin-1 char.
        assert_eq!(
            render_template(
                "{{params.extras.prompt}}、対象 {{input[0].source_locator}} — 完了{{selection_id}}",
                &env
            )
            .unwrap(),
            "夜の街、対象 /tmp/写真.png — 完了sel-1"
        );
        // Template that is nothing but a non-ASCII literal.
        assert_eq!(
            render_template("絵文字🎨も壊さない", &env).unwrap(),
            "絵文字🎨も壊さない"
        );
    }

    #[test]
    fn render_template_unterminated_offset_is_a_byte_index() {
        let params = serde_json::json!({ "extras": { "prompt": "夜の街" } });
        let inputs = vec![card_fixture("asset-uuid", "/tmp/写真.png")];
        let ctx = stub_ctx(&inputs, &params);
        let env = TemplateEnv::pre_handle(&ctx, ctx.params);
        // `夜` is 3 bytes, so the unterminated `{{` opens at byte 3 —
        // the reported offset indexes bytes, not chars.
        let err = render_template("夜{{a", &env).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("byte 3"), "unexpected message: {msg}");
    }

    #[test]
    fn jsonpath_first_and_many_navigate_the_documented_subset() {
        let doc = serde_json::json!({
            "status": "done",
            "outputs": [
                {"url": "https://a", "tags": ["cat"]},
                {"url": "https://b", "tags": ["dog"]}
            ]
        });
        assert_eq!(
            jsonpath_first(&doc, "$.status"),
            Some(Value::String("done".into()))
        );
        assert_eq!(
            jsonpath_first(&doc, "$.outputs[0].url"),
            Some(Value::String("https://a".into()))
        );
        assert_eq!(jsonpath_many(&doc, "$.outputs[*]").len(), 2);
    }

    #[test]
    fn match_rule_fires_only_on_equal_value() {
        let doc = serde_json::json!({ "status": "done" });
        let rule = MatchRule {
            path: "$.status".into(),
            equals: Value::String("done".into()),
            message_path: None,
        };
        assert!(match_rule(&rule, &doc));
        let rule_neg = MatchRule {
            path: "$.status".into(),
            equals: Value::String("failed".into()),
            message_path: None,
        };
        assert!(!match_rule(&rule_neg, &doc));
    }

    #[test]
    fn substitute_json_leaves_walks_nested_shapes() {
        let params = serde_json::json!({ "prompt": "cats" });
        let input = card_fixture("a", "/x");
        let inputs = vec![input];
        let ctx = DispatchContext {
            inputs: &inputs,
            selection_id: "s",
            dispatch_id: "d",
            persona_id: "p",
            action: "run",
            params: &params,
        };
        let env = TemplateEnv::pre_handle(&ctx, ctx.params);
        let body = serde_json::json!({
            "top": "{{params.prompt}}",
            "arr": ["{{input[0].source_locator}}", 42, true],
            "nested": { "field": "{{selection_id}}" }
        });
        let out = substitute_json_leaves(&body, &env).unwrap();
        assert_eq!(out["top"], "cats");
        assert_eq!(out["arr"][0], "/x");
        assert_eq!(out["arr"][1], 42);
        assert_eq!(out["arr"][2], true);
        assert_eq!(out["nested"]["field"], "s");
    }

    /// The shipped example is what `asterism-server schema print
    /// exporter:http:params` hands a caller, so it has to survive the
    /// same `parse_params` the exporter runs. It did not: the file
    /// drifted to `harvest.item` / `locator_path` while the struct
    /// moved to `harvest.map` / `locator`, and nothing failed.
    #[test]
    fn params_example_deserialises_into_the_current_struct() {
        let params: HttpDispatchParams = serde_json::from_str(params_example_json())
            .expect("schema/http_params.example.json must parse as HttpDispatchParams");
        assert_eq!(params.endpoint, "https://backend.example.com");
        assert_eq!(params.dispatch.handle_from, "$.job_id");
        assert_eq!(params.harvest.items_path, "$.outputs[*]");
        assert_eq!(params.harvest.map.locator, "{{item.url}}");
    }

    /// Deserialising the example only proves its *field names* are
    /// current — every template is an opaque `String` to serde. The
    /// other half of the same drift lived inside those strings
    /// (`{{inputs[0]}}` for `{{input[0]}}`, an `{{env.*}}` root that
    /// never existed), so the example is rendered here through the
    /// three phases the runner drives.
    #[test]
    fn params_example_templates_resolve_in_every_phase() {
        let raw: Value = serde_json::from_str(params_example_json()).unwrap();
        let params: HttpDispatchParams = serde_json::from_value(raw.clone()).unwrap();
        let inputs = vec![card_fixture("asset-uuid", "/tmp/photo.png")];
        let ctx = stub_ctx(&inputs, &raw);

        // dispatch — no handle exists yet.
        let pre = TemplateEnv::pre_handle(&ctx, ctx.params);
        render_template(&params.dispatch.path, &pre).unwrap();
        render_headers(&params.dispatch.headers, &pre).unwrap();
        let body = substitute_json_leaves(&params.dispatch.body_template, &pre).unwrap();
        assert_eq!(body["input_url"], "/tmp/photo.png");
        assert_eq!(body["prompt"], "photo studio portrait");
        assert_eq!(body["client_id"], "disp-1");

        // poll / harvest — the handle is in play.
        let handle = Value::String("job-1".into());
        let env = TemplateEnv::with_handle(&ctx, ctx.params, &handle);
        assert_eq!(
            render_template(&params.poll.path, &env).unwrap(),
            "/jobs/job-1"
        );
        render_headers(&params.poll.headers, &env).unwrap();
        render_template(&params.harvest.path, &env).unwrap();
        render_headers(&params.harvest.headers, &env).unwrap();

        // harvest.map — the only place `{{item.…}}` resolves.
        let item = serde_json::json!({ "url": "https://renders.test/a.png" });
        let item_env = env.with_item(&item);
        assert_eq!(
            render_template(&params.harvest.map.locator, &item_env).unwrap(),
            "https://renders.test/a.png"
        );
        render_template(&params.harvest.map.modality, &item_env).unwrap();
        // The example's cover hint is optional (`?`); this item has no
        // caption, so it has to resolve to empty rather than error.
        let cover = params.harvest.map.cover_hint.as_deref().unwrap();
        assert_eq!(render_template(cover, &item_env).unwrap(), "");
        let labels: Vec<String> = params
            .harvest
            .map
            .labels_static
            .iter()
            .map(|t| render_template(t, &item_env).unwrap())
            .collect();
        // The runner prepends `exporter:{slug}` to every Derived's
        // label list and does not dedupe
        // (`dispatch_runner_service.rs`), so an example that ships
        // `exporter:http` in `labels_static` teaches callers to put a
        // duplicate chip on every produced card.
        assert_eq!(labels, vec!["batch:disp-1".to_string()]);
    }

    /// Deserialising the example proves its field names, and rendering
    /// it proves its `{{...}}` templates. The third string family in
    /// the file is JSONPath, and nothing had ever evaluated one — a
    /// `$.ouputs[*]` typo would harvest zero items against a backend
    /// doing everything right, silently. Every path the example ships
    /// is evaluated here through the same `jsonpath_first` /
    /// `jsonpath_many` the exporter calls.
    #[test]
    fn params_example_jsonpaths_resolve_against_a_representative_response() {
        let params: HttpDispatchParams = serde_json::from_str(params_example_json()).unwrap();
        // One document standing in for all three phases: the fields
        // the paths reach are disjoint, so a single response exercises
        // every path without pretending the backend returns this shape
        // on every call.
        let resp = serde_json::json!({
            "job_id": "job-1",
            "status": "succeeded",
            "error": "upstream refused the workflow",
            "progress_message": "step 12/30",
            "outputs": [
                { "url": "https://renders.test/a.png" },
                { "url": "https://renders.test/b.png" }
            ]
        });

        // dispatch.handle_from — what gets persisted on the Handle and
        // interpolated into every later path.
        assert_eq!(
            jsonpath_first(&resp, &params.dispatch.handle_from),
            Some(Value::String("job-1".into()))
        );

        // poll.done_when / failed_when — both sides of the state
        // machine read the same field and differ only in `equals`.
        assert_eq!(
            jsonpath_first(&resp, &params.poll.done_when.path),
            Some(Value::String("succeeded".into()))
        );
        assert!(match_rule(&params.poll.done_when, &resp));
        assert_eq!(
            jsonpath_first(&resp, &params.poll.failed_when.path),
            Some(Value::String("succeeded".into()))
        );
        assert!(!match_rule(&params.poll.failed_when, &resp));
        let message_path = params
            .poll
            .failed_when
            .message_path
            .as_deref()
            .expect("the example ships a failure message path");
        assert_eq!(
            jsonpath_first(&resp, message_path),
            Some(Value::String("upstream refused the workflow".into()))
        );

        // poll.progress_message_path — echoed on the Running state.
        let progress_path = params
            .poll
            .progress_message_path
            .as_deref()
            .expect("the example ships a progress path");
        assert_eq!(
            jsonpath_first(&resp, progress_path),
            Some(Value::String("step 12/30".into()))
        );

        // harvest.items_path — one Derived per element, and the map's
        // locator has to resolve against each of them.
        let items = jsonpath_many(&resp, &params.harvest.items_path);
        assert_eq!(items.len(), 2, "the wildcard has to reach every output");

        let raw: Value = serde_json::from_str(params_example_json()).unwrap();
        let inputs = vec![card_fixture("asset-uuid", "/tmp/photo.png")];
        let ctx = stub_ctx(&inputs, &raw);
        let handle = Value::String("job-1".into());
        let env = TemplateEnv::with_handle(&ctx, ctx.params, &handle);
        let mut locators: Vec<String> = Vec::new();
        for item in &items {
            let item_env = env.with_item(item);
            locators.push(render_template(&params.harvest.map.locator, &item_env).unwrap());
        }
        assert_eq!(
            locators,
            vec![
                "https://renders.test/a.png".to_string(),
                "https://renders.test/b.png".to_string(),
            ]
        );
    }

    /// The example's `method` strings are matched by `send_request` at
    /// call time, so a typo there stays invisible until a live backend
    /// call comes back as `BackendRejected`. Compared against the
    /// accepted set as a literal — re-deriving the match arms here
    /// would just copy the typo.
    #[test]
    fn params_example_methods_are_in_the_accepted_set() {
        const ACCEPTED: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
        let params: HttpDispatchParams = serde_json::from_str(params_example_json()).unwrap();
        for (phase, method) in [
            ("dispatch", &params.dispatch.method),
            ("poll", &params.poll.method),
            ("harvest", &params.harvest.method),
        ] {
            assert!(
                ACCEPTED.contains(&method.to_ascii_uppercase().as_str()),
                "{phase}.method {method:?} is not one of {ACCEPTED:?}"
            );
        }
    }
}
