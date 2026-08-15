//! # asterism-exporter-cloud
//!
//! Profile-driven exporter for hosted generation APIs. Like
//! [`asterism_exporter_http`][http] it is configured rather than
//! written — a platform is a JSON profile, not a Rust crate — and it
//! keeps that crate's grammars verbatim by sharing them
//! ([`asterism_exporter_common`]). What it adds is the four things a
//! cloud platform needs and the HTTP exporter does not provide.
//!
//! [http]: https://github.com/ynishi/asterism
//!
//! ## The bytes end up in our custody
//!
//! A hosted platform answers with a URL that expires — ten minutes at
//! one vendor, thirty days at another. The HTTP exporter maps that URL
//! straight into the Derived's locator, so the record names something
//! that will not be there when it is next read. Here a [`FetchSchema`]
//! step pulls the bytes to a path under the custody root before the
//! harvest returns, and *that path* is the locator. The database
//! indexes; the bytes are files.
//!
//! The path is dispatch-addressed —
//! `<custody_root>/dispatch/<dispatch_id>/<nnn>-<name>` — because what
//! the harvest needs to answer is "which files did this dispatch
//! produce". Content addressing is a different question, already
//! answered by the core's digest axes, and can be layered on top
//! without moving anything. Writing is idempotent by path, so a
//! re-collect after a failed fetch overwrites rather than producing a
//! second asset.
//!
//! ## The profile names its secret and never carries it
//!
//! [`AuthSchema::secret_ref`] holds an *environment variable name*. The
//! value is read from the process environment at each call and used to
//! render `{{secret}}` in the auth header — it is not in the params
//! blob, so it is not on the dispatch row, which matters because params
//! are persisted unedited and handed back on every read. Loading a
//! `.env` file is the binary's job, done once at startup: an adapter
//! that went looking for dotenv files itself would make "which file did
//! this credential come from" invisible to the profile that named it.
//!
//! Where the resolved value could leak back out is anything this
//! adapter writes down about the call: the recorded exchange, and the
//! note the harvest puts on each produced asset. The recording redacts
//! the auth header by name *and* scrubs any other occurrence of the
//! value (see [`Exchange`]); the note scrubs what it copies for the same
//! reason, in one place, because a platform that echoes the request is
//! how a query-string credential comes back (see [`call_note`]).
//!
//! ## The profile declares its own deadline
//!
//! No shared default: the measured range across platforms is ten
//! minutes to thirty days, so a constant would be wrong nearly
//! everywhere. [`CloudDispatchParams::deadline_seconds`] is required,
//! and exceeding it fails the job with a message starting
//! [`EXPIRY_PREFIX`] — distinguishable from a backend failure, which is
//! reported in the backend's own words.
//!
//! ## The call is recorded, and it arrives with the artefact
//!
//! The request as sent and the response as received are kept on the
//! dispatch row, in the exporter-owned handle payload the runner already
//! persists and hands back — and the harvest copies that record onto
//! every [`Derived`] it returns, under `extra.cloud.call`, together with
//! the finished job's response whole. The reified asset therefore
//! carries how it was made rather than only a dispatch id to go and ask,
//! and it carries the part that is easiest to lose: the seed the
//! platform ran with and the prompt as it rewrote them are siblings of
//! the artefacts array, so keeping the selected item alone would drop
//! exactly the two values a hosted call cannot reconstruct.
//!
//! Both halves are unconditional, and that is a change from the first
//! cut of this adapter, where recording was a profile flag defaulting to
//! off. A hosted platform hands back a result URL and little else: the
//! model may be an ambient default the provider updates, the seed is an
//! input that is usually not echoed, and an enhanced prompt is not the
//! prompt that was sent. None of it is in the bytes, and none of it can
//! be recovered later by parsing them — the moment of the call is the
//! only moment it exists. A switch that turns off the one capture point
//! turns off the record entirely, and its default decided that for
//! nearly every profile. A profile that still carries the retired flag
//! keeps parsing, whichever way it is set — including `false`, which
//! this build no longer honours.
//!
//! What it costs is honest: a submit body and a submit response ride
//! along in the handle payload the poll loop reads on every tick, and a
//! copy of the record lands on each produced asset. That second copy is
//! per artefact and carries the whole harvest response, so a job with
//! several outputs stores the envelope once per output — for a hosted
//! generation call, kilobytes each. A generation payload large enough to
//! make that the wrong shape would be the measurement that forces a
//! dedicated column, and this adapter has not met one; the ComfyUI-scale
//! workflow blob that would is carried by a different exporter, against
//! a backend that embeds it in the file anyway.

