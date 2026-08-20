//! # asterism-exporter-http
//!
//! Schema-driven exporter for **HTTP job APIs**. Where
//! [`asterism_exporter_comfy`] hard-codes the ComfyUI protocol, this
//! crate stays backend-agnostic: **the caller supplies the submit / poll
//! / harvest shapes as JSON schema in the dispatch params**. One
//! deployable adapter, N backends.
//!
//! Position in the workspace: mirror of `asterism-importer-sqlite`
//! ("query + column map = whole importer") on the OUT side. Where the
//! SQLite importer's caller writes SQL + a column mapping, this
//! exporter's caller writes an HTTP shape + a JSON-path mapping.
//!
//! ## Why there is no second adapter for hosted platforms
//!
//! There was one, for a while: `asterism-exporter-cloud`, carrying its
//! own copy of this schema — the same poll predicates, the same harvest
//! item map, the same handle — because a hosted platform needed three
//! things this adapter had not grown yet. It was the wrong axis. A
//! hosted platform and a self-hosted backend speak the same job API:
//! submit, keep a handle, poll, collect. Whether the URL is `https`,
//! whether the credential comes from the environment, and how long to
//! keep waiting are configuration, not adapter identity, and splitting
//! on them bought duplication while leaving this side stuck with the
//! weaker credential story it had deferred.
//!
//! So the three arrived here as optional blocks, and "cloud" became a
//! profile:
//!
//! | block | absent | present |
//! |---|---|---|
//! | [`auth`](AuthSchema) | the profile carries no credential, or reaches one through its own params | the credential is named by environment variable and never persisted |
//! | [`fetch`](FetchSchema) | the backend's own URL is the locator | the bytes are pulled into custody first, and *that path* is the locator |
//! | `deadline_seconds` | poll until the backend answers | a job past its deadline fails as expired |
//!
//! The distinction worth keeping is a different one: a backend reachable
//! as an HTTP job API — which a profile covers — versus a backend that
//! ships an SDK, which will not be a profile at all.
//!
//! ## Params schema
//!
//! `CreateDispatchCommand.params_json` deserialises into
//! [`HttpDispatchParams`]. All three phases (`submit` / `poll` /
//! `harvest`) are configured up front so the runner can drive the state
//! machine without re-reading params on every tick.
//!
//! ```json
//! {
//!   "endpoint": "http://backend.example.com",
//!
//!   "submit": {
//!     "method": "POST",
//!     "path": "/generate",
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
//!       "source_url":    "{{item.url}}",
//!       "cover_hint":    "{{item.caption?}}",
//!       "labels_static": ["batch:{{dispatch_id}}"]
//!     }
//!   },
//!
//!   "auth": {
//!     "secret_ref":     "BACKEND_KEY",
//!     "header":         "authorization",
//!     "value_template": "Bearer {{secret}}"
//!   },
//!   "fetch": { "authenticated": false },
//!   "deadline_seconds": 86400,
//!
//!   "extras": {
//!     "prompt": "photo studio portrait"
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
//! ### Templates and JSONPath
//!
//! Both grammars are the shared adapter machinery, documented where
//! they are defined: [`asterism_exporter_common::template`] for the
//! `{{...}}` roots, the optional-`?` suffix and which of them resolve in
//! which phase, and [`asterism_exporter_common::jsonpath`] for the path
//! subset. They are not restated here — a grammar with two write-ups
//! grows two meanings, and a profile author cannot tell which one their
//! adapter implements.
//!
//! This exporter reaches them through the
//! [`TemplateAdapter`] / [`ResponsePath`] traits, and the implementation
//! it holds is [`SecretGrammar`] — the shared roots plus `{{secret}}`,
//! bound per call from whatever the `auth` block names. A profile
//! without that block binds nothing, and `{{secret}}` in one is refused
//! rather than rendered away.
//!
//! ## Where a credential may live, and what that costs
//!
//! Params are persisted unedited: the blob handed to
//! `CreateDispatchCommand` is stored whole as `dispatch_job.params_json`
//! and handed back out as `DispatchDto.params_json` on every read of the
//! dispatch, and nothing on that path filters, redacts, or drops a
//! field. A credential reached by `{{params.…}}` is therefore readable
//! by anything that can list dispatches — and, since the call is
//! recorded (below), it also rides on the assets the job produced.
//!
//! `auth.secret_ref` is the way out, and it holds an environment
//! variable *name*, never a value: the credential is resolved per call,
//! rendered into `{{secret}}`, and is in neither the params blob nor
//! anything written down. Loading a `.env` file is the binary's job,
//! done once at startup — an adapter that went looking for dotenv files
//! itself would make "which file did this credential come from"
//! invisible to the profile that named it.
//!
//! ## The call is recorded, and it arrives with the artefact
//!
//! The request as sent and the response as received are kept on the
//! dispatch row, in the exporter-owned handle payload the runner already
//! persists and hands back — and the harvest copies that record onto
//! every [`Derived`] it returns, under `extra.http.call`, together with
//! the finished job's response whole. A backend that answers with a
//! result URL and little else is the ordinary case: the model can be an
//! ambient default, the seed is an input that is usually not echoed, and
//! an enhanced prompt is not the prompt that was sent. None of that is
//! in the bytes, so the moment of the call is the only moment it exists.
//! The response is kept whole because those values are siblings of the
//! artefacts array rather than fields of an item.
//!
//! The recorded copy is scrubbed of the credential the `auth` block
//! named, and its headers are redacted. So is the handle itself, on the
//! way into the payload: `submit.handle_from` defaults to the whole
//! submit response, a backend is free to echo the request it was sent,
//! and that payload is handed back out on every read of the dispatch —
//! including to a caller that never touches the database. What no scrub
//! can reach is a token a profile interpolated out of its own params
//! into a URL or a body: the adapter was never told it was a
//! credential. That is the same trade the paragraph above describes,
//! one surface further along.

pub mod custody;
pub mod secret;

use std::collections::BTreeMap;
use std::path::PathBuf;

use asterism_dispatch_sdk::{
    AttemptRecord, Derived, DispatchContext, DispatchState, Exporter, ExporterError, Handle,
    ProgressHint,
};
use asterism_exporter_common::{ResponsePath, TemplateAdapter, TemplateEnv};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use custody::CustodyPaths;
pub use secret::SecretGrammar;

/// Slug the registry uses for this exporter.
pub const SLUG: &str = "http";

/// Slug this adapter also answers to, from when hosted platforms had
/// their own crate.
///
/// Kept because it is written down in two places that outlive the
/// merge: `dispatch_job.exporter_slug` on every dispatch that ran under
/// it, and `extra._dispatch.exporter_slug` on every asset one of those
/// produced. Registering the merged adapter under both names is what
/// makes those rows re-runnable and their history readable, rather than
/// a migration that rewrites what happened.
pub const LEGACY_HOSTED_SLUG: &str = "cloud";

/// Prefix on the message of a dispatch failed for exceeding its
/// profile's deadline, so an expiry is tellable from a backend failure.
pub const EXPIRY_PREFIX: &str = "deadline exceeded";

