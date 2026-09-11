//! # asterism-exporter-transfer
//!
//! One adapter for the one channel every stock agency sanctions for a
//! batch: the files on the agency's host over SFTP, FTPS or FTP, with a
//! CSV sidecar beside them. The scheme in the profile's endpoint chooses
//! which — one crate for all of them, the way
//! [`asterism_exporter_http`] is one crate for hosted and self-hosted
//! job APIs, because a host, a credential and a directory layout are the
//! whole of what differs. [`Scheme`] is the list.
//!
//! ## What it sends, and what it does not
//!
//! **The bytes are a release's stamped copies, named by path.** They
//! reach this adapter in [`RESERVED_KEY`]`.files`, which the send writes
//! and a profile may not — `asterism_core::application::send_service` is
//! where that is argued and where the refusal lives. Nothing here reads
//! `input.source_locator`: the inputs are the library rows the copies
//! were made from, and they are what the sidecar's columns are rendered
//! against, not where the bytes come from.
//!
//! A profile that names no file list is refused. There is no second
//! meaning for it — an adapter that fell back to the inputs would send
//! the library's own unstamped originals.
//!
//! ## Params schema
//!
//! `CreateDispatchCommand.params_json` deserialises into
//! [`TransferDispatchParams`]:
//!
//! ```json
//! {
//!   "endpoint": "sftp://stock.example.com:22/incoming/2026-09",
//!   "auth": {
//!     "user":               "contributor",
//!     "secret_ref":         "AGENCY_SFTP_PASSWORD",
//!     "key_ref":            "AGENCY_SFTP_KEY",
//!     "key_passphrase_ref": "AGENCY_SFTP_KEY_PASSPHRASE"
//!   },
//!   "host_key": { "fingerprint": "SHA256:0000000000000000000000000000000000000000000" },
//!   "allow_insecure": false,
//!   "remote_name_template": "{{item.name}}",
//!   "sidecar": {
//!     "filename": "metadata.csv",
//!     "columns": [
//!       { "header": "Filename", "template": "{{item.remote_name}}" },
//!       { "header": "Title",    "template": "{{item.card.title?}}" }
//!     ]
//!   }
//! }
//! ```
//!
//! - `endpoint` — `<scheme>://<host>[:<port>][/<dir>]`. The scheme
//!   chooses the protocol, and the schemes are `sftp`, `ftps`, `ftp` and
//!   `file`; [`Scheme`] is where each says what it costs, and
//!   [`transport::read_endpoint`] has the grammar and the reason an
//!   endpoint carries no account.
//! - `auth` — the account, and the *names* of the environment variables
//!   the credential is read from. Absent means the server takes an
//!   anonymous login, which is the FTP shape and not much else.
//! - `host_key` — what the far side's key has to be. Required for
//!   `sftp://`. The other schemes authenticate their host through TLS or
//!   not at all, so a well-formed `host_key` beside one of them is
//!   ignored — but a malformed one is refused whichever scheme it sits
//!   with, because it is read before the scheme is consulted and naming
//!   neither or both of its two forms is a profile that has not decided.
//! - `allow_insecure` — permission to speak `ftp://`, where the
//!   credential and the bytes cross the network in the clear.
//! - `remote_name_template` — what each file is called on the far side.
//!   Absent means the copy's own basename.
//! - `sidecar` — the CSV that goes beside the files. Its columns are the
//!   agency's, and the tree carries no agency's column set: the
//!   disclosure keyword an agency reads (Freepik's `_ai_generated`) is a
//!   column somebody's profile chose, exactly like every other one.
//!
//! `schema/transfer_params.example.json` is the runnable version of this
//! shape, and the tests at the bottom of this file are what keep it
//! honest.
//!
//! ### Templates
//!
//! The `{{...}}` grammar is the shared one, documented where it is
//! defined: [`asterism_exporter_common::template`]. What this adapter
//! binds `{{item}}` to is [`FileRow::item`] — one file's row plus the
//! card of the input it came from — and it binds it in the same shape
//! for `remote_name_template` and for every sidecar column.
//!
//! ## Where a credential lives
//!
//! In an environment variable, named by the profile and resolved per
//! call, for the reason `asterism_exporter_http`'s crate doc gives at
//! length: params are persisted unedited and handed back on every read
//! of the dispatch, so a value reachable by `{{params.…}}` is readable
//! by anything that can list dispatches. `key_ref` is the same rule one
//! step along — it names a variable holding the key's *location*, so
//! neither the key nor the path to it is on a row. The path is scrubbed
//! alongside the password and the passphrase rather than trusted to stay
//! out of a message: [`Credentials::secrets`] is that list.
//!
//! ## Two refusals that happen before anything is sent
//!
//! **`ftp://` needs `allow_insecure`.** The default answer to a
//! credential over cleartext is no, and a profile that means it says so
//! in one field.
//!
//! **An SFTP host is the host the profile names.** The fingerprint or
//! the `known_hosts` entry is checked as the connection opens, and a key
//! that does not match ends the dispatch with nothing put. There is no
//! prompt: a dispatch runs in a worker with nobody in front of it, and
//! "accept and remember" is a decision that would be made by the
//! absence of anyone to make it.
//!
//! Both are recorded on the attempt before the error is returned, so a
//! reader of the dispatch sees which refusal it was rather than a
//! message alone — and so is every other answer given between reading
//! the params and the first successful put, down to a blob that did not
//! parse. [`refuse`] is the one arm those leave through. On either side
//! of that span the shape is different and deliberately so: an action
//! this adapter does not take is the SDK's own variant and is answered
//! before anything is read, and a put that failed is one row among the
//! per-file ones below.
//!
//! ## The call is recorded per file
//!
//! [`AttemptRecord`] carries what was sent, under what name, and what
//! the server answered for each file and for the sidecar — with the
//! credential redacted, and the environment variable *names* kept so a
//! reader can tell which profile was in play. A put that failed part way
//! through leaves a record of every file either way: the run failed with
//! the first error, and what actually landed is a question only the
//! record can answer.
//!
//! The redaction is applied once per exit rather than per message,
//! wherever a record or an error leaves this crate, and it looks for
//! everything [`Credentials::secrets`] names —
//! [`Redaction`](asterism_exporter_common::Redaction) is what it is for.
//!
//! ## Lifecycle
//!
//! The whole transfer happens inside [`dispatch`](TransferExporter::dispatch),
//! the way `asterism-exporter-file`'s writes do: `poll` answers
//! [`DispatchState::Done`] at once, and `harvest` returns no
//! [`Derived`](asterism_dispatch_sdk::Derived). Nothing was made. A copy
//! on somebody else's host is the same content that left, and minting an
//! asset for it would put a second row in the library for every file
//! sent.