pub mod custody;
pub mod grammar;

use std::collections::BTreeMap;
use std::path::PathBuf;

use asterism_dispatch_sdk::{
    Derived, DispatchContext, DispatchState, Exporter, ExporterError, Handle, ProgressHint,
};
use asterism_exporter_common::{ResponsePath, TemplateAdapter, TemplateEnv};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use custody::CustodyPaths;
pub use grammar::CloudGrammar;

/// Slug the registry uses for this exporter.
pub const SLUG: &str = "cloud";

/// Public name for this exporter's params schema in the
/// `asterism-server schema` CLI (`exporter:cloud:params`).
pub const SCHEMA_NAME: &str = "exporter:cloud:params";

/// Prefix on the `Failed` message when the profile's own deadline was
/// exceeded rather than the backend reporting a failure.
///
/// A prefix rather than a state variant because `DispatchState` is the
/// SDK's vocabulary and one adapter's needs should not widen it before
/// a second adapter wants the same distinction. Public so a caller can
/// tell the two apart without matching on prose.
pub const EXPIRY_PREFIX: &str = "deadline exceeded";

/// What the recorded request substitutes for a redacted header value.
pub const REDACTED: &str = "«redacted»";

/// Canonical example JSON for [`CloudDispatchParams`] — streamed by
/// `asterism-server schema print exporter:cloud:params`.
pub fn params_example_json() -> &'static str {
    include_str!("../schema/cloud_params.example.json")
}

// ---------------------------------------------------------------------------
// Params schema.
// ---------------------------------------------------------------------------

/// One platform, as a profile.
#[derive(Debug, Clone, Deserialize)]
pub struct CloudDispatchParams {
    /// Base URL of the platform (no trailing slash needed).
    pub endpoint: String,
    /// How the credential is named and where it goes.
    pub auth: AuthSchema,
    /// How to submit the job.
    pub submit: SubmitSchema,
    /// How to check on the job.
    pub poll: PollSchema,
    /// How to read the finished job's outputs.
    pub harvest: HarvestSchema,
    /// How to pull the produced bytes into our custody.
    pub fetch: FetchSchema,
    /// How long this platform's job may take before the dispatch is
    /// recorded as expired. Required — see the crate doc for why there
    /// is no default.
    pub deadline_seconds: u64,
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

/// Schema for the submit call.
#[derive(Debug, Clone, Deserialize)]
pub struct SubmitSchema {
    /// HTTP method. Defaults to `POST`.
    #[serde(default = "default_post")]
    pub method: String,
    /// URL path appended to `endpoint` (template-substituted).
    pub path: String,
    /// Optional headers (template-substituted). The auth header is
    /// added on top of these.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// JSON body template. Placeholders are substituted at every
    /// string leaf.
    pub body_template: Value,
    /// Path into the response used to build the persisted handle.
    #[serde(default = "default_dollar")]
    pub handle_from: String,
}

/// Schema for the poll call.
#[derive(Debug, Clone, Deserialize)]
pub struct PollSchema {
    /// HTTP method. Defaults to `GET`.
    #[serde(default = "default_get")]
    pub method: String,
    /// URL path (template-substituted; typically references
    /// `{{handle}}`).
    pub path: String,
    /// Optional headers (template-substituted).
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Predicate — the job is done when this evaluates truthy.
    pub done_when: MatchRule,
    /// Predicate — the job failed when this evaluates truthy. Checked
    /// before `done_when`.
    pub failed_when: MatchRule,
    /// Optional path to a human-readable progress message.
    #[serde(default)]
    pub progress_message_path: Option<String>,
}

/// Schema for the harvest call.
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
    /// Path selecting one element per produced artefact.
    pub items_path: String,
    /// How one element becomes one Derived.
    pub map: DerivedMap,
}

/// How one harvested item becomes one [`Derived`].
#[derive(Debug, Clone, Deserialize)]
pub struct DerivedMap {
    /// Modality slug template (or static string). Defaults to `image`.
    #[serde(default = "default_image")]
    pub modality: String,
    /// Template resolving to the platform's URL for this artefact.
    ///
    /// Not the locator: the locator is where the bytes land after the
    /// fetch step, and this URL is expected to stop working.
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
}