/// What a redacted value is replaced by in the recorded request.
pub const REDACTED: &str = "«redacted»";

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
    ///
    /// Spelled `dispatch` before the hosted adapter merged in, and that
    /// spelling still parses: stored params are re-read whenever a
    /// dispatch is re-run, so a rename with no alias would strand every
    /// profile already on a row.
    #[serde(alias = "dispatch")]
    pub submit: SubmitSchema,
    /// How to check on the job.
    pub poll: PollSchema,
    /// How to collect results once the job is done.
    pub harvest: HarvestSchema,
    /// Where the credential comes from, for a backend that wants one.
    ///
    /// Absent means the profile names no credential: `{{secret}}` is
    /// then refused, and a profile that reaches a token through its own
    /// params still works on the terms the crate doc states.
    #[serde(default)]
    pub auth: Option<AuthSchema>,
    /// How the produced bytes are pulled into our custody.
    ///
    /// Absent means they are not: the backend's own URL is the locator,
    /// which is the right answer for a backend that keeps serving it.
    #[serde(default)]
    pub fetch: Option<FetchSchema>,
    /// How long this backend's job may take before the dispatch is
    /// recorded as expired.
    ///
    /// Absent means no ceiling — the runner keeps polling until the
    /// backend answers, which is what a local backend deserves. A
    /// hosted platform is the other case, and there is no shared
    /// default to give it: how long a job may take before its result is
    /// gone is a property of the platform, and the profile is where
    /// that platform is described.
    #[serde(default)]
    pub deadline_seconds: Option<u64>,
}

/// Where the credential comes from and how it is presented.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthSchema {
    /// **Name of an environment variable**, never a value. A profile
    /// carrying the credential itself would put it on the dispatch row.
    pub secret_ref: String,
    /// Header the rendered value goes in.
    #[serde(default = "default_auth_header")]
    pub header: String,
    /// Template for the header value. `{{secret}}` is the resolved
    /// variable; every other placeholder resolves as it does anywhere
    /// else.
    #[serde(default = "default_auth_value")]
    pub value_template: String,
}