#[cfg(test)]
mod fake;
pub mod ftp;
pub mod local;
pub mod sftp;
pub mod transport;

use std::sync::Arc;

use asterism_contract::dto::AssetCardDto;
use asterism_dispatch_sdk::{
    AttemptRecord, Derived, DispatchContext, DispatchState, Exporter, ExporterError, Handle,
};
use asterism_exporter_common::{CommonExportAdapter, Redaction, TemplateAdapter, TemplateEnv};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use transport::{
    Connector, Credentials, HostKey, Scheme, Target, Transport, TransportError, read_endpoint,
};

/// Slug the registry uses for this exporter.
pub const SLUG: &str = "transfer";

/// The single action name — a send puts a set on a host, and what
/// differs between destinations is the profile rather than the verb.
pub const ACTION_PUT: &str = "put";

/// Public name for this exporter's params schema in the
/// `asterism-server schema` CLI (`exporter:transfer:params`).
pub const SCHEMA_NAME: &str = "exporter:transfer:params";

/// The params key the send writes the release's file list under.
///
/// Spelled here and in `asterism_core::application::send_service`, which
/// is the writer and where the argument for a reserved key is. The two
/// crates cannot see each other — the core takes no exporter dependency
/// — so the word is agreed the way `"file"` and `"write"` already are
/// between that layer and `asterism-exporter-file`.
pub const RESERVED_KEY: &str = "release";

/// Canonical example JSON for [`TransferDispatchParams`] — streamed by
/// `asterism-server schema print exporter:transfer:params`.
pub fn params_example_json() -> &'static str {
    include_str!("../schema/transfer_params.example.json")
}

/// Params schema for [`SLUG`] dispatch calls.
#[derive(Debug, Clone, Deserialize)]
pub struct TransferDispatchParams {
    /// Where the files go: `<scheme>://<host>[:<port>][/<dir>]`.
    pub endpoint: String,
    /// The account, and the environment variables its credential is
    /// read from.
    #[serde(default)]
    pub auth: Option<AuthSchema>,
    /// What the far side's host key has to be.
    #[serde(default)]
    pub host_key: Option<HostKeySchema>,
    /// Permission to speak `ftp://`.
    #[serde(default)]
    pub allow_insecure: bool,
    /// What each file is called on the far side. Absent means the
    /// copy's own basename.
    #[serde(default)]
    pub remote_name_template: Option<String>,
    /// The CSV that goes beside the files.
    pub sidecar: SidecarSchema,
    /// The file list, written by the send.
    #[serde(rename = "release")]
    pub release: ReleaseFiles,
}

/// The account and where its credential is read from.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthSchema {
    /// The account name on the far side.
    pub user: String,
    /// **Name of an environment variable** holding the password.
    #[serde(default)]
    pub secret_ref: Option<String>,
    /// **Name of an environment variable** holding the path to the
    /// private key.
    #[serde(default)]
    pub key_ref: Option<String>,
    /// **Name of an environment variable** holding that key's
    /// passphrase.
    #[serde(default)]
    pub key_passphrase_ref: Option<String>,
}

/// What the far side's host key has to be.
///
/// Exactly one of the two. Both would be two answers to one question,
/// and neither is the profile not having decided.
#[derive(Debug, Clone, Deserialize)]
pub struct HostKeySchema {
    /// The expected fingerprint, as OpenSSH spells one.
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// A `known_hosts` file to look the host up in.
    #[serde(default)]
    pub known_hosts: Option<String>,
}

/// The CSV that goes beside the files.
///
/// One per send. An agency that caps a batch's rows caps it at a number
/// that agency changes on its own schedule, and splitting here would put
/// that number in the tree; a profile that has to send fewer files sends
/// fewer files, which is a second release and a second send.
#[derive(Debug, Clone, Deserialize)]
pub struct SidecarSchema {
    /// What the CSV is called on the far side.
    pub filename: String,
    /// The columns, in the order they are written.
    pub columns: Vec<SidecarColumn>,
}

/// One column: a header, and how each row's cell is built.
#[derive(Debug, Clone, Deserialize)]
pub struct SidecarColumn {
    /// The header, written verbatim in the first row.
    pub header: String,
    /// A `{{...}}` template, rendered once per file.
    pub template: String,
}

/// The release's file list, as the send wrote it.
#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseFiles {
    /// One entry per copy, in the order the release recorded them.
    pub files: Vec<FileRow>,
}

/// One copy: which library row it came from, where it is, what it is
/// called.
#[derive(Debug, Clone, Deserialize)]
pub struct FileRow {
    /// The library row the copy was made from, so a sidecar column can
    /// reach that row's card.
    pub asset_id: String,
    /// Where the copy is on this machine.
    pub path: String,
    /// What the copy is called.
    pub name: String,
}

impl FileRow {
    /// What `{{item}}` resolves against while this file is the one being
    /// sent.
    ///
    /// The row's own fields, the name it will land under, its position
    /// in the send, and `card` — the input this copy was made from, as
    /// the same [`AssetCardDto`] the grid shows. A column reaches the
    /// card through `{{item.card.…}}` rather than through a second set
    /// of top-level names, so a field the DTO grows is reachable the day
    /// it lands and cannot collide with a field of the row.
    ///
    /// The card is not optional: a row naming no input of this dispatch
    /// is refused before anything is planned, for the reason
    /// [`plan_send`] gives.
    pub fn item(&self, index: usize, remote_name: &str, card: &AssetCardDto) -> Value {
        serde_json::json!({
            "asset_id": self.asset_id,
            "path": self.path,
            "name": self.name,
            "remote_name": remote_name,
            "index": index,
            "card": serde_json::to_value(card).unwrap_or(Value::Null),
        })
    }
}

/// Payload persisted on the returned [`Handle`].
///
/// What landed, so a reader of the dispatch sees the shape of the send
/// without going to the attempt record for it. No credential and no
/// server answer: the answers are on the record, which is scrubbed.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TransferHandlePayload {
    /// The directory on the far side.
    directory: String,
    /// How many files were put, the sidecar not counted.
    sent: usize,
    /// What the sidecar is called.
    sidecar: String,
}