/// How the produced bytes are pulled into our custody.
#[derive(Debug, Clone, Deserialize)]
pub struct FetchSchema {
    /// Send the auth header on the download too. One platform's
    /// download needs the API key; another's URLs are public and
    /// reject the header.
    #[serde(default)]
    pub authenticated: bool,
    /// Extra headers on the download (template-substituted).
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// A poll predicate: the value at `path` equals `equals`.
#[derive(Debug, Clone, Deserialize)]
pub struct MatchRule {
    /// Path into the poll response.
    pub path: String,
    /// Value the path has to equal for the rule to fire.
    pub equals: Value,
    /// Optional path to a message extracted when the rule fires.
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

// ---------------------------------------------------------------------------
// Handle payload.
// ---------------------------------------------------------------------------

/// What this exporter persists between calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudHandlePayload {
    /// Value extracted via `submit.handle_from`.
    pub handle: Value,
    /// When the backend accepted the job — the deadline is measured
    /// from here, so it has to survive a restart with the handle.
    pub submitted_at_ms: i64,
    /// The recorded exchange. Written on every submit; optional only so
    /// that a job submitted before recording became unconditional still
    /// rehydrates on the poll after the upgrade, rather than failing as
    /// a corrupt handle for want of a field its submit never wrote.
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exchange {
    /// Submit request, as sent.
    pub request: RecordedRequest,
    /// Submit response, as received.
    pub response: Value,
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

// ---------------------------------------------------------------------------
// Exporter.
// ---------------------------------------------------------------------------

/// Profile-driven exporter for hosted generation APIs.
#[derive(Debug, Clone)]
pub struct CloudExporter {
    http: reqwest::Client,
    custody: CustodyPaths,
}

impl CloudExporter {
    /// Builds an exporter that takes custody of produced files under
    /// `custody_root` (the application directory).
    pub fn new(custody_root: PathBuf) -> Self {
        Self::with_client(
            custody_root,
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client build"),
        )
    }

    /// Uses a caller-supplied `reqwest::Client`.
    pub fn with_client(custody_root: PathBuf, http: reqwest::Client) -> Self {
        Self {
            http,
            custody: CustodyPaths::new(custody_root),
        }
    }

    /// Resolves the profile's credential from the environment.
    ///
    /// Fails naming the variable, because the profile named it: an
    /// author who mistyped `FAL_KEY` learns which spelling the process
    /// looked for.
    fn grammar(&self, params: &CloudDispatchParams) -> Result<CloudGrammar, ExporterError> {
        let secret = std::env::var(&params.auth.secret_ref).map_err(|_| {
            ExporterError::BackendRejected(format!(
                "auth.secret_ref names environment variable {:?}, which is not set",
                params.auth.secret_ref
            ))
        })?;
        Ok(CloudGrammar::new(secret))
    }

    /// Headers for one call: the profile's own, plus the auth header.
    fn headers(
        &self,
        grammar: &CloudGrammar,
        params: &CloudDispatchParams,
        own: &BTreeMap<String, String>,
        env: &TemplateEnv<'_>,
    ) -> Result<BTreeMap<String, String>, ExporterError> {
        let mut headers = grammar.render_headers(own, env)?;
        headers.insert(
            params.auth.header.clone(),
            grammar.render(&params.auth.value_template, env)?,
        );
        Ok(headers)
    }
}

#[async_trait]
impl Exporter for CloudExporter {
    fn slug(&self) -> &str {
        SLUG
    }

    fn accepts(&self, _action: &str) -> bool {
        // Actions are opaque to a profile-driven exporter, as they are
        // to the HTTP one: the action string is recorded on the job row
        // for audit granularity, and the profile decides what runs.
        true
    }

    async fn dispatch(&self, ctx: DispatchContext<'_>) -> Result<Handle, ExporterError> {
        let params = parse_params(&ctx)?;
        let grammar = self.grammar(&params)?;
        let env = TemplateEnv::pre_handle(&ctx, ctx.params);

        let url = format!(
            "{}{}",
            trim_trailing_slash(&params.endpoint),
            grammar.render(&params.submit.path, &env)?
        );
        let body = grammar.render_json(&params.submit.body_template, &env)?;
        let headers = self.headers(&grammar, &params, &params.submit.headers, &env)?;

        let response = send_json(
            &self.http,
            &params.submit.method,
            &url,
            &headers,
            Some(body.clone()),
        )
        .await
        .map_err(|e| grammar.scrub_error(e))?;

        let handle = grammar
            .select_first(&response, &params.submit.handle_from)
            .ok_or_else(|| {
                ExporterError::BackendRejected(format!(
                    "submit response missing handle_from path {:?}",
                    params.submit.handle_from
                ))
            })?;

        let exchange = Some(Exchange {
            request: RecordedRequest {
                method: params.submit.method.clone(),
                // Scrubbed, not copied: query-parameter auth is a real
                // shape, so a profile may legitimately have rendered
                // `{{secret}}` into this string.
                url: grammar.scrub_text(&url),
                headers: grammar.redact_headers(&headers, &params.auth.header),
                body: Some(grammar.scrub(body)),
            },
            response: grammar.scrub(response),
        });

        Ok(Handle::new(
            SLUG,
            serde_json::to_value(CloudHandlePayload {
                handle,
                submitted_at_ms: Utc::now().timestamp_millis(),
                exchange,
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
        let expiry = expiry_state(&params, &payload, Utc::now().timestamp_millis());

        let grammar = self.grammar(&params)?;
        let env = TemplateEnv::with_handle(&ctx, ctx.params, &payload.handle);
        let url = format!(
            "{}{}",
            trim_trailing_slash(&params.endpoint),
            grammar.render(&params.poll.path, &env)?
        );
        let headers = self.headers(&grammar, &params, &params.poll.headers, &env)?;

        // The backend is asked even when the deadline has passed, and
        // the expiry is only the answer if the backend does not have a
        // better one. A job that finished a second late finished; a
        // deadline is our rule about how long to keep waiting, not a
        // claim about what the platform did. When the call itself fails
        // past the deadline — the platform has forgotten the job, which
        // is what a deadline predicts — the expiry is the more useful
        // explanation than the 404.
        let response = match send_json(&self.http, &params.poll.method, &url, &headers, None).await
        {
            Ok(response) => response,
            Err(err) => return expiry.ok_or_else(|| grammar.scrub_error(err)),
        };

        if match_rule(&grammar, &params.poll.failed_when, &response) {
            let message = params
                .poll
                .failed_when
                .message_path
                .as_deref()
                .and_then(|p| grammar.select_first(&response, p))
                .and_then(|v| grammar.display_string(v))
                .unwrap_or_else(|| "backend reported failure".into());
            return Ok(DispatchState::Failed { message });
        }
        if match_rule(&grammar, &params.poll.done_when, &response) {
            return Ok(DispatchState::Done);
        }
        // Still working, and out of time: this is where the deadline
        // earns its keep, by ending a loop that would otherwise re-poll
        // a queue position forever.
        if let Some(expired) = expiry {
            return Ok(expired);
        }
        Ok(DispatchState::Running(ProgressHint {
            current: None,
            total: None,
            message: params
                .poll
                .progress_message_path
                .as_deref()
                .and_then(|p| grammar.select_first(&response, p))
                .and_then(|v| grammar.display_string(v)),
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

        let url = format!(
            "{}{}",
            trim_trailing_slash(&params.endpoint),
            grammar.render(&params.harvest.path, &env)?
        );
        let headers = self.headers(&grammar, &params, &params.harvest.headers, &env)?;
        let response = send_json(&self.http, &params.harvest.method, &url, &headers, None)
            .await
            .map_err(|e| grammar.scrub_error(e))?;

        let items = grammar.select(&response, &params.harvest.items_path);
        let now = Utc::now();
        // Built once and cloned per item: every artefact of one job came
        // out of the same call, and re-deriving it inside the loop would
        // invite the two copies to drift.
        let call = call_note(&grammar, &payload, response);
        let mut out: Vec<Derived> = Vec::with_capacity(items.len());

        for (index, item) in items.iter().enumerate() {
            let item_env = env.with_item(item);
            let map = &params.harvest.map;
            let source_url = grammar.render(&map.source_url, &item_env)?;

            // The fetch is the whole point of this adapter: the URL
            // above is expected to expire, so the Derived names what we
            // wrote, not what the platform served.
            let mut fetch_headers = grammar.render_headers(&params.fetch.headers, &item_env)?;
            if params.fetch.authenticated {
                fetch_headers.insert(
                    params.auth.header.clone(),
                    grammar.render(&params.auth.value_template, &item_env)?,
                );
            }
            let bytes = fetch_bytes(&self.http, &source_url, &fetch_headers)
                .await
                .map_err(|e| grammar.scrub_error(e))?;
            let locator = self
                .custody
                .write(ctx.dispatch_id, index, &source_url, &bytes)
                .await?;

            let mut labels = Vec::with_capacity(map.labels_static.len());
            for template in &map.labels_static {
                labels.push(grammar.render(template, &item_env)?);
            }

            out.push(Derived {
                modality: grammar.render(&map.modality, &item_env)?,
                locator: locator.to_string_lossy().into_owned(),
                occurred_at: now,
                cover_hint: map
                    .cover_hint
                    .as_deref()
                    .map(|t| grammar.render(t, &item_env))
                    .transpose()?,
                register_note: map
                    .register_note
                    .as_deref()
                    .map(|t| grammar.render(t, &item_env))
                    .transpose()?,
                labels,
                file_size_bytes: Some(bytes.len() as u64),
                duration_ms: None,
                // The platform's own item, and the URL it is expected to
                // stop serving, kept beside the file we now hold. Both
                // scrubbed: a download authenticated by query parameter
                // renders the credential into that URL, and this one is
                // persisted on the Asset rather than the dispatch.
                extra: serde_json::json!({
                    "cloud": {
                        "item": grammar.scrub(item.clone()),
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

fn parse_params(ctx: &DispatchContext<'_>) -> Result<CloudDispatchParams, ExporterError> {
    serde_json::from_value(ctx.params.clone())
        .map_err(|e| ExporterError::BackendRejected(format!("invalid cloud params: {e}")))
}

fn parse_handle_payload(handle: &Handle) -> Result<CloudHandlePayload, ExporterError> {
    serde_json::from_value(handle.payload.clone())
        .map_err(|e| ExporterError::BackendRejected(format!("corrupt cloud handle: {e}")))
}

/// The call that produced a harvest, as it is written onto every
/// artefact that came out of it.
///
/// A projection of the handle payload rather than the payload itself:
/// the payload is this adapter's working state, and what an asset should
/// carry is the record of the call — which job the platform gave us,
/// when we asked, what we sent, what came back. Serialising the payload
/// wholesale would put the first shape on assets and make every later
/// working-state field a wire change nobody asked for.
///
/// `result` is the harvest response whole, and not only because it is
/// cheap to keep: the fields a re-run would need are routinely *outside*
/// the items array. The seed the platform actually used, the prompt as
/// it rewrote it, the timings — these are siblings of the array, so an
/// adapter that kept the selected item and dropped the envelope would
/// discard the two values a hosted call is least able to reconstruct.
/// The item stays beside it because which of the returned artefacts this
/// asset is cannot be read back off the envelope.
///
/// `request` and `response` are absent together, and only for a job
/// submitted before recording became unconditional. Absent here is the
/// literal "this handle predates the record" rather than a claim about
/// the call, and it is written as a missing key rather than a null for
/// the reason the disclosure vocabulary states: a null reads as a value
/// somebody wrote.
///
/// # Why the scrub happens here and not at each caller
///
/// The two values this builds from that have not already been through
/// the grammar are the harvest response and the handle, and the handle
/// is the one that does not look like it needs it. `submit.handle_from`
/// defaults to `$` — the whole submit response — and a platform that
/// echoes the request it was sent then puts a `{{secret}}` rendered into
/// a query string or a body field inside the value this note copies onto
/// an asset. Taking both raw and scrubbing them in one place is what
/// stops the next field added here from being the one that forgets.
fn call_note(grammar: &CloudGrammar, payload: &CloudHandlePayload, result: Value) -> Value {
    let mut note = serde_json::json!({
        "handle": grammar.scrub(payload.handle.clone()),
        "submitted_at_ms": payload.submitted_at_ms,
        "result": grammar.scrub(result),
    });
    if let (Some(exchange), Some(map)) = (payload.exchange.as_ref(), note.as_object_mut()) {
        map.insert(
            "request".into(),
            serde_json::to_value(&exchange.request).expect("a recorded request serialises"),
        );
        map.insert("response".into(), exchange.response.clone());
    }
    note
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

fn match_rule(grammar: &CloudGrammar, rule: &MatchRule, response: &Value) -> bool {
    match grammar.select_first(response, &rule.path) {
        Some(actual) => actual == rule.equals,
        None => false,
    }
}

/// `Some(Failed)` once the profile's deadline has passed.
///
/// Split out and taking `now_ms` so the rule is testable without
/// waiting out a deadline or reaching for a clock abstraction the rest
/// of the crate does not need.
pub fn expiry_state(
    params: &CloudDispatchParams,
    payload: &CloudHandlePayload,
    now_ms: i64,
) -> Option<DispatchState> {
    let elapsed_ms = now_ms.saturating_sub(payload.submitted_at_ms);
    let deadline_ms = (params.deadline_seconds as i64).saturating_mul(1000);
    (elapsed_ms > deadline_ms).then(|| DispatchState::Failed {
        message: format!(
            "{EXPIRY_PREFIX}: {}s since submit, profile allows {}s",
            elapsed_ms / 1000,
            params.deadline_seconds
        ),
    })
}

async fn send_json(
    http: &reqwest::Client,
    method: &str,
    url: &str,
    headers: &BTreeMap<String, String>,
    body: Option<Value>,
) -> Result<Value, ExporterError> {
    let response = send(http, method, url, headers, body).await?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| ExporterError::Other(anyhow::anyhow!("{method} {url}: {e}")))?;
    if !status.is_success() {
        return Err(ExporterError::BackendRejected(format!(
            "{method} {url} HTTP {status}: {text}"
        )));
    }
    serde_json::from_str(&text).map_err(|e| {
        ExporterError::BackendRejected(format!("{method} {url} response not JSON: {e}"))
    })
}

async fn fetch_bytes(
    http: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
) -> Result<Vec<u8>, ExporterError> {
    let response = send(http, "GET", url, headers, None).await?;
    let status = response.status();
    if !status.is_success() {
        // Worth saying which URL: at this point the job succeeded and
        // only the download failed, and the two are told apart by what
        // the message names.
        return Err(ExporterError::BackendRejected(format!(
            "fetching {url} returned HTTP {status}"
        )));
    }
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| ExporterError::Other(anyhow::anyhow!("fetching {url}: {e}")))
}

async fn send(
    http: &reqwest::Client,
    method: &str,
    url: &str,
    headers: &BTreeMap<String, String>,
    body: Option<Value>,
) -> Result<reqwest::Response, ExporterError> {
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
    let mut request = http.request(http_method, url);
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    request
        .send()
        .await
        .map_err(|e| ExporterError::Other(anyhow::anyhow!("{method} {url}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params_fixture() -> CloudDispatchParams {
        serde_json::from_str(params_example_json()).expect("the shipped example parses")
    }

    #[test]
    fn slug_is_stable_and_action_open() {
        let exporter = CloudExporter::new(PathBuf::from("/tmp/asterism-test"));
        assert_eq!(exporter.slug(), SLUG);
        assert!(exporter.accepts("txt2img"));
        assert!(exporter.accepts("anything"));
    }

    /// The shipped example is what `asterism-server schema print
    /// exporter:cloud:params` hands a profile author, so it has to
    /// survive the same parse the exporter runs.
    #[test]
    fn params_example_deserialises_into_the_current_struct() {
        let params = params_fixture();
        assert_eq!(params.auth.secret_ref, "FAL_KEY");
        assert_eq!(params.deadline_seconds, 86_400);
        assert_eq!(params.submit.handle_from, "$.request_id");
        assert_eq!(params.harvest.items_path, "$.images[*]");
        assert_eq!(params.harvest.map.source_url, "{{item.url}}");
        // fal.ai serves its outputs from a public CDN, so the download
        // must not carry the key. A platform whose downloads are
        // authenticated flips this one field.
        assert!(!params.fetch.authenticated);
    }

    /// A profile carrying a credential instead of naming one is the
    /// mistake this adapter exists to make impossible, so the shipped
    /// example must not teach it: nothing in the example may look like
    /// a secret value, and the auth value template must go through
    /// `{{secret}}`.
    #[test]
    fn the_example_names_a_variable_rather_than_carrying_a_value() {
        let params = params_fixture();
        assert!(
            params.auth.value_template.contains("{{secret}}"),
            "auth.value_template has to reach the credential through {{{{secret}}}}"
        );
        let raw = params_example_json();
        for forbidden in ["api_key", "apikey", "token\":", "secret\":"] {
            assert!(
                !raw.to_ascii_lowercase().contains(forbidden),
                "the example must not model carrying a credential ({forbidden:?})"
            );
        }
    }

    #[test]
    fn a_job_expires_only_after_its_own_deadline() {
        let params = params_fixture();
        let payload = CloudHandlePayload {
            handle: serde_json::json!("req-1"),
            submitted_at_ms: 1_000_000,
            exchange: None,
        };
        let deadline_ms = params.deadline_seconds as i64 * 1000;

        assert!(expiry_state(&params, &payload, 1_000_000 + deadline_ms).is_none());
        let expired = expiry_state(&params, &payload, 1_000_000 + deadline_ms + 1)
            .expect("one millisecond past the deadline expires");
        match expired {
            DispatchState::Failed { message } => {
                assert!(
                    message.starts_with(EXPIRY_PREFIX),
                    "an expiry has to be tellable from a backend failure: {message}"
                );
                assert!(message.contains("86400s"), "{message}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// Recording used to be `"record_exchange": true` in the profile.
    /// The field is gone, and a profile in the field that still carries
    /// it has to keep working — it says the thing this build now does
    /// anyway, and failing a live profile over a retired flag would be
    /// this change breaking the configurations that had already asked
    /// for the behaviour it makes unconditional.
    ///
    /// `false` is the arm worth pinning: that profile said "do not
    /// record", and this build records anyway. Parsing it rather than
    /// refusing it is the decision — a live profile is not failed over a
    /// setting that no longer exists — and the operator learns the
    /// behaviour changed from the changelog, not from a broken dispatch.
    #[test]
    fn a_profile_still_carrying_the_retired_flag_parses() {
        for setting in [true, false] {
            let mut raw: Value =
                serde_json::from_str(params_example_json()).expect("example parses");
            raw.as_object_mut()
                .expect("the example is an object")
                .insert("record_exchange".into(), Value::Bool(setting));
            let params: CloudDispatchParams =
                serde_json::from_value(raw).expect("a retired flag is ignored, not refused");
            assert_eq!(params.deadline_seconds, 86_400);
        }
    }

    fn grammar_fixture() -> CloudGrammar {
        CloudGrammar::new("sk-live-not-a-real-key".into())
    }

    fn exchange_fixture() -> Exchange {
        Exchange {
            request: RecordedRequest {
                method: "POST".into(),
                url: "https://queue.example/run".into(),
                headers: BTreeMap::from([("authorization".into(), REDACTED.into())]),
                body: Some(serde_json::json!({ "prompt": "a portrait", "seed": 7 })),
            },
            response: serde_json::json!({ "request_id": "req-1", "status": "IN_QUEUE" }),
        }
    }

    /// Shaped like a hosted platform's finished job: the artefacts in an
    /// array, and the two values a re-run would need — the seed it
    /// actually used, the prompt as it rewrote it — outside that array.
    fn harvest_result_fixture() -> Value {
        serde_json::json!({
            "images": [{ "url": "https://cdn.example/a.png" }],
            "seed": 913_224,
            "prompt": "photo studio portrait, soft key light, 85mm",
        })
    }

    /// What the artefact has to carry away from the call: the platform's
    /// own id for the job, when it was asked for, the request as sent —
    /// the only place the prompt and the seed exist, since the file will
    /// not have them — and the response that named the job.
    #[test]
    fn the_call_note_carries_what_was_sent_and_what_came_back() {
        let note = call_note(
            &grammar_fixture(),
            &CloudHandlePayload {
                handle: serde_json::json!("req-1"),
                submitted_at_ms: 42,
                exchange: Some(exchange_fixture()),
            },
            harvest_result_fixture(),
        );

        assert_eq!(note["handle"], serde_json::json!("req-1"));
        assert_eq!(note["submitted_at_ms"], serde_json::json!(42));
        assert_eq!(note["request"]["body"]["prompt"], "a portrait");
        assert_eq!(note["request"]["body"]["seed"], 7);
        assert_eq!(note["response"]["request_id"], "req-1");
        // The redaction happened when the exchange was recorded; the
        // note must not undo it by reaching past the recorded copy.
        assert_eq!(note["request"]["headers"]["authorization"], REDACTED);
    }

    /// The envelope, not just the item that became this asset: what the
    /// platform decided — the seed it ran with, the prompt it rewrote —
    /// sits beside the artefacts array and is gone from every other
    /// surface once the job ages out.
    #[test]
    fn the_call_note_keeps_what_the_platform_decided_beside_the_artefacts() {
        let note = call_note(
            &grammar_fixture(),
            &CloudHandlePayload {
                handle: serde_json::json!("req-1"),
                submitted_at_ms: 42,
                exchange: Some(exchange_fixture()),
            },
            harvest_result_fixture(),
        );

        assert_eq!(note["result"]["seed"], 913_224);
        assert_eq!(
            note["result"]["prompt"],
            "photo studio portrait, soft key light, 85mm"
        );
    }

    /// A job submitted by the previous build and harvested by this one
    /// has no recorded exchange. It still gets a note — the job id and
    /// the submit time are on the handle either way — and the two keys
    /// it cannot fill stay absent rather than landing as nulls that read
    /// as "the platform returned nothing".
    #[test]
    fn a_handle_from_before_the_record_notes_the_call_it_can() {
        let note = call_note(
            &grammar_fixture(),
            &CloudHandlePayload {
                handle: serde_json::json!("req-old"),
                submitted_at_ms: 7,
                exchange: None,
            },
            harvest_result_fixture(),
        );

        assert_eq!(note["handle"], serde_json::json!("req-old"));
        assert_eq!(note["submitted_at_ms"], serde_json::json!(7));
        // The harvest is this build's, so its half of the record lands
        // even when the submit's half cannot.
        assert_eq!(note["result"]["seed"], 913_224);
        let map = note.as_object().expect("the note is an object");
        assert!(!map.contains_key("request"), "{note}");
        assert!(!map.contains_key("response"), "{note}");
    }

    /// The note is written onto an asset, so it is held to what the
    /// recorded exchange is held to. Two ways the credential reaches it
    /// without anyone putting it there: `submit.handle_from` defaults to
    /// `$`, which makes the handle the whole submit response, and a
    /// platform is free to echo the request — including a URL a profile
    /// rendered `{{secret}}` into.
    #[test]
    fn the_note_cannot_carry_the_credential_a_platform_echoed_back() {
        let secret = "sk-live-not-a-real-key";
        let note = call_note(
            &grammar_fixture(),
            &CloudHandlePayload {
                handle: serde_json::json!({
                    "request_id": "req-1",
                    "echo": { "url": format!("https://queue.example/run?key={secret}") },
                }),
                submitted_at_ms: 42,
                exchange: Some(exchange_fixture()),
            },
            serde_json::json!({
                "images": [{ "url": format!("https://cdn.example/a.png?token={secret}") }],
                "seed": 913_224,
            }),
        );

        let rendered = note.to_string();
        assert!(
            !rendered.contains(secret),
            "the credential reached an asset's note: {rendered}"
        );
        // Scrubbed, not dropped: what the platform said still has to be
        // readable, or the note stops being the record it exists to be.
        assert_eq!(note["handle"]["request_id"], "req-1");
        assert_eq!(note["result"]["seed"], 913_224);
    }

    /// The runner rehydrates this from the row on every tick, so a
    /// payload that does not round-trip loses the deadline's origin.
    #[test]
    fn the_handle_payload_round_trips() {
        let payload = CloudHandlePayload {
            handle: serde_json::json!({ "request_id": "req-1" }),
            submitted_at_ms: 42,
            exchange: Some(Exchange {
                request: RecordedRequest {
                    method: "POST".into(),
                    url: "https://queue.example/run".into(),
                    headers: BTreeMap::from([("authorization".into(), REDACTED.into())]),
                    body: Some(serde_json::json!({ "prompt": "a portrait" })),
                },
                response: serde_json::json!({ "request_id": "req-1" }),
            }),
        };
        let back: CloudHandlePayload =
            serde_json::from_value(serde_json::to_value(&payload).unwrap()).unwrap();
        assert_eq!(back.submitted_at_ms, 42);
        assert_eq!(
            back.exchange.unwrap().request.headers["authorization"],
            REDACTED
        );
    }
}