/// How the produced bytes are pulled into our custody.
#[derive(Debug, Clone, Deserialize)]
pub struct FetchSchema {
    /// Send the auth header on the download too. Some backends want
    /// the credential on the download as well; others serve their
    /// artefacts from a public URL that rejects the header, so this is
    /// per profile rather than implied by `auth`.
    #[serde(default)]
    pub authenticated: bool,
    /// Extra headers on the download (template-substituted).
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// Schema for the `POST /prompt`-shaped submit call.
#[derive(Debug, Clone, Deserialize)]
pub struct SubmitSchema {
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
    /// Per-item mapping. Every field is optional except `source_url`.
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
    /// Template resolving to the backend's URL for this artefact.
    ///
    /// With no `fetch` block this *is* the locator — the reified asset
    /// reads from the backend, which is why it was called `locator`
    /// before hosted platforms merged in, and why that spelling still
    /// parses. With a `fetch` block it is the URL the bytes are pulled
    /// from and the locator names the file we then hold, because a URL
    /// a platform stops serving is not somewhere an asset can point.
    #[serde(alias = "locator")]
    pub source_url: String,
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

fn default_auth_header() -> String {
    "authorization".into()
}
fn default_auth_value() -> String {
    "Bearer {{secret}}".into()
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
pub struct HttpHandlePayload {
    /// Value extracted via `submit.handle_from` — the exporter uses
    /// this to fill the `{{handle.…}}` placeholder on subsequent
    /// polls.
    pub handle: Value,
    /// When the backend accepted the job. The deadline is measured from
    /// here, so it has to survive a restart with the handle.
    ///
    /// Optional because a job submitted before this field existed is
    /// still in flight when the process carrying this code starts
    /// polling it, and a handle that fails to rehydrate is a job lost to
    /// a struct change. Absent means no deadline can be applied to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_at_ms: Option<i64>,
    /// The recorded exchange. Written on every submit; optional for the
    /// same reason `submitted_at_ms` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange: Option<Exchange>,
}

/// The request as sent beside the response as received.
///
/// Redaction happens on the way in, not on the way out: the auth header
/// is replaced by name, and any other occurrence of the resolved secret
/// — a profile is free to put `{{secret}}` in a query string or a body
/// field — is scrubbed from the recorded copy. A redaction applied at
/// read time would mean the value had already been written down.
///
/// The secret the `auth` block named, that is. A token a profile
/// interpolated out of its own params into a URL or a body is out of
/// reach here as it is everywhere else — the adapter was never told it
/// was a credential (the crate doc states the trade). This record is one
/// more surface it can land on, and the poll and harvest requests it
/// carries are surfaces nothing recorded before.
///
/// Recorded whichever way the call went. A refused submit is the case a
/// reader has the most questions about — which endpoint, with which
/// body, and what the backend actually said — and it is the case that
/// produces no handle to carry the answer, so this shape also travels
/// through [`DispatchContext::attempt`] onto the row itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exchange {
    /// Submit request, as sent.
    pub request: RecordedRequest,
    /// Status the backend answered with.
    ///
    /// Absent on a record written for a call that never reached one
    /// (see `error`), and on a handle from before the field existed. It
    /// is what separates a backend that rejected the request from one
    /// that was not there — different questions, and a reader should not
    /// have to infer which they are looking at from the shape of the
    /// body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Response as received: the parsed document when the answer was
    /// JSON, the raw text as a string when it was not, and `null` when
    /// nothing answered.
    pub response: Value,
    /// Why no answer arrived — DNS failure, refused connection, timeout.
    ///
    /// Present only on a call the backend never answered, which is the
    /// same thing `status` being absent says. Both are written because
    /// "not reached" is the state a reader most wants a sentence for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One recorded HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedRequest {
    /// HTTP method.
    pub method: String,
    /// Full URL.
    pub url: String,
    /// Headers, with the credential removed.
    pub headers: BTreeMap<String, String>,
    /// JSON body, if the call had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

/// Schema-driven [`Exporter`] for HTTP job APIs.
#[derive(Debug, Clone)]
pub struct HttpExporter {
    http: reqwest::Client,
    custody: CustodyPaths,
}

impl HttpExporter {
    /// Builds an exporter that takes custody of produced files under
    /// `custody_root` (the application directory) whenever a profile
    /// asks for it with a `fetch` block.
    ///
    /// The root is a constructor argument rather than a params field
    /// because a params-supplied path would let a dispatch write outside
    /// the profile that ran it.
    /// The 120-second client timeout is a whole-request ceiling, and it
    /// is the hosted adapter's rather than this one's earlier 30: the
    /// same client now serves a `fetch` block pulling an artefact down,
    /// which is a download rather than a JSON call, and 30 seconds is a
    /// size limit dressed as a timeout. A submit that hangs is bounded
    /// by the profile's own deadline, which is the number meant to
    /// decide how long to wait.
    pub fn new(custody_root: PathBuf) -> Self {
        Self::with_client(
            custody_root,
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client build"),
        )
    }

    /// Uses a caller-supplied `reqwest::Client` (integration tests
    /// pass a mocked one).
    pub fn with_client(custody_root: PathBuf, http: reqwest::Client) -> Self {
        Self {
            http,
            custody: CustodyPaths::new(custody_root),
        }
    }

    /// The grammar for one call: the shared roots, plus `{{secret}}`
    /// when the profile named a credential to bind to it.
    ///
    /// Resolved per call rather than held on the struct, because the
    /// value comes from the process environment and a profile that
    /// names a variable which is not set has to fail *naming it* — an
    /// author who mistyped `BACKEND_KEY` learns which spelling was
    /// looked for.
    fn grammar(&self, params: &HttpDispatchParams) -> Result<SecretGrammar, ExporterError> {
        let Some(auth) = params.auth.as_ref() else {
            return Ok(SecretGrammar::unauthenticated());
        };
        let secret = std::env::var(&auth.secret_ref).map_err(|_| {
            ExporterError::BackendRejected(format!(
                "auth.secret_ref names environment variable {:?}, which is not set",
                auth.secret_ref
            ))
        })?;
        Ok(SecretGrammar::new(secret))
    }

    /// Headers for one call: the profile's own, plus the auth header
    /// when there is one.
    fn headers(
        &self,
        grammar: &SecretGrammar,
        params: &HttpDispatchParams,
        own: &BTreeMap<String, String>,
        env: &TemplateEnv<'_>,
    ) -> Result<BTreeMap<String, String>, ExporterError> {
        let mut headers = grammar.render_headers(own, env)?;
        if let Some(auth) = params.auth.as_ref() {
            headers.insert(
                auth.header.clone(),
                grammar.render(&auth.value_template, env)?,
            );
        }
        Ok(headers)
    }

    /// Evaluates a poll predicate against a response body.
    ///
    /// A path that matches nothing is false rather than an error: on a
    /// poll that is "not yet", which is the answer a job in flight
    /// should produce.
    fn match_rule(&self, grammar: &SecretGrammar, rule: &MatchRule, resp: &Value) -> bool {
        match grammar.select_first(resp, &rule.path) {
            Some(actual) => actual == rule.equals,
            None => false,
        }
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
        let grammar = self.grammar(&params)?;
        let base = trim_trailing_slash(&params.endpoint);
        let env = TemplateEnv::pre_handle(&ctx, ctx.params);
        let path = grammar.render(&params.submit.path, &env)?;
        let url = format!("{base}{path}");
        let body = grammar.render_json(&params.submit.body_template, &env)?;
        let headers = self.headers(&grammar, &params, &params.submit.headers, &env)?;
        let answer = send_request(
            &self.http,
            &params.submit.method,
            &url,
            &headers,
            Some(body.clone()),
        )
        .await
        .map_err(|e| grammar.scrub_error(e))?;
        // Recorded before the answer is judged, because the judgement is
        // where a refusal stops being something this method can hand
        // back: it leaves as an error, and the error is a sentence. What
        // the row keeps of a refused submit is written here or nowhere.
        let exchange = record_exchange(
            &grammar,
            params.auth.as_ref().map(|auth| auth.header.as_str()),
            &params.submit.method,
            &url,
            &headers,
            Some(&body),
            &answer,
        );
        ctx.attempt
            .record(AttemptRecord::new(SLUG, attempt_payload(&exchange)));
        let resp = answer
            .into_body(&params.submit.method, &url)
            .map_err(|e| grammar.scrub_error(e))?;
        let handle_value = handle_from_response(&grammar, &resp, &params.submit.handle_from)?;
        Ok(Handle::new(
            SLUG,
            serde_json::to_value(HttpHandlePayload {
                handle: handle_value,
                submitted_at_ms: Some(Utc::now().timestamp_millis()),
                exchange: Some(exchange),
            })
            .expect("handle payload serialises"),
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
        let grammar = self.grammar(&params)?;
        let expiry = expiry_state(&params, &payload, Utc::now().timestamp_millis());
        let env = TemplateEnv::with_handle(&ctx, ctx.params, &payload.handle);
        let base = trim_trailing_slash(&params.endpoint);
        let path = grammar.render(&params.poll.path, &env)?;
        let url = format!("{base}{path}");
        let headers = self.headers(&grammar, &params, &params.poll.headers, &env)?;

        // The backend is asked even when the deadline has passed, and
        // the expiry is only the answer if the backend does not have a
        // better one. A job that finished a second late finished; a
        // deadline is our rule about how long to keep waiting, not a
        // claim about what the backend did. When the call itself fails
        // past the deadline — the platform has forgotten the job, which
        // is what a deadline predicts — the expiry is the more useful
        // explanation than the 404.
        let auth_header = params.auth.as_ref().map(|auth| auth.header.as_str());
        let (resp, exchange) =
            match send_request(&self.http, &params.poll.method, &url, &headers, None).await {
                Ok(answer) => {
                    let exchange = record_exchange(
                        &grammar,
                        auth_header,
                        &params.poll.method,
                        &url,
                        &headers,
                        None,
                        &answer,
                    );
                    match answer.into_body(&params.poll.method, &url) {
                        Ok(resp) => (resp, exchange),
                        Err(err) => {
                            // A poll that cannot be read is the same
                            // unanswered question a refused submit is,
                            // and the handle it could write onto holds
                            // the *submit* — overwriting that would
                            // trade one record for another.
                            ctx.attempt
                                .record(AttemptRecord::new(SLUG, attempt_payload(&exchange)));
                            return expiry.ok_or_else(|| grammar.scrub_error(err));
                        }
                    }
                }
                Err(err) => return expiry.ok_or_else(|| grammar.scrub_error(err)),
            };

        if self.match_rule(&grammar, &params.poll.failed_when, &resp) {
            let message = params
                .poll
                .failed_when
                .message_path
                .as_deref()
                .and_then(|p| grammar.select_first(&resp, p))
                .and_then(|v| grammar.display_string(v))
                .unwrap_or_else(|| "backend reported failure".into());
            // The backend answered, and the answer is that the job is
            // over. Same reason as above: this is the call that ends the
            // run, so it is the one a reader will come back to.
            ctx.attempt
                .record(AttemptRecord::new(SLUG, attempt_payload(&exchange)));
            return Ok(DispatchState::Failed { message });
        }
        if self.match_rule(&grammar, &params.poll.done_when, &resp) {
            return Ok(DispatchState::Done);
        }
        // Still working, and out of time: this is where the deadline
        // earns its keep, by ending a loop that would otherwise re-poll
        // a queue position forever.
        if let Some(expired) = expiry {
            return Ok(expired);
        }
        let message = params
            .poll
            .progress_message_path
            .as_deref()
            .and_then(|p| grammar.select_first(&resp, p))
            .and_then(|v| grammar.display_string(v));
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
        let grammar = self.grammar(&params)?;
        let env = TemplateEnv::with_handle(&ctx, ctx.params, &payload.handle);
        let base = trim_trailing_slash(&params.endpoint);
        let path = grammar.render(&params.harvest.path, &env)?;
        let url = format!("{base}{path}");
        let headers = self.headers(&grammar, &params, &params.harvest.headers, &env)?;
        let answer = send_request(&self.http, &params.harvest.method, &url, &headers, None)
            .await
            .map_err(|e| grammar.scrub_error(e))?;
        let exchange = record_exchange(
            &grammar,
            params.auth.as_ref().map(|auth| auth.header.as_str()),
            &params.harvest.method,
            &url,
            &headers,
            None,
            &answer,
        );
        let resp = match answer.into_body(&params.harvest.method, &url) {
            Ok(resp) => resp,
            Err(err) => {
                // The job ran and the collection failed, which is the
                // one thing a reader cannot reconstruct from the
                // artefacts: there are none to read the call off.
                ctx.attempt
                    .record(AttemptRecord::new(SLUG, attempt_payload(&exchange)));
                return Err(grammar.scrub_error(err));
            }
        };
        let items = grammar.select(&resp, &params.harvest.items_path);
        let now = Utc::now();
        // Built once and cloned per item: every artefact of one job came
        // out of the same call, and re-deriving it inside the loop would
        // invite the two copies to drift.
        let call = call_note(&grammar, &payload, resp);
        let mut out: Vec<Derived> = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let item_env = env.with_item(item);
            let modality = grammar.render(&params.harvest.map.modality, &item_env)?;
            let source_url = grammar.render(&params.harvest.map.source_url, &item_env)?;
            // With a `fetch` block the URL above is expected to stop
            // working, so the Derived names what we wrote; without one
            // the backend keeps serving it and it *is* the locator.
            let (locator, file_size_bytes) = match params.fetch.as_ref() {
                Some(fetch) => {
                    let mut fetch_headers = grammar.render_headers(&fetch.headers, &item_env)?;
                    // `authenticated` without an `auth` block has
                    // nothing to send, and that is a profile mistake
                    // rather than a call to make differently: the
                    // download goes out unauthenticated and the backend
                    // says what it thinks of that.
                    if let (true, Some(auth)) = (fetch.authenticated, params.auth.as_ref()) {
                        fetch_headers.insert(
                            auth.header.clone(),
                            grammar.render(&auth.value_template, &item_env)?,
                        );
                    }
                    let bytes = fetch_bytes(&self.http, &source_url, &fetch_headers)
                        .await
                        .map_err(|e| grammar.scrub_error(e))?;
                    let written = self
                        .custody
                        .write(ctx.dispatch_id, index, &source_url, &bytes)
                        .await?;
                    (
                        written.to_string_lossy().into_owned(),
                        Some(bytes.len() as u64),
                    )
                }
                // Nothing was read, so nothing is known about the size:
                // absent rather than zero, which would sort ahead of
                // every measured value on an ascending axis.
                None => (source_url.clone(), None),
            };
            let cover_hint = params
                .harvest
                .map
                .cover_hint
                .as_deref()
                .map(|t| grammar.render(t, &item_env))
                .transpose()?;
            let register_note = params
                .harvest
                .map
                .register_note
                .as_deref()
                .map(|t| grammar.render(t, &item_env))
                .transpose()?;
            let mut labels: Vec<String> = Vec::new();
            for tmpl in &params.harvest.map.labels_static {
                labels.push(grammar.render(tmpl, &item_env)?);
            }
            if let Some(path) = &params.harvest.map.labels_path {
                for v in grammar.select(
                    item,
                    path.trim_start_matches("$.item").trim_start_matches("$"),
                ) {
                    if let Some(s) = grammar.display_string(v) {
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
                file_size_bytes,
                duration_ms: None,
                extra: serde_json::json!({
                    "http": {
                        "item": grammar.scrub(item.clone()),
                        // The backend's own URL, kept beside the file we
                        // now hold when there is one. Scrubbed: a
                        // download authenticated by query parameter
                        // renders the credential into it, and this is
                        // persisted on the asset.
                        "source_url": grammar.scrub_text(&source_url),
                        // How this artefact was asked for, travelling
                        // with the artefact. The same record is on the
                        // dispatch row; it is repeated here because an
                        // asset that has to resolve an id to say what
                        // made it is one whose answer can go missing
                        // separately from it.
                        "call": call.clone(),
                    }
                }),
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

/// Accepts this adapter's own handles and the ones the hosted adapter
/// stamped before it merged in.
///
/// A handle kind is written at submit and read on every tick after it,
/// so a job in flight across the merge has a `cloud` handle and a
/// process that only answers to `http`. Refusing it would fail a job
/// that is running fine on the far side.
fn check_kind(handle: &Handle) -> Result<(), ExporterError> {
    if handle.kind != SLUG && handle.kind != LEGACY_HOSTED_SLUG {
        return Err(ExporterError::HandleMismatch {
            exporter_slug: SLUG.into(),
            handle_kind: handle.kind.clone(),
        });
    }
    Ok(())
}

/// The call as it will be written down: the request as sent, and
/// whatever came back of it.
///
/// Takes the [`Answer`] rather than a judgement of it, which is the
/// whole point — a rejection and an unreachable host are both recorded
/// here, and both are cases where the caller is about to return an error
/// and have nothing left to record from.
///
/// Everything is scrubbed on the way in, for the reason [`Exchange`]
/// states: a redaction applied at read time would mean the value had
/// already been written down.
fn record_exchange(
    grammar: &SecretGrammar,
    auth_header: Option<&str>,
    method: &str,
    url: &str,
    headers: &BTreeMap<String, String>,
    body: Option<&Value>,
    answer: &Answer,
) -> Exchange {
    let (status, response, error) = answer.recorded(grammar);
    Exchange {
        request: RecordedRequest {
            method: method.to_string(),
            // Scrubbed, not copied: query-parameter auth is a real
            // shape, so a profile may legitimately have rendered
            // `{{secret}}` into this string.
            url: grammar.scrub_text(url),
            headers: grammar.redact_headers(headers, auth_header),
            body: body.map(|body| grammar.scrub(body.clone())),
        },
        status,
        response,
        error,
    }
}

/// The exchange as an [`AttemptRecord`] payload.
///
/// Under the same `exchange` key the handle payload uses, so the record
/// of a refused submit and the record of an accepted one are read the
/// same way — which is the property the whole path exists for.
fn attempt_payload(exchange: &Exchange) -> Value {
    serde_json::json!({
        "exchange": serde_json::to_value(exchange).expect("a recorded exchange serialises"),
    })
}

/// The call that produced a harvest, as it is written onto every
/// artefact that came out of it.
///
/// A projection of the handle payload rather than the payload itself:
/// the payload is this adapter's working state, and what an asset should
/// carry is the record of the call — which job the backend gave us, when
/// we asked, what we sent, what came back. Serialising the payload
/// wholesale would put the working shape on assets and make every later
/// field of it a wire change nobody asked for.
///
/// `result` is the harvest response whole, and not only because it is
/// cheap to keep: the fields a re-run would need are routinely *outside*
/// the items array. The seed a backend actually used, the prompt as it
/// rewrote it, the timings — these are siblings of the array, so an
/// adapter that kept the selected item and dropped the envelope would
/// discard the values a job is least able to reconstruct. The item stays
/// beside it because which of the returned artefacts this asset is
/// cannot be read back off the envelope.
///
/// `request` and `response` are absent together, and only for a job
/// submitted before the record existed. Absent here is the literal "this
/// handle predates the record" rather than a claim about the call, and
/// it is written as a missing key rather than a null for the reason the
/// disclosure vocabulary states: a null reads as a value somebody wrote.
///
/// # Why the scrub happens here and not at each caller
///
/// The harvest response arrives raw and is scrubbed here. The handle
/// was scrubbed on the way into the payload, and is scrubbed again on
/// the way out — not because the first pass is in doubt, but because a
/// job submitted before that pass existed is still in flight, and its
/// handle is the whole submit response of a backend that may have
/// echoed the request it was sent. The scrub is idempotent, so the
/// second pass costs a walk of a small document and covers the case the
/// first cannot reach. Taking everything through one place is what
/// stops the next field added here from being the one that forgets.
fn call_note(grammar: &SecretGrammar, payload: &HttpHandlePayload, result: Value) -> Value {
    let mut note = serde_json::json!({
        "handle": grammar.scrub(payload.handle.clone()),
        "result": grammar.scrub(result),
    });
    let Some(map) = note.as_object_mut() else {
        return note;
    };
    if let Some(submitted_at_ms) = payload.submitted_at_ms {
        map.insert("submitted_at_ms".into(), serde_json::json!(submitted_at_ms));
    }
    if let Some(exchange) = payload.exchange.as_ref() {
        map.insert(
            "request".into(),
            serde_json::to_value(&exchange.request).expect("a recorded request serialises"),
        );
        map.insert("response".into(), exchange.response.clone());
    }
    note
}

/// The handle as it will be persisted: what the profile's `handle_from`
/// path selects out of the submit response, scrubbed before it is
/// anything else.
///
/// The scrub is not belt-and-braces here. `handle_from` defaults to `$`,
/// so the handle is routinely the whole submit response, and a backend
/// that echoes the request it was sent hands back whatever the profile
/// rendered `{{secret}}` into. The core persists this payload verbatim
/// and hands it out on every read of the dispatch — including to a
/// caller with no database access — so the raw copy is where a
/// credential would outlive the call.
///
/// Scrubbed rather than dropped, because the poll templates
/// `{{handle.…}}` off this same value and has to be able to reach the
/// backend again: a job id is untouched, and only the echo changes.
///
/// Split out from `dispatch` so the property is checkable without
/// standing up a backend or setting a process-wide environment
/// variable, in the way [`expiry_state`] takes its clock as an argument.
fn handle_from_response(
    grammar: &SecretGrammar,
    resp: &Value,
    handle_from: &str,
) -> Result<Value, ExporterError> {
    grammar
        .select_first(resp, handle_from)
        .map(|value| grammar.scrub(value))
        .ok_or_else(|| {
            ExporterError::BackendRejected(format!(
                "submit response missing handle_from path {handle_from:?}"
            ))
        })
}

/// `Some(Failed)` once the profile's deadline has passed.
///
/// Split out and taking `now_ms` so the rule is testable without
/// waiting out a deadline or reaching for a clock abstraction the rest
/// of the crate does not need. A profile with no deadline, and a handle
/// from before submit times were recorded, both answer `None`: neither
/// is a job that has run out of time, they are jobs nothing said when to
/// stop waiting for.
pub fn expiry_state(
    params: &HttpDispatchParams,
    payload: &HttpHandlePayload,
    now_ms: i64,
) -> Option<DispatchState> {
    let deadline_seconds = params.deadline_seconds?;
    let submitted_at_ms = payload.submitted_at_ms?;
    let elapsed_ms = now_ms.saturating_sub(submitted_at_ms);
    let deadline_ms = (deadline_seconds as i64).saturating_mul(1000);
    (elapsed_ms > deadline_ms).then(|| DispatchState::Failed {
        message: format!(
            "{EXPIRY_PREFIX}: {}s since submit, profile allows {deadline_seconds}s",
            elapsed_ms / 1000,
        ),
    })
}

async fn fetch_bytes(
    http: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
) -> Result<Vec<u8>, ExporterError> {
    let mut req = http.get(url);
    for (name, value) in headers {
        req = req.header(name.as_str(), value.as_str());
    }
    let resp = req
        .send()
        .await
        .map_err(|e| ExporterError::Other(anyhow::anyhow!("fetching {url}: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        // Worth saying which URL: at this point the job succeeded and
        // only the download failed, and the two are told apart by what
        // the message names.
        return Err(ExporterError::BackendRejected(format!(
            "fetching {url} returned HTTP {status}"
        )));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| ExporterError::Other(anyhow::anyhow!("fetching {url}: {e}")))
}

fn trim_trailing_slash(s: &str) -> &str {
    s.trim_end_matches('/')
}

/// What one call came back as, before anything has been decided about
/// it.
///
/// [`send_request`] used to answer `Result<Value, ExporterError>`, which
/// is the shape of the *judgement* rather than of the answer: a rejection
/// arrived as a sentence with the status and the body already formatted
/// into it, and nothing downstream could record what came back because
/// nothing downstream still had it. Splitting the two lets the same call
/// feed the record and the verdict.
enum Answer {
    /// The backend answered.
    Answered {
        /// Status line, kept whole so the message a rejection produces
        /// reads as it always did.
        status: reqwest::StatusCode,
        /// Body as received.
        text: String,
        /// The body parsed, or why it would not parse.
        json: Result<Value, String>,
    },
    /// Nothing answered — DNS, refused connection, timeout, a body that
    /// stopped mid-read.
    Unreachable {
        /// What the transport said.
        error: String,
    },
}

impl Answer {
    /// The parsed body of a call that succeeded — what every caller here
    /// wants, and the errors they raised before [`Answer`] existed.
    fn into_body(self, method: &str, url: &str) -> Result<Value, ExporterError> {
        match self {
            Answer::Unreachable { error } => Err(ExporterError::Other(anyhow::anyhow!(error))),
            Answer::Answered { status, text, json } => {
                if !status.is_success() {
                    return Err(ExporterError::BackendRejected(format!(
                        "http {method} {url} HTTP {status}: {text}"
                    )));
                }
                json.map_err(|e| {
                    ExporterError::BackendRejected(format!(
                        "http {method} {url} response not JSON: {e}"
                    ))
                })
            }
        }
    }

    /// The answer as it is written down: the status, the body, and the
    /// transport's own words when there was no body to have.
    ///
    /// A non-JSON body is recorded as the string it was rather than
    /// dropped — an HTML error page is frequently the most informative
    /// thing a misconfigured endpoint returns.
    fn recorded(&self, grammar: &SecretGrammar) -> (Option<u16>, Value, Option<String>) {
        match self {
            Answer::Unreachable { error } => (None, Value::Null, Some(grammar.scrub_text(error))),
            Answer::Answered { status, text, json } => {
                let body = match json {
                    Ok(value) => grammar.scrub(value.clone()),
                    Err(_) => Value::String(grammar.scrub_text(text)),
                };
                (Some(status.as_u16()), body, None)
            }
        }
    }
}

async fn send_request(
    http: &reqwest::Client,
    method: &str,
    url: &str,
    headers: &BTreeMap<String, String>,
    body: Option<Value>,
) -> Result<Answer, ExporterError> {
    // The one failure that is not an answer and not a transport error:
    // no call goes out at all, because the profile named a method this
    // adapter does not send.
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
    let resp = match req.send().await {
        Ok(resp) => resp,
        Err(e) => {
            return Ok(Answer::Unreachable {
                error: format!("http {method} {url}: {e}"),
            });
        }
    };
    let status = resp.status();
    // Read as text and parse here rather than asking reqwest for JSON:
    // the raw body is what a rejection's message carries and what a
    // non-JSON answer is recorded as, and `json()` consumes the response
    // without leaving either behind.
    let text = match resp.text().await {
        Ok(text) => text,
        Err(e) => {
            return Ok(Answer::Unreachable {
                error: format!("http {method} {url}: {e}"),
            });
        }
    };
    let json = serde_json::from_str::<Value>(&text).map_err(|e| e.to_string());
    Ok(Answer::Answered { status, text, json })
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_contract::dto::AssetCardDto;

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
            // Nothing here drives a call through `dispatch`, so nothing
            // records; what the recorder carries out of a real call is
            // asserted end to end
            // (`asterism-server/tests/dispatch_remote_backend_e2e.rs`).
            attempt: &asterism_dispatch_sdk::DISCARD_ATTEMPTS,
        }
    }

    #[test]
    fn slug_is_stable_and_action_open() {
        let exp = HttpExporter::new(PathBuf::from("/tmp/asterism-test"));
        assert_eq!(exp.slug(), SLUG);
        assert!(exp.accepts("run"));
        assert!(exp.accepts("any-slug"));
    }

    // The template and JSONPath mechanics are tested where they are
    // defined (`asterism_exporter_common::template` / `::jsonpath`).
    // What stays here is what this crate owns: the params schema, the
    // shipped example, and the state-machine rules read out of a
    // response. The example is exercised through the same grammar the
    // exporter holds, so a test cannot pass against a spelling the
    // exporter does not use.

    #[test]
    fn match_rule_fires_only_on_equal_value() {
        let exp = HttpExporter::new(PathBuf::from("/tmp/asterism-test"));
        let g = SecretGrammar::unauthenticated();
        let doc = serde_json::json!({ "status": "done" });
        let rule = MatchRule {
            path: "$.status".into(),
            equals: Value::String("done".into()),
            message_path: None,
        };
        assert!(exp.match_rule(&g, &rule, &doc));
        let rule_neg = MatchRule {
            path: "$.status".into(),
            equals: Value::String("failed".into()),
            message_path: None,
        };
        assert!(!exp.match_rule(&g, &rule_neg, &doc));
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
        assert_eq!(params.submit.handle_from, "$.job_id");
        assert_eq!(params.harvest.items_path, "$.outputs[*]");
        assert_eq!(params.harvest.map.source_url, "{{item.url}}");
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
        let g = SecretGrammar::unauthenticated();

        // auth.value_template — the one template the phases below never
        // reach, because rendering it needs a bound credential. Left
        // out, the example could ship `{{secrets}}` and every other
        // assertion here would still pass.
        let auth = params
            .auth
            .as_ref()
            .expect("the example ships an auth block");
        assert_eq!(
            SecretGrammar::new("k-123".into())
                .render(
                    &auth.value_template,
                    &TemplateEnv::pre_handle(&ctx, ctx.params)
                )
                .expect("the example's auth template renders"),
            "Bearer k-123"
        );

        // dispatch — no handle exists yet.
        let pre = TemplateEnv::pre_handle(&ctx, ctx.params);
        g.render(&params.submit.path, &pre).unwrap();
        g.render_headers(&params.submit.headers, &pre).unwrap();
        let body = g.render_json(&params.submit.body_template, &pre).unwrap();
        assert_eq!(body["input_url"], "/tmp/photo.png");
        assert_eq!(body["prompt"], "photo studio portrait");
        assert_eq!(body["client_id"], "disp-1");

        // poll / harvest — the handle is in play.
        let handle = Value::String("job-1".into());
        let env = TemplateEnv::with_handle(&ctx, ctx.params, &handle);
        assert_eq!(g.render(&params.poll.path, &env).unwrap(), "/jobs/job-1");
        g.render_headers(&params.poll.headers, &env).unwrap();
        g.render(&params.harvest.path, &env).unwrap();
        g.render_headers(&params.harvest.headers, &env).unwrap();

        // harvest.map — the only place `{{item.…}}` resolves.
        let item = serde_json::json!({ "url": "https://renders.test/a.png" });
        let item_env = env.with_item(&item);
        assert_eq!(
            g.render(&params.harvest.map.source_url, &item_env).unwrap(),
            "https://renders.test/a.png"
        );
        g.render(&params.harvest.map.modality, &item_env).unwrap();
        // The example's cover hint is optional (`?`); this item has no
        // caption, so it has to resolve to empty rather than error.
        let cover = params.harvest.map.cover_hint.as_deref().unwrap();
        assert_eq!(g.render(cover, &item_env).unwrap(), "");
        let labels: Vec<String> = params
            .harvest
            .map
            .labels_static
            .iter()
            .map(|t| g.render(t, &item_env).unwrap())
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
    /// is evaluated here through the same grammar the exporter holds.
    #[test]
    fn params_example_jsonpaths_resolve_against_a_representative_response() {
        let params: HttpDispatchParams = serde_json::from_str(params_example_json()).unwrap();
        let exp = HttpExporter::new(PathBuf::from("/tmp/asterism-test"));
        let g = SecretGrammar::unauthenticated();
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
            g.select_first(&resp, &params.submit.handle_from),
            Some(Value::String("job-1".into()))
        );

        // poll.done_when / failed_when — both sides of the state
        // machine read the same field and differ only in `equals`.
        assert_eq!(
            g.select_first(&resp, &params.poll.done_when.path),
            Some(Value::String("succeeded".into()))
        );
        assert!(exp.match_rule(&g, &params.poll.done_when, &resp));
        assert_eq!(
            g.select_first(&resp, &params.poll.failed_when.path),
            Some(Value::String("succeeded".into()))
        );
        assert!(!exp.match_rule(&g, &params.poll.failed_when, &resp));
        let message_path = params
            .poll
            .failed_when
            .message_path
            .as_deref()
            .expect("the example ships a failure message path");
        assert_eq!(
            g.select_first(&resp, message_path),
            Some(Value::String("upstream refused the workflow".into()))
        );

        // poll.progress_message_path — echoed on the Running state.
        let progress_path = params
            .poll
            .progress_message_path
            .as_deref()
            .expect("the example ships a progress path");
        assert_eq!(
            g.select_first(&resp, progress_path),
            Some(Value::String("step 12/30".into()))
        );

        // harvest.items_path — one Derived per element, and the map's
        // locator has to resolve against each of them.
        let items = g.select(&resp, &params.harvest.items_path);
        assert_eq!(items.len(), 2, "the wildcard has to reach every output");

        let raw: Value = serde_json::from_str(params_example_json()).unwrap();
        let inputs = vec![card_fixture("asset-uuid", "/tmp/photo.png")];
        let ctx = stub_ctx(&inputs, &raw);
        let handle = Value::String("job-1".into());
        let env = TemplateEnv::with_handle(&ctx, ctx.params, &handle);
        let mut locators: Vec<String> = Vec::new();
        for item in &items {
            let item_env = env.with_item(item);
            locators.push(g.render(&params.harvest.map.source_url, &item_env).unwrap());
        }
        assert_eq!(
            locators,
            vec![
                "https://renders.test/a.png".to_string(),
                "https://renders.test/b.png".to_string(),
            ]
        );
    }

    /// The blocks the hosted adapter brought are optional, and a profile
    /// written before they existed is the case that has to keep working:
    /// no credential, no custody, no deadline, and the two blocks under
    /// their old names.
    #[test]
    fn a_profile_from_before_the_merge_still_parses() {
        let params: HttpDispatchParams = serde_json::from_value(serde_json::json!({
            "endpoint": "http://backend.test",
            "dispatch": {
                "path": "/generate",
                "body_template": { "prompt": "{{params.extras.prompt}}" },
                "handle_from": "$.job_id"
            },
            "poll": {
                "path": "/status/{{handle}}",
                "done_when": { "path": "$.status", "equals": "done" },
                "failed_when": { "path": "$.status", "equals": "failed" }
            },
            "harvest": {
                "path": "/result/{{handle}}",
                "items_path": "$.outputs[*]",
                "map": { "locator": "{{item.url}}" }
            }
        }))
        .expect("the pre-merge spelling is an alias, not a removal");

        assert_eq!(params.submit.path, "/generate");
        assert_eq!(params.harvest.map.source_url, "{{item.url}}");
        assert!(params.auth.is_none(), "no credential is named");
        assert!(params.fetch.is_none(), "the backend URL stays the locator");
        assert!(
            params.deadline_seconds.is_none(),
            "nothing says when to stop"
        );
    }

    fn payload_fixture(submitted_at_ms: Option<i64>) -> HttpHandlePayload {
        HttpHandlePayload {
            handle: serde_json::json!("job-1"),
            submitted_at_ms,
            exchange: Some(Exchange {
                request: RecordedRequest {
                    method: "POST".into(),
                    url: "http://backend.test/jobs".into(),
                    headers: BTreeMap::from([("authorization".into(), REDACTED.into())]),
                    body: Some(serde_json::json!({ "prompt": "a portrait", "seed": 7 })),
                },
                status: Some(200),
                response: serde_json::json!({ "job_id": "job-1", "status": "queued" }),
                error: None,
            }),
        }
    }

    /// A deadline is a profile's own rule, and most profiles do not have
    /// one. Without it a job is never expired, however long it has been
    /// in flight — a local backend that takes a day is not a failure.
    #[test]
    fn a_job_expires_only_when_its_profile_said_when() {
        let mut params: HttpDispatchParams =
            serde_json::from_str(params_example_json()).expect("example parses");
        let payload = payload_fixture(Some(1_000_000));
        let deadline_ms = params.deadline_seconds.expect("the example ships one") as i64 * 1000;

        assert!(expiry_state(&params, &payload, 1_000_000 + deadline_ms).is_none());
        let expired = expiry_state(&params, &payload, 1_000_000 + deadline_ms + 1)
            .expect("one millisecond past the deadline expires");
        match expired {
            DispatchState::Failed { message } => assert!(
                message.starts_with(EXPIRY_PREFIX),
                "an expiry has to be tellable from a backend failure: {message}"
            ),
            other => panic!("expected Failed, got {other:?}"),
        }

        params.deadline_seconds = None;
        assert!(
            expiry_state(&params, &payload, 1_000_000 + deadline_ms * 100).is_none(),
            "a profile with no deadline never expires a job"
        );
    }

    /// A job submitted before the handle carried a submit time is still
    /// in flight when this build starts polling it. Nothing can be
    /// measured from a moment that was never recorded, so it is not
    /// expired rather than expired at once.
    #[test]
    fn a_handle_without_a_submit_time_is_not_expired_by_default() {
        let params: HttpDispatchParams =
            serde_json::from_str(params_example_json()).expect("example parses");
        assert!(expiry_state(&params, &payload_fixture(None), i64::MAX / 2).is_none());
    }

    /// What the artefact has to carry away from the call: the backend's
    /// own id for the job, when it was asked for, the request as sent —
    /// the only place the prompt and the seed exist, since the file will
    /// not have them — the response that named the job, and the finished
    /// job's response whole.
    #[test]
    fn the_call_note_carries_what_was_sent_and_what_came_back() {
        let note = call_note(
            &SecretGrammar::unauthenticated(),
            &payload_fixture(Some(42)),
            serde_json::json!({
                "outputs": [{ "url": "http://backend.test/a.png" }],
                "seed": 913_224,
                "prompt": "a portrait, soft key light, 85mm",
            }),
        );

        assert_eq!(note["handle"], serde_json::json!("job-1"));
        assert_eq!(note["submitted_at_ms"], serde_json::json!(42));
        assert_eq!(note["request"]["body"]["prompt"], "a portrait");
        assert_eq!(note["request"]["body"]["seed"], 7);
        assert_eq!(note["response"]["job_id"], "job-1");
        // The envelope, not just the item that became the asset: the
        // seed the backend ran with and the prompt as it rewrote it sit
        // beside the artefacts array.
        assert_eq!(note["result"]["seed"], 913_224);
        assert_eq!(note["result"]["prompt"], "a portrait, soft key light, 85mm");
    }

    /// The note is written onto an asset, so it is held to what the
    /// recorded exchange is held to. Two ways the credential reaches it
    /// without anyone putting it there: `submit.handle_from` defaults to
    /// `$`, which makes the handle the whole submit response, and a
    /// backend is free to echo the request — including a URL a profile
    /// rendered `{{secret}}` into.
    #[test]
    fn the_note_cannot_carry_the_credential_a_backend_echoed_back() {
        let secret = "test-credential-not-a-real-key";
        let mut payload = payload_fixture(Some(42));
        payload.handle = serde_json::json!({
            "job_id": "job-1",
            "echo": { "url": format!("http://backend.test/jobs?key={secret}") },
        });

        let note = call_note(
            &SecretGrammar::new(secret.into()),
            &payload,
            serde_json::json!({
                "outputs": [{ "url": format!("http://backend.test/a.png?token={secret}") }],
                "seed": 913_224,
            }),
        );

        let rendered = note.to_string();
        assert!(
            !rendered.contains(secret),
            "the credential reached an asset's note: {rendered}"
        );
        // Scrubbed, not dropped: what the backend said still has to be
        // readable, or the note stops being the record it exists to be.
        assert_eq!(note["handle"]["job_id"], "job-1");
        assert_eq!(note["result"]["seed"], 913_224);
    }

    /// The same echo, one surface earlier. The note above is what lands
    /// on an asset; this is what lands on the dispatch row, is read back
    /// on every poll, and now reaches every reader of the dispatch
    /// through `DispatchDto.handle_json`. A scrub at the note boundary
    /// alone would leave the credential on the row it came to rest on.
    #[test]
    fn the_persisted_handle_cannot_carry_the_credential_a_backend_echoed_back() {
        let secret = "test-credential-not-a-real-key";
        let grammar = SecretGrammar::new(secret.into());
        let resp = serde_json::json!({
            "job_id": "job-1",
            "echo": { "url": format!("http://backend.test/jobs?key={secret}") },
        });

        // `$` is `handle_from`'s default, so this is the ordinary case
        // rather than a profile doing something unusual.
        let handle = handle_from_response(&grammar, &resp, "$")
            .expect("the whole document is what `$` selects");
        let rendered = handle.to_string();
        assert!(
            !rendered.contains(secret),
            "the credential reached the persisted handle: {rendered}"
        );
        // Scrubbed, not dropped: the poll renders `{{handle.…}}` off
        // this value, so it still has to name the job.
        assert_eq!(handle["job_id"], "job-1");
    }

    /// A profile naming a path the backend does not answer with is a
    /// profile the backend has rejected, and the message says which
    /// path. Checked beside the scrub because both now live in one
    /// function, and the error is the half that predates it.
    #[test]
    fn a_handle_path_that_resolves_to_nothing_names_itself() {
        let err = handle_from_response(
            &SecretGrammar::unauthenticated(),
            &serde_json::json!({ "id": "job-1" }),
            "$.job_id",
        )
        .expect_err("the response has no `job_id`");
        match err {
            ExporterError::BackendRejected(message) => {
                assert!(
                    message.contains("$.job_id"),
                    "the rejection has to name the path that failed: {message}"
                );
            }
            other => panic!("a missing handle path is a backend rejection, not {other:?}"),
        }
    }

    /// The handle a job in flight across the merge carries says `cloud`,
    /// because that is the crate that submitted it. Refusing it would
    /// fail a job that is running fine on the far side.
    #[test]
    fn a_handle_stamped_by_the_hosted_adapter_is_still_ours() {
        let ours = Handle::new(SLUG, serde_json::json!({ "handle": "job-1" }));
        let legacy = Handle::new(LEGACY_HOSTED_SLUG, serde_json::json!({ "handle": "job-1" }));
        let foreign = Handle::new("comfy", serde_json::json!({ "handle": "job-1" }));

        assert!(check_kind(&ours).is_ok());
        assert!(check_kind(&legacy).is_ok());
        assert!(matches!(
            check_kind(&foreign),
            Err(ExporterError::HandleMismatch { .. })
        ));
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
            ("submit", &params.submit.method),
            ("poll", &params.poll.method),
            ("harvest", &params.harvest.method),
        ] {
            assert!(
                ACCEPTED.contains(&method.to_ascii_uppercase().as_str()),
                "{phase}.method {method:?} is not one of {ACCEPTED:?}"
            );
        }
    }

    fn sent_headers() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("authorization".into(), "Bearer sk-not-a-real-key".into()),
            ("content-type".into(), "application/json".into()),
        ])
    }

    /// A backend that answers with a rejection has said something, and
    /// what it said is the answer to the question a refused submit
    /// leaves behind (#76).
    #[test]
    fn a_refusal_is_recorded_with_its_status_and_what_it_said() {
        let answer = Answer::Answered {
            status: reqwest::StatusCode::UNAUTHORIZED,
            text: r#"{"error":"invalid api key"}"#.into(),
            json: Ok(serde_json::json!({ "error": "invalid api key" })),
        };

        let exchange = record_exchange(
            &SecretGrammar::unauthenticated(),
            None,
            "POST",
            "http://backend.test/generate",
            &sent_headers(),
            Some(&serde_json::json!({ "prompt": "a portrait" })),
            &answer,
        );

        assert_eq!(exchange.status, Some(401));
        assert_eq!(exchange.response["error"], "invalid api key");
        assert!(
            exchange.error.is_none(),
            "a backend that answered was reached"
        );
        assert_eq!(exchange.request.method, "POST");
        assert_eq!(exchange.request.url, "http://backend.test/generate");
        assert_eq!(
            exchange.request.body.expect("the body as sent")["prompt"],
            "a portrait"
        );
        // And the judgement the same answer produces is unchanged: the
        // sentence on the row stays a sentence.
        match answer.into_body("POST", "http://backend.test/generate") {
            Err(ExporterError::BackendRejected(message)) => {
                assert!(message.contains("HTTP 401"), "{message}");
                assert!(message.contains("invalid api key"), "{message}");
            }
            other => panic!("a 401 is a rejection, not {other:?}"),
        }
    }

    /// "The endpoint said no" and "there is nothing at that endpoint"
    /// are different questions, so the record answers them differently:
    /// a status and a body against neither, plus the transport's words.
    #[test]
    fn a_backend_never_reached_is_recorded_as_not_reached() {
        let answer = Answer::Unreachable {
            error: "http POST http://backend.test/generate: connection refused".into(),
        };

        let exchange = record_exchange(
            &SecretGrammar::unauthenticated(),
            None,
            "POST",
            "http://backend.test/generate",
            &sent_headers(),
            None,
            &answer,
        );

        assert!(exchange.status.is_none());
        assert_eq!(exchange.response, Value::Null);
        assert!(
            exchange
                .error
                .expect("the transport says why")
                .contains("connection refused")
        );
    }

    /// A misconfigured endpoint answers with an error page as often as
    /// with JSON, and that page is frequently the most informative thing
    /// about the refusal. Kept as the string it was rather than dropped
    /// for failing to parse.
    #[test]
    fn an_answer_that_is_not_json_is_recorded_as_what_it_was() {
        let answer = Answer::Answered {
            status: reqwest::StatusCode::BAD_GATEWAY,
            text: "<html><body>upstream unavailable</body></html>".into(),
            json: Err("expected value at line 1 column 1".into()),
        };

        let exchange = record_exchange(
            &SecretGrammar::unauthenticated(),
            None,
            "POST",
            "http://backend.test/generate",
            &sent_headers(),
            None,
            &answer,
        );

        assert_eq!(exchange.status, Some(502));
        assert_eq!(
            exchange.response,
            Value::String("<html><body>upstream unavailable</body></html>".into())
        );
    }

    /// The refused path is held to what the accepted one is held to.
    /// Three ways the credential reaches this record without anyone
    /// putting it there: the auth header it was sent in, a URL a profile
    /// rendered `{{secret}}` into, and a backend that quotes the request
    /// it just rejected.
    #[test]
    fn nothing_recorded_about_a_refusal_carries_the_credential() {
        let secret = "test-credential-not-a-real-key";
        let grammar = SecretGrammar::new(secret.into());
        let answer = Answer::Answered {
            status: reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            text: String::new(),
            json: Ok(serde_json::json!({
                "error": "unusable request",
                "echoed": { "url": format!("http://backend.test/generate?key={secret}") },
            })),
        };

        let exchange = record_exchange(
            &grammar,
            Some("authorization"),
            "POST",
            &format!("http://backend.test/generate?key={secret}"),
            &BTreeMap::from([("authorization".into(), format!("Bearer {secret}"))]),
            Some(&serde_json::json!({ "prompt": "a portrait", "key": secret })),
            &answer,
        );

        let rendered = attempt_payload(&exchange).to_string();
        assert!(
            !rendered.contains(secret),
            "the credential reached the record of a refused submit: {rendered}"
        );
        // Scrubbed, not dropped: the record has to stay readable or it
        // stops answering the questions it exists for.
        assert_eq!(exchange.status, Some(422));
        assert_eq!(exchange.response["error"], "unusable request");
        assert_eq!(exchange.request.headers["authorization"], REDACTED);
    }

    /// The record of a refused submit and the record of an accepted one
    /// are read the same way — same key, same shape — which is the
    /// property that makes the two cases equally legible (#76).
    #[test]
    fn the_attempt_payload_speaks_the_handles_own_shape() {
        let exchange = record_exchange(
            &SecretGrammar::unauthenticated(),
            None,
            "POST",
            "http://backend.test/generate",
            &sent_headers(),
            None,
            &Answer::Answered {
                status: reqwest::StatusCode::OK,
                text: r#"{"job_id":"job-1"}"#.into(),
                json: Ok(serde_json::json!({ "job_id": "job-1" })),
            },
        );

        let payload = attempt_payload(&exchange);
        let from_attempt: Exchange =
            serde_json::from_value(payload["exchange"].clone()).expect("the record round-trips");
        let handle_payload = HttpHandlePayload {
            handle: serde_json::json!("job-1"),
            submitted_at_ms: Some(42),
            exchange: Some(exchange),
        };
        let from_handle: Exchange = serde_json::from_value(
            serde_json::to_value(&handle_payload).expect("the handle serialises")["exchange"]
                .clone(),
        )
        .expect("the handle's copy round-trips");

        assert_eq!(from_attempt.status, from_handle.status);
        assert_eq!(from_attempt.response, from_handle.response);
        assert_eq!(from_attempt.request.url, from_handle.request.url);
    }

    /// A handle written before the record grew a status still
    /// rehydrates. A job submitted under the old shape is in flight when
    /// the process carrying this code starts polling it, and a handle
    /// that fails to parse is a job lost to a struct change.
    #[test]
    fn a_handle_from_before_the_status_existed_still_parses() {
        let legacy = serde_json::json!({
            "handle": "job-1",
            "submitted_at_ms": 42,
            "exchange": {
                "request": {
                    "method": "POST",
                    "url": "http://backend.test/jobs",
                    "headers": {},
                },
                "response": { "job_id": "job-1" },
            },
        });

        let payload: HttpHandlePayload =
            serde_json::from_value(legacy).expect("an older handle still rehydrates");
        let exchange = payload.exchange.expect("it carried an exchange");
        assert!(
            exchange.status.is_none(),
            "absent, because nothing recorded one"
        );
        assert_eq!(exchange.response["job_id"], "job-1");
    }
}