/// Transport-backed [`Exporter`].
///
/// Holds the thing that opens connections rather than opening them
/// itself — see [`transport`] for why that seam is where it is.
#[derive(Clone)]
pub struct TransferExporter {
    connector: Arc<dyn Connector>,
}

impl TransferExporter {
    /// Builds the exporter over the protocols this crate implements.
    pub fn new() -> Self {
        Self {
            connector: Arc::new(RealConnector),
        }
    }

    /// Builds the exporter over a far side somebody else provides.
    pub fn with_connector(connector: Arc<dyn Connector>) -> Self {
        Self { connector }
    }
}

impl Default for TransferExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TransferExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferExporter").finish_non_exhaustive()
    }
}

/// The connector that speaks the protocols, chosen by scheme.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealConnector;

#[async_trait]
impl Connector for RealConnector {
    async fn open(
        &self,
        target: &Target,
        credentials: &Credentials,
        host_key: Option<&HostKey>,
    ) -> Result<Box<dyn Transport>, TransportError> {
        match target.scheme {
            Scheme::Sftp => sftp::open(target, credentials, host_key).await,
            Scheme::Ftps | Scheme::Ftp => ftp::open(target, credentials).await,
            Scheme::File => local::open(target).await,
        }
    }
}

#[async_trait]
impl Exporter for TransferExporter {
    fn slug(&self) -> &str {
        SLUG
    }

    fn accepts(&self, action: &str) -> bool {
        action == ACTION_PUT
    }

    async fn dispatch(&self, ctx: DispatchContext<'_>) -> Result<Handle, ExporterError> {
        if !self.accepts(ctx.action) {
            return Err(ExporterError::UnsupportedAction {
                exporter_slug: SLUG.into(),
                action: ctx.action.into(),
            });
        }
        // Nothing is resolved yet, so there is nothing for a scrub to
        // look for: every refusal down to `resolve_credentials` is about
        // text the profile itself supplied. The scrub becomes real the
        // moment a credential exists, and from there every exit of this
        // function goes through it.
        let bare = Redaction::none();
        let params: TransferDispatchParams = match serde_json::from_value(ctx.params.clone()) {
            Ok(params) => params,
            Err(e) => {
                return Err(refuse(
                    &ctx,
                    &bare,
                    None,
                    TransportError::Refused(format!("invalid transfer params: {e}")),
                ));
            }
        };
        let described = Some(&params);

        let target =
            read_endpoint(&params.endpoint).map_err(|e| refuse(&ctx, &bare, described, e))?;
        let host_key = host_key_of(&params).map_err(|e| refuse(&ctx, &bare, described, e))?;
        check_scheme(&target, &params, host_key.as_ref())
            .map_err(|e| refuse(&ctx, &bare, described, e))?;
        let credentials = resolve_credentials(params.auth.as_ref())
            .map_err(|e| refuse(&ctx, &bare, described, e))?;
        let scrub = Redaction::of(credentials.secrets());

        // Every name and every cell is settled before the connection
        // opens. A template that does not resolve is a profile mistake,
        // and finding it out with a session open would leave a
        // half-filled directory behind on somebody's host.
        let plan = plan_send(&ctx, &params).map_err(|e| refuse(&ctx, &scrub, described, e))?;

        let mut wire = match self
            .connector
            .open(&target, &credentials, host_key.as_ref())
            .await
        {
            Ok(wire) => wire,
            Err(err) => return Err(refuse(&ctx, &scrub, described, err)),
        };
        if let Err(err) = wire.ensure_dir().await {
            return Err(refuse(&ctx, &scrub, described, err));
        }

        let mut sent = Vec::with_capacity(plan.len());
        let mut first: Option<String> = None;
        for step in &plan {
            let outcome = put_one(wire.as_mut(), step).await;
            if first.is_none()
                && let Err(why) = &outcome
            {
                first = Some(why.clone());
            }
            sent.push(recorded_file(step, &outcome));
        }
        let sidecar = sidecar_bytes(&params.sidecar, &plan);
        let sidecar_outcome = wire
            .put(&params.sidecar.filename, sidecar.as_bytes())
            .await
            .map_err(|err| err.to_string());
        if first.is_none()
            && let Err(why) = &sidecar_outcome
        {
            first = Some(why.clone());
        }
        let _ = wire.close().await;

        ctx.attempt.record(AttemptRecord::new(
            SLUG,
            scrub.json(serde_json::json!({
                "endpoint": params.endpoint,
                "scheme": target.scheme.as_str(),
                "directory": target.dir,
                "account": account_note(params.auth.as_ref()),
                "files": sent,
                "sidecar": {
                    "name": params.sidecar.filename,
                    "rows": plan.len(),
                    "outcome": outcome_word(&sidecar_outcome),
                    "answer": sidecar_outcome.as_ref().err(),
                },
            })),
        ));

        if let Some(why) = first {
            return Err(scrub.error(ExporterError::BackendRejected(why)));
        }
        let payload = TransferHandlePayload {
            directory: target.dir,
            sent: plan.len(),
            sidecar: params.sidecar.filename,
        };
        Ok(Handle::new(SLUG, serde_json::to_value(payload).unwrap()))
    }

    async fn poll(
        &self,
        _ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<DispatchState, ExporterError> {
        // The transfer already ran inside `dispatch`; nothing to wait
        // for. Guard the kind slug so a misrouted handle fails fast
        // instead of silently succeeding.
        check_kind(handle)?;
        Ok(DispatchState::Done)
    }

    async fn harvest(
        &self,
        _ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<Vec<Derived>, ExporterError> {
        check_kind(handle)?;
        // Nothing was made — see the crate docs. The runner takes an
        // empty harvest to `reify`, which parks the row in `Done` with
        // no output assets.
        Ok(Vec::new())
    }
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

/// One file, with everything about it settled: what to send, under what
/// name, and the cells its sidecar row will carry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Planned {
    /// Where the copy is on this machine.
    path: String,
    /// What it lands under on the far side.
    remote_name: String,
    /// This file's sidecar row, one cell per declared column.
    cells: Vec<String>,
}

/// Everything the send will do, worked out before the connection opens.
fn plan_send(
    ctx: &DispatchContext<'_>,
    params: &TransferDispatchParams,
) -> Result<Vec<Planned>, TransportError> {
    if params.release.files.is_empty() {
        return Err(TransportError::Refused(format!(
            "a transfer profile carries the release's file list under {RESERVED_KEY:?}, \
             and this one lists no file"
        )));
    }
    check_remote_name(&params.sidecar.filename)?;
    let env = TemplateEnv::pre_handle(ctx, ctx.params);
    let mut plan = Vec::with_capacity(params.release.files.len());
    for (index, row) in params.release.files.iter().enumerate() {
        // A row has to name one of this dispatch's own inputs. That is
        // what ties the bytes to the snapshot the run is over: without
        // it any readable path on this machine, written into a file list
        // by hand, would be put on somebody's host by an adapter that
        // had no way to know it was not a release's copy.
        let card = ctx
            .inputs
            .iter()
            .find(|card| card.id == row.asset_id)
            .ok_or_else(|| {
                TransportError::Refused(format!(
                    "the file list names asset {:?}, which is not one of this \
                     dispatch's inputs; a send's bytes are the copies made from \
                     the snapshot it runs over",
                    row.asset_id
                ))
            })?;
        let remote_name = match &params.remote_name_template {
            None => row.name.clone(),
            Some(template) => {
                let item = row.item(index, &row.name, card);
                CommonExportAdapter
                    .render(template, &env.with_item(&item))
                    .map_err(|e| TransportError::Refused(e.to_string()))?
            }
        };
        check_remote_name(&remote_name)?;
        let item = row.item(index, &remote_name, card);
        let item_env = env.with_item(&item);
        let mut cells = Vec::with_capacity(params.sidecar.columns.len());
        for column in &params.sidecar.columns {
            cells.push(
                CommonExportAdapter
                    .render(&column.template, &item_env)
                    .map_err(|e| TransportError::Refused(e.to_string()))?,
            );
        }
        plan.push(Planned {
            path: row.path.clone(),
            remote_name,
            cells,
        });
    }
    Ok(plan)
}

/// What a file may be called on the far side: one path segment.
///
/// Stated here rather than in each [`Transport`], because the question
/// is about the name and not about the protocol carrying it. A
/// `remote_name_template` that rendered a separator would land the bytes
/// outside the directory the endpoint named — a different destination
/// from the one the send recorded — and a name that rendered empty is
/// not a file anywhere.
fn check_remote_name(name: &str) -> Result<(), TransportError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(TransportError::Refused(format!(
            "a file's name on the far side is one path segment, and this is \
             not: {name:?}"
        )));
    }
    Ok(())
}

/// Sends one file, answering with what the far side said if it refused.
async fn put_one(wire: &mut dyn Transport, step: &Planned) -> Result<usize, String> {
    let bytes = std::fs::read(&step.path)
        .map_err(|e| format!("read the copy to send at {}: {e}", step.path))?;
    wire.put(&step.remote_name, &bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(bytes.len())
}

/// One file as the attempt record holds it.
fn recorded_file(step: &Planned, outcome: &Result<usize, String>) -> Value {
    serde_json::json!({
        "name": step.remote_name,
        "source": step.path,
        "bytes": outcome.as_ref().ok(),
        "outcome": outcome_word(outcome),
        "answer": outcome.as_ref().err(),
    })
}

/// What the record calls an outcome.
fn outcome_word<T>(outcome: &Result<T, String>) -> &'static str {
    match outcome {
        Ok(_) => "sent",
        Err(_) => "failed",
    }
}

/// The account, as the record may hold it: the name, and the *names* of
/// the variables its credential came from.
fn account_note(auth: Option<&AuthSchema>) -> Value {
    match auth {
        None => Value::Null,
        Some(auth) => serde_json::json!({
            "user": auth.user,
            "secret_ref": auth.secret_ref,
            "key_ref": auth.key_ref,
            "key_passphrase_ref": auth.key_passphrase_ref,
        }),
    }
}

/// Records a refusal and turns it into what `dispatch` returns.
///
/// Every arm between reading the params and the first successful put
/// comes through here, which is what makes the crate doc's promises hold
/// as one mechanism rather than as a rule each arm has to remember. Not
/// the two outside that span: an unsupported action is answered before
/// there is anything to describe and is the SDK's own variant, which
/// routing through here would rewrite as a rejection by the backend; a
/// put that failed has a row of its own among the per-file ones.
///
/// The record is written because this is the arm a reader has the most
/// questions about: no handle is produced, so without it the endpoint,
/// the account and the reason would leave with the error and the row
/// would keep one message. The scrub is applied to both halves — see
/// [`Redaction`] for what a call's own text can carry back.
///
/// `params` is absent only when the blob did not deserialise, which is
/// the one refusal that happens before there is a profile to describe.
fn refuse(
    ctx: &DispatchContext<'_>,
    scrub: &Redaction,
    params: Option<&TransferDispatchParams>,
    why: TransportError,
) -> ExporterError {
    let message = why.to_string();
    ctx.attempt.record(AttemptRecord::new(
        SLUG,
        scrub.json(serde_json::json!({
            "endpoint": params.map(|params| params.endpoint.as_str()),
            "account": params
                .map(|params| account_note(params.auth.as_ref()))
                .unwrap_or(Value::Null),
            "refused": message,
            "files": [],
        })),
    ));
    scrub.error(ExporterError::BackendRejected(message))
}

/// What the profile said the host's key would be.
fn host_key_of(params: &TransferDispatchParams) -> Result<Option<HostKey>, TransportError> {
    let Some(schema) = params.host_key.as_ref() else {
        return Ok(None);
    };
    match (&schema.fingerprint, &schema.known_hosts) {
        (Some(fingerprint), None) => Ok(Some(HostKey::Fingerprint(fingerprint.clone()))),
        (None, Some(path)) => Ok(Some(HostKey::KnownHosts(path.into()))),
        _ => Err(TransportError::Refused(
            "host_key names the fingerprint the host offers or a known_hosts file to \
             look it up in — one of the two, and this names neither or both"
                .into(),
        )),
    }
}

/// The two refusals that happen before a socket opens.
fn check_scheme(
    target: &Target,
    params: &TransferDispatchParams,
    host_key: Option<&HostKey>,
) -> Result<(), TransportError> {
    if target.scheme == Scheme::Ftp && !params.allow_insecure {
        return Err(TransportError::Refused(
            "ftp:// sends the credential and the files in the clear; a profile that \
             means it says allow_insecure: true"
                .into(),
        ));
    }
    if target.scheme == Scheme::Sftp && host_key.is_none() {
        return Err(TransportError::Refused(
            "an sftp:// profile says which host key it expects, as host_key.fingerprint \
             or host_key.known_hosts; there is nobody in front of a dispatch to ask"
                .into(),
        ));
    }
    Ok(())
}

/// Reads the credential out of the environment the profile named it in.
///
/// A variable the profile names and the environment does not have is a
/// refusal rather than an empty credential: an anonymous login shaped
/// like an authenticated one is a worse place to learn about it than the
/// dispatch that will not start.
fn resolve_credentials(auth: Option<&AuthSchema>) -> Result<Credentials, TransportError> {
    let Some(auth) = auth else {
        return Ok(Credentials::default());
    };
    Ok(Credentials {
        user: Some(auth.user.clone()),
        password: from_env(auth.secret_ref.as_deref(), "auth.secret_ref")?,
        key_path: from_env(auth.key_ref.as_deref(), "auth.key_ref")?.map(Into::into),
        key_passphrase: from_env(
            auth.key_passphrase_ref.as_deref(),
            "auth.key_passphrase_ref",
        )?,
    })
}

/// One environment variable, by the name a profile field gave.
///
/// A `Refused`, so it joins every other answer given before a byte moved
/// and reaches the attempt record the way they do.
fn from_env(name: Option<&str>, field: &str) -> Result<Option<String>, TransportError> {
    let Some(name) = name else {
        return Ok(None);
    };
    std::env::var(name).map(Some).map_err(|_| {
        TransportError::Refused(format!(
            "{field} names environment variable {name:?}, which is not set"
        ))
    })
}

/// The sidecar, as RFC 4180 spells a CSV.
///
/// Written here rather than taken from a crate. The rule is four
/// sentences long — a cell carrying a comma, a quote, a carriage return
/// or a newline is wrapped in quotes, and a quote inside one is doubled
/// — and it is the whole of what this adapter needs from CSV: nothing
/// here reads one back, so there is no parser whose acceptance would
/// have to agree with a writer's.
fn sidecar_bytes(schema: &SidecarSchema, plan: &[Planned]) -> String {
    let mut out = String::new();
    push_row(&mut out, schema.columns.iter().map(|c| c.header.as_str()));
    for step in plan {
        push_row(&mut out, step.cells.iter().map(String::as_str));
    }
    out
}

/// One CSV record, terminated the way RFC 4180 terminates one.
fn push_row<'a>(out: &mut String, cells: impl Iterator<Item = &'a str>) {
    let mut first = true;
    for cell in cells {
        if !first {
            out.push(',');
        }
        first = false;
        if cell.contains([',', '"', '\r', '\n']) {
            out.push('"');
            out.push_str(&cell.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(cell);
        }
    }
    out.push_str("\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Fake, FakeConnector};
    use asterism_dispatch_sdk::{AttemptRecord, AttemptRecorder};
    use std::sync::Mutex;

    /// Keeps whatever the exporter recorded, so a test can read it the
    /// way the runner's own recorder lets the row read it.
    #[derive(Default)]
    struct Recorded(Mutex<Option<AttemptRecord>>);

    impl AttemptRecorder for Recorded {
        fn record(&self, record: AttemptRecord) {
            *self.0.lock().unwrap() = Some(record);
        }
    }

    impl Recorded {
        fn payload(&self) -> Value {
            self.0
                .lock()
                .unwrap()
                .clone()
                .expect("the exporter recorded the call")
                .payload
        }
    }

    fn card(id: &str, title: Option<&str>) -> AssetCardDto {
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
            source_locator: "/library/original.png".into(),
            group_ids: vec![],
            primary_group_position: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            rating: None,
            palette: None,
            has_note: false,
            has_thread: false,
            role: "item".into(),
            title: title.map(Into::into),
            member_count: 0,
            score: None,
            snippet: None,
            found_by: None,
            author_kind: None,
            author_subject: None,
            operator_ai: None,
        }
    }

    /// The inputs a send over `copies(dir, names)` runs with: one card
    /// per row, under the id that row names. A row naming no input is
    /// refused, so a test that reaches the plan supplies these.
    fn inputs_for(names: &[&str]) -> Vec<AssetCardDto> {
        (0..names.len())
            .map(|nth| card(&format!("asset-{nth}"), None))
            .collect()
    }

    /// A directory of stamped copies, and the file list a send would
    /// have written for them.
    fn copies(dir: &std::path::Path, names: &[&str]) -> Vec<Value> {
        names
            .iter()
            .enumerate()
            .map(|(nth, name)| {
                let path = dir.join(name);
                std::fs::write(&path, format!("bytes-{nth}")).expect("write a copy");
                serde_json::json!({
                    "asset_id": format!("asset-{nth}"),
                    "path": path.display().to_string(),
                    "name": name,
                })
            })
            .collect()
    }

    fn profile(endpoint: &str, files: Vec<Value>, extra: Value) -> Value {
        let mut params = serde_json::json!({
            "endpoint": endpoint,
            "sidecar": {
                "filename": "metadata.csv",
                "columns": [
                    { "header": "Filename", "template": "{{item.remote_name}}" },
                    { "header": "Title", "template": "{{item.card.title?}}" }
                ]
            },
            RESERVED_KEY: { "files": files },
        });
        for (key, value) in extra.as_object().expect("an object").clone() {
            params[key] = value;
        }
        params
    }

    async fn run(
        exporter: &TransferExporter,
        params: &Value,
        inputs: &[AssetCardDto],
        recorded: &Recorded,
    ) -> Result<Handle, ExporterError> {
        let ctx = DispatchContext {
            inputs,
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            persona_id: "p1",
            action: ACTION_PUT,
            params,
            attempt: recorded,
        };
        exporter.dispatch(ctx).await
    }

    #[test]
    fn slug_and_accepts_are_stable() {
        let exporter = TransferExporter::new();
        assert_eq!(exporter.slug(), SLUG);
        assert!(exporter.accepts(ACTION_PUT));
        assert!(!exporter.accepts("write"));
    }

    /// The order of the puts is the file list's order, each file lands
    /// under the copy's own name, and the sidecar goes beside them.
    #[tokio::test]
    async fn every_file_is_put_in_order_with_the_sidecar_beside_them() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png", "two.png"]);
        let params = profile(
            "sftp://host/incoming",
            files,
            serde_json::json!({ "host_key": { "fingerprint": "SHA256:known" } }),
        );
        let far = Fake::shared();
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(far.clone()));

        let handle = run(
            &exporter,
            &params,
            &[card("asset-0", Some("first")), card("asset-1", None)],
            &recorded,
        )
        .await
        .expect("the send");

        assert_eq!(handle.kind, SLUG);
        assert!(*far.directory_made.lock().unwrap(), "the directory first");
        assert_eq!(
            far.order(),
            vec![
                "one.png".to_string(),
                "two.png".to_string(),
                "metadata.csv".to_string()
            ]
        );
        assert_eq!(far.file("one.png").unwrap(), b"bytes-0");
        assert_eq!(far.file("two.png").unwrap(), b"bytes-1");
    }

    /// One row per file in send order, under the headers the profile
    /// declared, rendered from the input cards.
    #[tokio::test]
    async fn the_sidecar_has_a_row_per_file_in_send_order() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png", "two.png"]);
        let params = profile(
            "file:///unused",
            files,
            serde_json::json!({
                "sidecar": {
                    "filename": "metadata.csv",
                    "columns": [
                        { "header": "Filename", "template": "{{item.remote_name}}" },
                        { "header": "Title", "template": "{{item.card.title?}}" },
                        { "header": "Batch", "template": "{{dispatch_id}}" }
                    ]
                }
            }),
        );
        let far = Fake::shared();
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(far.clone()));

        run(
            &exporter,
            &params,
            &[
                card("asset-0", Some("a quiet, sunlit \"room\"")),
                card("asset-1", None),
            ],
            &recorded,
        )
        .await
        .expect("the send");

        let csv = String::from_utf8(far.file("metadata.csv").expect("the sidecar")).unwrap();
        assert_eq!(
            csv,
            "Filename,Title,Batch\r\n\
             one.png,\"a quiet, sunlit \"\"room\"\"\",disp-1\r\n\
             two.png,,disp-1\r\n",
            "a comma and a quote are what RFC 4180 quotes and doubles; a card with \
             no title contributes an empty cell rather than a missing one"
        );
    }

    /// The copy's basename is the default, and a profile that wants
    /// another name says so once.
    #[tokio::test]
    async fn a_profile_may_rename_what_lands_on_the_far_side() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        let params = profile(
            "file:///unused",
            files,
            serde_json::json!({ "remote_name_template": "{{persona_id}}-{{item.index}}.png" }),
        );
        let far = Fake::shared();
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(far.clone()));

        run(&exporter, &params, &[card("asset-0", None)], &recorded)
            .await
            .expect("the send");

        assert_eq!(
            far.order(),
            vec!["p1-0.png".to_string(), "metadata.csv".to_string()]
        );
    }

    /// The default answer to a credential over cleartext is no, and the
    /// refusal happens before a connection is opened.
    #[tokio::test]
    async fn plain_ftp_without_the_opt_in_is_refused_before_any_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        let params = profile("ftp://host/incoming", files, serde_json::json!({}));
        let far = Fake::shared();
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(far.clone()));

        let refused = run(&exporter, &params, &[], &recorded)
            .await
            .expect_err("cleartext is opt-in");

        assert!(refused.to_string().contains("allow_insecure"), "{refused}");
        assert!(far.order().is_empty(), "nothing was put");
        assert!(!*far.directory_made.lock().unwrap(), "nothing was opened");
        let payload = recorded.payload();
        assert!(
            payload["refused"]
                .as_str()
                .expect("the refusal is on the record")
                .contains("allow_insecure")
        );
    }

    /// And a profile that means it says so.
    #[tokio::test]
    async fn plain_ftp_with_the_opt_in_goes_out() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        let params = profile(
            "ftp://host/incoming",
            files,
            serde_json::json!({ "allow_insecure": true }),
        );
        let far = Fake::shared();
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(far.clone()));

        run(&exporter, &params, &inputs_for(&["one.png"]), &recorded)
            .await
            .expect("the send");

        assert_eq!(far.order().len(), 2);
    }

    /// A host offering a key the profile does not name is refused with
    /// nothing put — see the crate docs for why there is no prompt.
    #[tokio::test]
    async fn a_host_key_that_does_not_match_is_refused_before_any_put() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        let params = profile(
            "sftp://host/incoming",
            files,
            serde_json::json!({ "host_key": { "fingerprint": "SHA256:expected" } }),
        );
        let far = Fake::shared();
        let recorded = Recorded::default();
        let exporter =
            TransferExporter::with_connector(FakeConnector::offering(far.clone(), "SHA256:other"));

        let refused = run(&exporter, &params, &inputs_for(&["one.png"]), &recorded)
            .await
            .expect_err("the host is not the one named");

        assert!(refused.to_string().contains("SHA256:other"), "{refused}");
        assert!(far.order().is_empty(), "nothing was put");
        assert!(
            recorded.payload()["refused"]
                .as_str()
                .expect("the refusal is on the record")
                .contains("SHA256:expected")
        );
    }

    /// An `sftp://` profile that says nothing about the host key is the
    /// same refusal as one that names the wrong key.
    #[tokio::test]
    async fn an_sftp_profile_without_a_host_key_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        let params = profile("sftp://host/incoming", files, serde_json::json!({}));
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(Fake::shared()));

        let refused = run(&exporter, &params, &[], &recorded)
            .await
            .expect_err("an unverified host key is handed to whoever answered");

        assert!(refused.to_string().contains("host_key"), "{refused}");
    }

    /// A put that failed part way leaves a record of every file, and
    /// the run fails with the first thing that went wrong.
    #[tokio::test]
    async fn a_partial_put_records_every_file_and_fails_with_the_first_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png", "two.png", "three.png"]);
        let params = profile("file:///unused", files, serde_json::json!({}));
        let far = Fake::shared();
        far.refusing("two.png", "550 quota exceeded");
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(far.clone()));

        let failed = run(
            &exporter,
            &params,
            &inputs_for(&["one.png", "two.png", "three.png"]),
            &recorded,
        )
        .await
        .expect_err("the far side said no to one of them");

        assert!(
            failed.to_string().contains("550 quota exceeded"),
            "{failed}"
        );
        assert_eq!(
            far.order(),
            vec![
                "one.png".to_string(),
                "three.png".to_string(),
                "metadata.csv".to_string()
            ],
            "the pass continues, so what did land is what the record says"
        );
        let payload = recorded.payload();
        let recorded_files = payload["files"].as_array().expect("a row per file");
        assert_eq!(recorded_files.len(), 3);
        assert_eq!(recorded_files[0]["outcome"], "sent");
        assert_eq!(recorded_files[0]["bytes"], 7);
        assert_eq!(recorded_files[1]["outcome"], "failed");
        assert_eq!(recorded_files[1]["answer"], "550 quota exceeded");
        assert_eq!(recorded_files[2]["outcome"], "sent");
    }

    /// The record names the variables and never their values, and the
    /// scrub is what stands between an answer that echoes a credential
    /// and the row that answer lands on.
    #[tokio::test]
    async fn nothing_recorded_carries_the_credential() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        // SAFETY: the variable is this test's own name, and nothing
        // else in this binary reads or writes it.
        unsafe { std::env::set_var("ASTERISM_TEST_TRANSFER_PASSWORD", "hunter2") };
        let params = profile(
            "file:///unused",
            files,
            serde_json::json!({
                "auth": {
                    "user": "contributor",
                    "secret_ref": "ASTERISM_TEST_TRANSFER_PASSWORD"
                }
            }),
        );
        let far = Fake::shared();
        far.refusing("one.png", "530 login incorrect for hunter2");
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(far.clone()));

        let failed = run(&exporter, &params, &inputs_for(&["one.png"]), &recorded)
            .await
            .expect_err("the far side said no");

        let payload = recorded.payload().to_string();
        assert!(!payload.contains("hunter2"), "{payload}");
        assert!(
            payload.contains(asterism_exporter_common::REDACTED),
            "{payload}"
        );
        assert!(
            payload.contains("ASTERISM_TEST_TRANSFER_PASSWORD"),
            "the name stays, so a reader can tell which profile was in play: {payload}"
        );
        assert!(!failed.to_string().contains("hunter2"), "{failed}");
    }

    /// The arm nothing here composed: a server that will not take the
    /// login answers in its own words and commonly quotes what it was
    /// sent. Neither the password nor the key's location may reach the
    /// row through it, and that is the scrub's job rather than the
    /// server's.
    #[tokio::test]
    async fn a_refused_login_leaves_neither_the_password_nor_the_key_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        let key = tmp.path().join("agency.ed25519");
        let key_path = key.display().to_string();
        // SAFETY: both variables are this test's own names, and nothing
        // else in this binary reads or writes them.
        unsafe {
            std::env::set_var("ASTERISM_TEST_TRANSFER_REFUSED_PASSWORD", "hunter2");
            std::env::set_var("ASTERISM_TEST_TRANSFER_REFUSED_KEY", &key_path);
        }
        let params = profile(
            "file:///unused",
            files,
            serde_json::json!({
                "auth": {
                    "user": "contributor",
                    "secret_ref": "ASTERISM_TEST_TRANSFER_REFUSED_PASSWORD",
                    "key_ref": "ASTERISM_TEST_TRANSFER_REFUSED_KEY"
                }
            }),
        );
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::refusing_to_open(
            Fake::shared(),
            &format!("530 login incorrect for hunter2 using key {key_path}"),
        ));

        let refused = run(&exporter, &params, &inputs_for(&["one.png"]), &recorded)
            .await
            .expect_err("the far side would not take the login");

        let payload = recorded.payload().to_string();
        assert!(!payload.contains("hunter2"), "{payload}");
        assert!(!payload.contains(&key_path), "{payload}");
        assert!(
            payload.contains(asterism_exporter_common::REDACTED),
            "{payload}"
        );
        assert!(
            payload.contains("ASTERISM_TEST_TRANSFER_REFUSED_KEY"),
            "the names stay, so a reader can tell which profile was in \
             play: {payload}"
        );
        assert!(
            payload.contains("contributor"),
            "the account is on the record on purpose: {payload}"
        );
        let message = refused.to_string();
        assert!(!message.contains("hunter2"), "{message}");
        assert!(!message.contains(&key_path), "{message}");
    }

    /// A variable the profile names and the environment does not have
    /// is a refusal, not an anonymous login — and the row says so rather
    /// than keeping the message alone.
    #[tokio::test]
    async fn a_credential_the_environment_does_not_have_stops_the_dispatch() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        let params = profile(
            "file:///unused",
            files,
            serde_json::json!({
                "auth": {
                    "user": "contributor",
                    "secret_ref": "ASTERISM_TEST_NO_SUCH_VARIABLE"
                }
            }),
        );
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(Fake::shared()));

        let refused = run(&exporter, &params, &[], &recorded)
            .await
            .expect_err("the credential is not there");

        assert!(refused.to_string().contains("secret_ref"), "{refused}");
        assert!(
            recorded.payload()["refused"]
                .as_str()
                .expect("the refusal is on the record")
                .contains("ASTERISM_TEST_NO_SUCH_VARIABLE")
        );
    }

    /// A file list is only allowed to name this dispatch's own inputs.
    /// Without that, a hand-written list reaching this adapter through
    /// the generic dispatch route would put any readable local path on
    /// somebody's host.
    #[tokio::test]
    async fn a_row_naming_no_input_of_this_dispatch_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let secret = tmp.path().join("not-a-release-copy.png");
        std::fs::write(&secret, b"bytes").expect("a file nothing released");
        let params = profile(
            "file:///unused",
            vec![serde_json::json!({
                "asset_id": "an-asset-this-dispatch-does-not-have",
                "path": secret.display().to_string(),
                "name": "not-a-release-copy.png",
            })],
            serde_json::json!({}),
        );
        let far = Fake::shared();
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(far.clone()));

        let refused = run(&exporter, &params, &inputs_for(&["one.png"]), &recorded)
            .await
            .expect_err("the row names nothing this dispatch is over");

        assert!(refused.to_string().contains("inputs"), "{refused}");
        assert!(far.order().is_empty(), "nothing was put");
        assert!(!*far.directory_made.lock().unwrap(), "nothing was opened");
    }

    /// A name that walked out of the directory would land the bytes
    /// somewhere the send did not record, and the refusal is above the
    /// transport so every protocol gets it.
    #[tokio::test]
    async fn a_remote_name_that_is_not_one_segment_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        let params = profile(
            "file:///unused",
            files,
            serde_json::json!({ "remote_name_template": "../escaped-{{item.index}}.png" }),
        );
        let far = Fake::shared();
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(far.clone()));

        let refused = run(&exporter, &params, &inputs_for(&["one.png"]), &recorded)
            .await
            .expect_err("a remote name is one path segment");

        assert!(
            refused.to_string().contains("one path segment"),
            "{refused}"
        );
        assert!(far.order().is_empty(), "nothing was put");
        assert!(!*far.directory_made.lock().unwrap(), "nothing was opened");
    }

    /// The same rule answers for the sidecar, which is put beside the
    /// files by a name the profile gave.
    #[tokio::test]
    async fn a_sidecar_filename_that_is_not_one_segment_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        let params = profile(
            "file:///unused",
            files,
            serde_json::json!({
                "sidecar": {
                    "filename": "../metadata.csv",
                    "columns": [{ "header": "Filename", "template": "{{item.remote_name}}" }]
                }
            }),
        );
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(Fake::shared()));

        let refused = run(&exporter, &params, &inputs_for(&["one.png"]), &recorded)
            .await
            .expect_err("a sidecar name is one path segment");

        assert!(
            refused.to_string().contains("one path segment"),
            "{refused}"
        );
    }

    /// Every answer this adapter gives without a handle reaches the row,
    /// including the two that happen before there is a profile or a
    /// credential to describe.
    #[tokio::test]
    async fn a_params_blob_that_does_not_parse_is_recorded() {
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(Fake::shared()));

        let refused = run(
            &exporter,
            &serde_json::json!({ "endpoint": "file:///unused" }),
            &[],
            &recorded,
        )
        .await
        .expect_err("a profile without a sidecar or a file list is not one");

        assert!(refused.to_string().contains("invalid transfer params"));
        let payload = recorded.payload();
        assert!(
            payload["refused"]
                .as_str()
                .expect("the refusal is on the record")
                .contains("invalid transfer params")
        );
        assert!(
            payload["endpoint"].is_null(),
            "nothing was parsed, so nothing is described: {payload}"
        );
    }

    /// A profile with no file list has nothing this adapter would send,
    /// and falling back to the inputs would send the library's own
    /// unstamped originals.
    #[tokio::test]
    async fn a_profile_that_lists_no_file_is_refused() {
        let params = profile("file:///unused", vec![], serde_json::json!({}));
        let far = Fake::shared();
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(far.clone()));

        let refused = run(&exporter, &params, &[card("asset-0", None)], &recorded)
            .await
            .expect_err("there is nothing to send");

        assert!(refused.to_string().contains(RESERVED_KEY), "{refused}");
        assert!(far.order().is_empty());
    }

    /// A template that names something the card does not have is a
    /// profile mistake, and it is found before a connection is opened
    /// rather than after half the set has landed.
    #[tokio::test]
    async fn a_column_that_does_not_resolve_stops_the_send_before_it_starts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        let params = profile(
            "file:///unused",
            files,
            serde_json::json!({
                "sidecar": {
                    "filename": "metadata.csv",
                    "columns": [{ "header": "Nope", "template": "{{item.card.nope}}" }]
                }
            }),
        );
        let far = Fake::shared();
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(far.clone()));

        let refused = run(&exporter, &params, &[card("asset-0", None)], &recorded)
            .await
            .expect_err("the column names nothing");

        assert!(refused.to_string().contains("did not resolve"), "{refused}");
        assert!(!*far.directory_made.lock().unwrap(), "nothing was opened");
    }

    /// Nothing was made, so nothing is minted: the runner takes the
    /// empty harvest to `reify` and the row parks in `Done` with no
    /// output assets.
    #[tokio::test]
    async fn the_run_is_done_at_once_and_harvests_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = copies(tmp.path(), &["one.png"]);
        let params = profile("file:///unused", files, serde_json::json!({}));
        let recorded = Recorded::default();
        let exporter = TransferExporter::with_connector(FakeConnector::accepting(Fake::shared()));
        let handle = run(&exporter, &params, &inputs_for(&["one.png"]), &recorded)
            .await
            .expect("send");

        let ctx = DispatchContext {
            inputs: &[],
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            persona_id: "p1",
            action: ACTION_PUT,
            params: &params,
            attempt: &recorded,
        };

        assert_eq!(
            exporter.poll(ctx, &handle).await.unwrap(),
            DispatchState::Done
        );
        assert!(exporter.harvest(ctx, &handle).await.unwrap().is_empty());
    }

    /// A handle from another adapter is a routing mistake, and it fails
    /// fast rather than reporting a send that did not happen.
    #[tokio::test]
    async fn a_handle_from_another_adapter_is_refused() {
        let params = profile("file:///unused", vec![], serde_json::json!({}));
        let recorded = Recorded::default();
        let exporter = TransferExporter::new();
        let ctx = DispatchContext {
            inputs: &[],
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            persona_id: "p1",
            action: ACTION_PUT,
            params: &params,
            attempt: &recorded,
        };
        let stranger = Handle::new("file", serde_json::json!({}));

        assert!(matches!(
            exporter.poll(ctx, &stranger).await,
            Err(ExporterError::HandleMismatch { .. })
        ));
    }

    /// The shipped example is what `asterism-server schema print
    /// exporter:transfer:params` hands a caller, so it has to survive
    /// the same `serde_json::from_value` `dispatch` runs on `ctx.params`.
    #[test]
    fn the_params_example_deserialises_into_the_current_struct() {
        let params: TransferDispatchParams = serde_json::from_str(params_example_json())
            .expect("schema/transfer_params.example.json parses as TransferDispatchParams");

        assert!(params.endpoint.starts_with("sftp://"));
        assert!(
            params.host_key.is_some(),
            "the example advertises sftp, which is refused without one"
        );
        assert!(!params.sidecar.columns.is_empty());
        assert!(
            !params.release.files.is_empty(),
            "the file list is what a send writes, and an example without one \
             documents a dispatch that would be refused"
        );
    }

    /// The example's own templates have to resolve against the shape
    /// they are written for, or the runnable example is not runnable.
    #[test]
    fn the_params_example_templates_resolve() {
        let params: Value = serde_json::from_str(params_example_json()).expect("json");
        let typed: TransferDispatchParams = serde_json::from_value(params.clone()).expect("params");
        let inputs: Vec<AssetCardDto> = typed
            .release
            .files
            .iter()
            .map(|row| card(&row.asset_id, Some("a title")))
            .collect();
        let ctx = DispatchContext {
            inputs: &inputs,
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            persona_id: "p1",
            action: ACTION_PUT,
            params: &params,
            attempt: &asterism_dispatch_sdk::DISCARD_ATTEMPTS,
        };

        let plan = plan_send(&ctx, &typed).expect("every template resolves");

        assert_eq!(plan.len(), typed.release.files.len());
        assert!(
            plan.iter()
                .all(|step| step.cells.len() == typed.sidecar.columns.len())
        );
    }
}
