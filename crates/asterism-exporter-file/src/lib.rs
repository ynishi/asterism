//! # asterism-exporter-file
//!
//! Generic filesystem exporter. Takes a Selection of already-ingested
//! Assets, copies (or symlinks, or just references) each input into a
//! caller-supplied output directory, and emits one [`Derived`] per
//! written entry.
//!
//! Position in the workspace:
//!
//! - Mirrors the `asterism-importer-image` / `-video` / `-audio`
//!   position on the OUT side — a concrete, name-shaped exporter
//!   registered next to `comfy` in the server's [`ExporterRegistry`].
//! - Unlike the importers (subprocess binaries that POST to the
//!   server) this crate is a **library** because exporters run
//!   in-process inside the apalis `DispatchRun` worker.
//!
//! ## When to use
//!
//! - Physically duplicate every asset in a Selection into a
//!   pre-configured drop folder — e.g. the Comfy `input/` mount, a
//!   Photoshop watch folder, a portable archive dir.
//! - Materialise a **read-only reference** to each Selection member
//!   as a new derived Asset (`mode = "reference"`), so a Selection
//!   can be turned into a persistent, linkable subset without moving
//!   any bytes.
//!
//! ## Params schema
//!
//! `CreateDispatchCommand.params_json` deserialises into
//! [`FileDispatchParams`]:
//!
//! ```json
//! {
//!   "output_dir": "/absolute/path",
//!   "mode": "copy" | "symlink" | "reference" | "instruction",
//!   "filename_template": "{{basename}}",
//!   "modality": "image",
//!   "labels": ["archive"],
//!   "instruction": { "workflow": "wf-1", "prompt": "..." }
//! }
//! ```
//!
//! - `output_dir` — the directory the exporter writes into. Created
//!   recursively when it does not exist.
//! - `mode` — how the input is materialised on disk:
//!   - `copy` — physical copy (owns the bytes, safe against source
//!     deletion).
//!   - `symlink` — creates a symlink pointing at the source. Cheaper
//!     than copy; breaks if the source moves.
//!   - `reference` — nothing is written; the Derived's `locator`
//!     points at the original source. Useful for "make a subset
//!     Selection into a listable dispatch history" without touching
//!     the filesystem.
//!   - `instruction` — writes a single dispatch-scoped JSON file to
//!     `output_dir` that embeds the caller-supplied `instruction`
//!     blob plus the input locator list. Intended for fs-mediated
//!     handoff to an external receiver (e.g. a Comfy watch-folder
//!     plugin) that runs the workflow and drops results into a
//!     directory an Importer is watching. Emits exactly one Derived
//!     (`modality = "instruction"` by default) pointing at the
//!     written file. `filename_template` defaults to
//!     `"{{dispatch_id}}.json"` in this mode.
//! - `filename_template` — filename shape under `output_dir`.
//!   Supports these placeholders (simple text substitution, no
//!   arithmetic):
//!   - `{{basename}}` — original filename (base name including
//!     extension). Default when the field is omitted.
//!   - `{{stem}}` — original filename without extension.
//!   - `{{ext}}` — original extension (without the dot).
//!   - `{{index}}` — 0-indexed position within the Selection.
//!   - `{{selection_id}}` / `{{dispatch_id}}` / `{{persona_id}}` —
//!     ids from the dispatch context.
//!   - `{{action}}` — the exporter action slug.
//!     Collisions (two input assets that would produce the same output
//!     path) are broken by appending `-<index>` to the stem before the
//!     extension.
//! - `modality` — modality slug written on each Derived. Optional; if
//!   omitted the exporter passes the input's modality through
//!   verbatim so the derived Asset lands in the same grid lane as
//!   its source.
//! - `labels` — extra labels appended to each Derived (in addition
//!   to the exporter/action labels the core prepends).
//!
//! ## Lifecycle
//!
//! Filesystem writes are synchronous; the exporter does all the work
//! inside [`dispatch`](FileExporter::dispatch) and stashes the
//! produced [`Derived`] list on the returned [`Handle`]'s payload.
//! `poll` always returns [`DispatchState::Done`] immediately;
//! `harvest` just deserialises the cached list. This keeps the
//! runner's state machine identical to the network-backed exporters
//! without introducing a fake "waiting" phase.

use std::path::{Path, PathBuf};

use asterism_contract::dto::AssetCardDto;
use asterism_contract::sidecar::{SIDECAR_IDENTITY_KEY, SIDECAR_SCHEMA};
use asterism_dispatch_sdk::{
    Derived, DispatchContext, DispatchState, Exporter, ExporterError, Handle,
};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Slug the registry uses for this exporter.
pub const SLUG: &str = "file";
/// The single action name — every params variant flows through the
/// same `write` action; the actual write behaviour is controlled by
/// [`FileDispatchParams::mode`].
pub const ACTION_WRITE: &str = "write";

/// Public name for this exporter's params schema in the
/// `asterism-server schema` CLI (`exporter:file:params`).
pub const SCHEMA_NAME: &str = "exporter:file:params";

/// Canonical example JSON for [`FileDispatchParams`] — the shape
/// an external caller (CC, LLM prompt, Comfy plugin author) needs
/// to construct a valid params blob. Streamed by
/// `asterism-server schema print exporter:file:params`.
pub fn params_example_json() -> &'static str {
    include_str!("../schema/file_params.example.json")
}

/// Write mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteMode {
    /// Physically copy the input bytes to `output_dir`.
    Copy,
    /// Create a symlink under `output_dir` pointing at the source.
    Symlink,
    /// Do not touch the filesystem — the Derived's `locator` points
    /// at the original source path.
    #[default]
    Reference,
    /// Write a single instruction JSON file describing the entire
    /// dispatch (params + selection input list) into `output_dir`.
    /// The written file is intended to be consumed by an external
    /// receiver (e.g. a Comfy watch-folder plugin) that runs the
    /// workflow and drops results back into a directory an Importer
    /// is watching. Emits exactly one Derived pointing at the written
    /// instruction file.
    Instruction,
}

/// Params schema for [`SLUG`] dispatch calls.
#[derive(Debug, Clone, Deserialize)]
pub struct FileDispatchParams {
    /// Absolute directory the exporter writes into.
    pub output_dir: String,
    /// Write mode. Defaults to [`WriteMode::Reference`] (no disk I/O).
    #[serde(default)]
    pub mode: WriteMode,
    /// Optional filename template under `output_dir` (see module
    /// docs). Defaults to `"{{basename}}"`.
    #[serde(default)]
    pub filename_template: Option<String>,
    /// Optional modality slug for the Derived rows. When `None`, the
    /// input asset's modality is passed through.
    #[serde(default)]
    pub modality: Option<String>,
    /// Extra labels appended to every Derived.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Instruction payload embedded into the written JSON when
    /// [`WriteMode::Instruction`] is selected. The receiver plugin
    /// gets this verbatim under the `instruction` key alongside the
    /// dispatch context and the input list. Ignored by every other
    /// mode.
    #[serde(default)]
    pub instruction: Option<serde_json::Value>,
    /// Emit a `<output_file>.meta.json` sidecar per input, holding
    /// the input's [`AssetCardDto`] JSON. Applies to copy / symlink
    /// / reference modes. Ignored by instruction mode (the
    /// dispatch envelope already carries the input list inline).
    /// Defaults to `true`: embedded metadata does not survive
    /// automation pipelines, so a dispatched file must carry its
    /// sidecar for the Re-In roundtrip to recover lineage without
    /// caller opt-in. Pass `false` to opt out.
    #[serde(default = "default_emit_metadata")]
    pub emit_metadata: bool,
    /// Emit an `_schema/asset_card.json` file at the head of
    /// `output_dir` (once per dispatch), holding the canonical
    /// [`AssetCardDto`] example JSON so the receiver can discover
    /// the sidecar shape without an out-of-band `--print-schema`
    /// call. Defaults to `false`.
    #[serde(default)]
    pub emit_schema_manifest: bool,
    /// Allowlist of top-level [`AssetCardDto`] field names to keep
    /// in the emitted metadata (sidecars + instruction-mode input
    /// list). `None` / empty vec = every field. Kept as
    /// `Vec<String>` so open-slug additions to the DTO do not need
    /// an enum change here.
    #[serde(default)]
    pub metadata_fields: Option<Vec<String>>,
}

/// Serde default for [`Params::emit_metadata`] (`bool` fields cannot
/// carry a literal default inline). ON by default: sidecar/push is the
/// lineage path that survives automation, so emitting it must not
/// require caller opt-in.
fn default_emit_metadata() -> bool {
    true
}

/// Payload persisted on the returned [`Handle`].
///
/// Filesystem work is synchronous, so all work happens in `dispatch`
/// and the results are cached here — subsequent `poll` / `harvest`
/// calls are pure reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileHandlePayload {
    /// Cached Derived list, ready to hand back to the core.
    derived: Vec<Derived>,
}

/// Filesystem-backed [`Exporter`].
///
/// Stateless — cheap to `Clone`; a single instance is registered per
/// server via [`asterism_infra::dispatch::ExporterRegistry`].
#[derive(Debug, Clone, Default)]
pub struct FileExporter;

impl FileExporter {
    /// Builds a stateless exporter.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Exporter for FileExporter {
    fn slug(&self) -> &str {
        SLUG
    }

    fn accepts(&self, action: &str) -> bool {
        action == ACTION_WRITE
    }

    async fn dispatch(&self, ctx: DispatchContext<'_>) -> Result<Handle, ExporterError> {
        if !self.accepts(ctx.action) {
            return Err(ExporterError::UnsupportedAction {
                exporter_slug: SLUG.into(),
                action: ctx.action.into(),
            });
        }
        let params: FileDispatchParams = serde_json::from_value(ctx.params.clone())
            .map_err(|e| ExporterError::BackendRejected(format!("invalid file params: {e}")))?;

        // Resolve `~` / `~/` and enforce absolute-path only. Silent
        // fallback removed after the 2026-07-20 Tauri-dev incident
        // where HOME did not propagate to the backend process and a
        // literal `~/selection1/…` directory got created under the
        // process CWD.
        let output_dir_expanded = resolve_output_dir(&params.output_dir)?;
        let output_dir = PathBuf::from(&output_dir_expanded);
        // Reference mode writes no payload, so it normally has no
        // reason to create the directory — but it still writes
        // sidecars and the schema manifest there when asked. Creating
        // it whenever *something* lands inside is the condition that
        // actually matters; keying on the mode alone made
        // `reference` + `emit_metadata` fail with ENOENT unless the
        // manifest happened to create the directory first.
        let writes_into_output_dir = params.mode != WriteMode::Reference
            || params.emit_metadata
            || params.emit_schema_manifest;
        if writes_into_output_dir {
            std::fs::create_dir_all(&output_dir).map_err(|e| {
                ExporterError::BackendRejected(format!(
                    "create output_dir {}: {e}",
                    output_dir.display()
                ))
            })?;
        }

        // Instruction mode writes a single dispatch-scoped JSON file
        // and returns exactly one Derived. Short-circuit before the
        // per-input loop so the rest of the file can keep assuming
        // "one Derived per input".
        if params.mode == WriteMode::Instruction {
            let template = params
                .filename_template
                .clone()
                .unwrap_or_else(|| "{{dispatch_id}}.json".into());
            let filename = TemplateEnv::for_dispatch(&ctx).render(&template);
            let target = output_dir.join(&filename);

            let body = serde_json::json!({
                "dispatch_id": ctx.dispatch_id,
                "selection_id": ctx.selection_id,
                "persona_id": ctx.persona_id,
                "action": ctx.action,
                "instruction": params.instruction.clone().unwrap_or(serde_json::Value::Null),
                // Serialise the full AssetCardDto per input — the
                // wire shape backends already know from the Tauri
                // grid, filtered later per params.metadata_fields.
                "inputs": ctx.inputs.iter().map(|i| filter_card(i, params.metadata_fields.as_deref())).collect::<Vec<_>>(),
            });
            let bytes = serde_json::to_vec_pretty(&body).map_err(|e| {
                ExporterError::BackendRejected(format!("serialize instruction: {e}"))
            })?;
            std::fs::write(&target, &bytes).map_err(|e| {
                ExporterError::BackendRejected(format!(
                    "write instruction {}: {e}",
                    target.display()
                ))
            })?;

            let modality = params
                .modality
                .clone()
                .unwrap_or_else(|| "instruction".into());
            let mut labels = params.labels.clone();
            labels.push(format!("file:{}", mode_slug(WriteMode::Instruction)));

            let now = Utc::now();
            let final_locator = target.display().to_string();
            let derived = vec![Derived {
                modality,
                locator: final_locator.clone(),
                occurred_at: now,
                cover_hint: None,
                register_note: None,
                labels,
                file_size_bytes: Some(bytes.len() as u64),
                duration_ms: None,
                extra: serde_json::json!({
                    "file": {
                        "mode": mode_slug(WriteMode::Instruction),
                        "output": final_locator,
                        "input_count": ctx.inputs.len(),
                    }
                }),
                batch_hint: None,
            }];
            let payload = FileHandlePayload { derived };
            return Ok(Handle::new(SLUG, serde_json::to_value(payload).unwrap()));
        }

        let template = params
            .filename_template
            .clone()
            .unwrap_or_else(|| "{{basename}}".into());

        let mut used: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        let mut derived: Vec<Derived> = Vec::with_capacity(ctx.inputs.len());
        let now = Utc::now();

        // Optional dispatch-scoped schema manifest — written once at
        // the head of the dispatch dir so a receiver can discover
        // the sidecar shape without hitting the CLI.
        if params.emit_schema_manifest {
            let schema_dir = output_dir.join("_schema");
            std::fs::create_dir_all(&schema_dir).map_err(|e| {
                ExporterError::BackendRejected(format!(
                    "create schema dir {}: {e}",
                    schema_dir.display()
                ))
            })?;
            let schema_path = schema_dir.join("asset_card.json");
            std::fs::write(
                &schema_path,
                asterism_dispatch_sdk::schema::asset_card_example_json(),
            )
            .map_err(|e| {
                ExporterError::BackendRejected(format!(
                    "write schema manifest {}: {e}",
                    schema_path.display()
                ))
            })?;
        }

        for (index, input) in ctx.inputs.iter().enumerate() {
            let subst = TemplateEnv::per_input(input, index, &ctx);
            let filename = subst.render(&template);
            let mut target = output_dir.join(&filename);
            let mut collision_counter = 1u32;
            while !used.insert(target.clone()) {
                target = disambiguate(&output_dir.join(&filename), collision_counter);
                collision_counter += 1;
            }

            let final_locator = match params.mode {
                WriteMode::Copy => {
                    let src = Path::new(&input.source_locator);
                    std::fs::copy(src, &target).map_err(|e| {
                        ExporterError::BackendRejected(format!(
                            "copy {} -> {}: {e}",
                            input.source_locator,
                            target.display()
                        ))
                    })?;
                    target.display().to_string()
                }
                WriteMode::Symlink => {
                    // If a stale symlink is sitting where we want to
                    // write, remove it first so `symlink` does not
                    // fail with EEXIST. This only clears symlinks
                    // (not real files) so a filename collision with
                    // a physical file below `output_dir` still
                    // surfaces as an error.
                    if target.symlink_metadata().is_ok() {
                        let _ = std::fs::remove_file(&target);
                    }
                    #[cfg(unix)]
                    {
                        std::os::unix::fs::symlink(&input.source_locator, &target).map_err(
                            |e| {
                                ExporterError::BackendRejected(format!(
                                    "symlink {} -> {}: {e}",
                                    input.source_locator,
                                    target.display()
                                ))
                            },
                        )?;
                    }
                    #[cfg(not(unix))]
                    {
                        return Err(ExporterError::BackendRejected(
                            "symlink mode is only supported on unix".into(),
                        ));
                    }
                    target.display().to_string()
                }
                WriteMode::Reference => input.source_locator.clone(),
                WriteMode::Instruction => {
                    // Instruction mode short-circuits above, so the
                    // per-input loop never sees it. Reached only if
                    // someone edits the top-of-fn guard without
                    // updating this match — surface it loudly.
                    unreachable!("instruction mode handled before per-input loop")
                }
            };

            // Fallback chain: explicit param → the input's own
            // classification → `work_product` (an unclassified input,
            // asset-model v4, still needs a concrete slug because
            // `DerivedDto.modality` is not optional yet — the exported
            // artefact is a work product of the dispatch either way).
            let modality = params
                .modality
                .clone()
                .or_else(|| input.modality.clone())
                .unwrap_or_else(|| "work_product".to_string());

            let mut labels = params.labels.clone();
            labels.push(format!("file:{}", mode_slug(params.mode)));

            // Optional per-input sidecar with the AssetCardDto JSON
            // so a receiver watching output_dir sees the metadata
            // alongside the payload without another round trip. The
            // sidecar sits next to the payload (or, for reference
            // mode, next to what would have been the payload) as
            // `<filename>.meta.json`.
            if params.emit_metadata {
                let sidecar_path = target.with_extension({
                    let mut ext = target
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    if !ext.is_empty() {
                        ext.push('.');
                    }
                    ext.push_str("meta.json");
                    ext
                });
                let mut body = filter_card(input, params.metadata_fields.as_deref());
                stamp_sidecar_identity(&mut body, &ctx, input);
                let bytes = serde_json::to_vec_pretty(&body).map_err(|e| {
                    ExporterError::BackendRejected(format!("serialize metadata sidecar: {e}"))
                })?;
                std::fs::write(&sidecar_path, &bytes).map_err(|e| {
                    ExporterError::BackendRejected(format!(
                        "write metadata sidecar {}: {e}",
                        sidecar_path.display()
                    ))
                })?;
            }

            derived.push(Derived {
                modality,
                locator: final_locator.clone(),
                occurred_at: now,
                cover_hint: None,
                register_note: None,
                labels,
                file_size_bytes: None,
                duration_ms: None,
                extra: serde_json::json!({
                    "file": {
                        "mode": mode_slug(params.mode),
                        "source": input.source_locator,
                        "output": final_locator,
                        "index": index,
                    }
                }),
                batch_hint: None,
            });
        }

        let payload = FileHandlePayload { derived };
        Ok(Handle::new(SLUG, serde_json::to_value(payload).unwrap()))
    }

    async fn poll(
        &self,
        _ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<DispatchState, ExporterError> {
        // Filesystem work already ran during `dispatch`; nothing to
        // wait for. Guard the kind slug so a misrouted handle fails
        // fast instead of silently succeeding.
        check_kind(handle)?;
        Ok(DispatchState::Done)
    }

    async fn harvest(
        &self,
        _ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<Vec<Derived>, ExporterError> {
        check_kind(handle)?;
        let payload: FileHandlePayload = serde_json::from_value(handle.payload.clone())
            .map_err(|e| ExporterError::BackendRejected(format!("corrupt file handle: {e}")))?;
        Ok(payload.derived)
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

/// Resolves the caller-supplied `output_dir` into an absolute
/// filesystem path or fails loudly.
///
/// Two rejections, both trigger [`ExporterError::BackendRejected`]:
///
/// 1. A `~` / `~/`-prefixed path where `$HOME` is not readable
///    (older versions returned the input verbatim, which caused a
///    literal `~` directory to be created under the process CWD on
///    macOS Tauri dev builds where HOME did not propagate — the
///    2026-07-20 `~/selection1/…` fallout).
/// 2. A relative path. The exporter is not a shell; the caller must
///    hand it an absolute path (or a `~`-prefixed path that resolves
///    to one via HOME).
///
/// Anything already absolute passes through verbatim. Nested `~`
/// inside a segment (`/tmp/~/foo`) is treated as literal — no shell
/// glob semantics.
fn resolve_output_dir(path: &str) -> Result<String, ExporterError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ExporterError::BackendRejected(
            "output_dir must not be empty".into(),
        ));
    }
    let resolved: String = if trimmed == "~" {
        std::env::var_os("HOME")
            .ok_or_else(|| {
                ExporterError::BackendRejected(
                    "output_dir=~ requires $HOME to be set; \
                     pass an absolute path instead"
                        .into(),
                )
            })?
            .to_string_lossy()
            .into_owned()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = std::env::var_os("HOME").ok_or_else(|| {
            ExporterError::BackendRejected(
                "output_dir starts with '~/' but $HOME is not set; \
                 pass an absolute path instead"
                    .into(),
            )
        })?;
        format!("{}/{}", home.to_string_lossy(), rest)
    } else {
        trimmed.to_string()
    };
    if !resolved.starts_with('/') {
        return Err(ExporterError::BackendRejected(format!(
            "output_dir must be an absolute path (starts with '/'); got {resolved:?}"
        )));
    }
    Ok(resolved)
}

fn mode_slug(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::Copy => "copy",
        WriteMode::Symlink => "symlink",
        WriteMode::Reference => "reference",
        WriteMode::Instruction => "instruction",
    }
}

/// Appends `-<n>` before the extension of a candidate path so
/// collisions do not overwrite earlier writes.
fn disambiguate(candidate: &Path, n: u32) -> PathBuf {
    let parent = candidate.parent().unwrap_or_else(|| Path::new("."));
    let stem = candidate
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("out");
    match candidate.extension().and_then(|s| s.to_str()) {
        Some(ext) => parent.join(format!("{stem}-{n}.{ext}")),
        None => parent.join(format!("{stem}-{n}")),
    }
}

/// Filename-template substitution environment. Kept tiny on purpose
/// — the docs enumerate every placeholder the callers can rely on.
struct TemplateEnv<'a> {
    /// Locator of the input for `{{basename}}` / `{{stem}}` /
    /// `{{ext}}` derivation. Empty when the env is dispatch-scoped
    /// (instruction mode), which makes those placeholders render as
    /// empty strings.
    source_locator: &'a str,
    index: usize,
    selection_id: &'a str,
    dispatch_id: &'a str,
    persona_id: &'a str,
    action: &'a str,
}

impl<'a> TemplateEnv<'a> {
    /// Per-input env for the copy / symlink / reference loop.
    fn per_input(input: &'a AssetCardDto, index: usize, ctx: &'a DispatchContext<'a>) -> Self {
        Self {
            source_locator: &input.source_locator,
            index,
            selection_id: ctx.selection_id,
            dispatch_id: ctx.dispatch_id,
            persona_id: ctx.persona_id,
            action: ctx.action,
        }
    }

    /// Dispatch-scoped env for [`WriteMode::Instruction`]. The
    /// per-input placeholders (`basename` / `stem` / `ext` / `index`)
    /// render as empty / `0` since there is no single input to point
    /// at; the interesting placeholders are `dispatch_id` /
    /// `selection_id` / `persona_id` / `action`.
    fn for_dispatch(ctx: &'a DispatchContext<'a>) -> Self {
        Self {
            source_locator: "",
            index: 0,
            selection_id: ctx.selection_id,
            dispatch_id: ctx.dispatch_id,
            persona_id: ctx.persona_id,
            action: ctx.action,
        }
    }

    fn render(&self, template: &str) -> String {
        let (basename, stem, ext) = split_source_name(self.source_locator);
        template
            .replace("{{basename}}", &basename)
            .replace("{{stem}}", &stem)
            .replace("{{ext}}", &ext)
            .replace("{{index}}", &self.index.to_string())
            .replace("{{selection_id}}", self.selection_id)
            .replace("{{dispatch_id}}", self.dispatch_id)
            .replace("{{persona_id}}", self.persona_id)
            .replace("{{action}}", self.action)
    }
}

/// Serialises an [`AssetCardDto`] to JSON, optionally filtered by
/// a caller-supplied allowlist of top-level field names.
/// `None` / an empty list yields the full DTO.
/// Writes the export's identity into a sidecar body.
///
/// Without this, a sidecar says only *what was exported* (the input
/// card). With it, a returning artefact can also say *which export it
/// came out of* — `dispatch_id` resolves to the assets this dispatch
/// produced, which is the hop the file actually travelled through.
/// The source id stays alongside as the fallback for readers (and
/// exports) that predate this block.
///
/// Deliberately applied *after* `metadata_fields` filtering: that
/// allowlist chooses which card fields to disclose, which is a
/// different question from whether the file knows where it came from.
fn stamp_sidecar_identity(
    body: &mut serde_json::Value,
    ctx: &DispatchContext<'_>,
    input: &AssetCardDto,
) {
    let mut identity = serde_json::json!({
        "schema": SIDECAR_SCHEMA,
        "exporter_slug": SLUG,
        "source_asset_id": input.id,
    });
    // The id fields go in under the contract's field consts — the
    // reader lives in a crate this one cannot see, and a spelling
    // drift between the two fails silently as "no identity here".
    identity[asterism_contract::sidecar::SIDECAR_DISPATCH_ID_FIELD] =
        serde_json::json!(ctx.dispatch_id);
    // The pursuit stamp travels beside the dispatch id (#29): the
    // dispatch names the hop, the pursuit names the line of work, and
    // a returning artefact can keep the second even when truncation
    // costs it the first. Absent rather than null when the job
    // predates the stamp — a sidecar states what it knows.
    if let Some(pursuit) = ctx.pursuit_id {
        identity[asterism_contract::sidecar::SIDECAR_PURSUIT_ID_FIELD] = serde_json::json!(pursuit);
    }
    match body {
        serde_json::Value::Object(map) => {
            map.insert(SIDECAR_IDENTITY_KEY.to_string(), identity);
        }
        // `filter_card` always yields an object today; if that ever
        // changes, keep the payload and nest it rather than dropping
        // either half.
        other => {
            let carried = other.take();
            *other = serde_json::json!({ "card": carried, SIDECAR_IDENTITY_KEY: identity });
        }
    }
}

fn filter_card(card: &AssetCardDto, allow: Option<&[String]>) -> serde_json::Value {
    let full = serde_json::to_value(card).expect("AssetCardDto serialises cleanly");
    match allow {
        None => full,
        Some([]) => full,
        Some(list) => {
            let mut out = serde_json::Map::new();
            if let serde_json::Value::Object(map) = full {
                for f in list {
                    if let Some(v) = map.get(f) {
                        out.insert(f.clone(), v.clone());
                    }
                }
            }
            serde_json::Value::Object(out)
        }
    }
}

/// Splits a locator into `(basename, stem, ext)`. Falls back to
/// sensible defaults when the locator has no filename component (a
/// URL, an id-only string).
fn split_source_name(locator: &str) -> (String, String, String) {
    let path = Path::new(locator);
    let basename = path
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| "asset".into());
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| basename.clone());
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_default();
    (basename, stem, ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card_fixture(source_locator: &str) -> AssetCardDto {
        AssetCardDto {
            id: "asset-uuid".into(),
            persona_id: "persona-uuid".into(),
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
        let exp = FileExporter::new();
        assert_eq!(exp.slug(), SLUG);
        assert!(exp.accepts(ACTION_WRITE));
        assert!(!exp.accepts("copy"));
        assert!(!exp.accepts(""));
    }

    #[test]
    fn split_source_name_handles_url_and_path_and_id_only() {
        assert_eq!(
            split_source_name("/tmp/foo/bar.png"),
            ("bar.png".to_string(), "bar".to_string(), "png".to_string())
        );
        assert_eq!(
            split_source_name("no-extension"),
            (
                "no-extension".to_string(),
                "no-extension".to_string(),
                "".to_string()
            )
        );
    }

    #[test]
    fn template_env_replaces_every_documented_placeholder() {
        let env = TemplateEnv {
            source_locator: "/tmp/album/photo.png",
            index: 3,
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            persona_id: "persona-uuid",
            action: "write",
        };
        let out = env.render(
            "{{selection_id}}/{{dispatch_id}}/{{persona_id}}/{{action}}/\
             {{index}}-{{stem}}.{{ext}}",
        );
        assert_eq!(out, "sel-1/disp-1/persona-uuid/write/3-photo.png");
        assert_eq!(env.render("{{basename}}"), "photo.png");
    }

    #[test]
    fn disambiguate_appends_counter_before_extension() {
        let candidate = Path::new("/out/photo.png");
        assert_eq!(disambiguate(candidate, 1), Path::new("/out/photo-1.png"));
        let ext_less = Path::new("/out/photo");
        assert_eq!(disambiguate(ext_less, 2), Path::new("/out/photo-2"));
    }

    #[test]
    fn resolve_output_dir_expands_tilde_and_rejects_relative() {
        let home = std::env::var("HOME").expect("HOME is always set on unix");
        // Tilde forms resolve to $HOME.
        assert_eq!(resolve_output_dir("~").unwrap(), home);
        assert_eq!(
            resolve_output_dir("~/selection1").unwrap(),
            format!("{home}/selection1")
        );
        // Absolute paths pass through verbatim.
        assert_eq!(
            resolve_output_dir("/absolute/path").unwrap(),
            "/absolute/path"
        );
        // Nested `~` inside an already-absolute path is left as
        // literal — no shell glob semantics.
        assert_eq!(resolve_output_dir("/tmp/~/foo").unwrap(), "/tmp/~/foo");
        // Relative paths are rejected (the 2026-07-20 fallout: a
        // literal `~/selection1` under CWD was the tell that the
        // silent fallback was wrong).
        assert!(matches!(
            resolve_output_dir("relative/path"),
            Err(ExporterError::BackendRejected(_))
        ));
        assert!(matches!(
            resolve_output_dir(""),
            Err(ExporterError::BackendRejected(_))
        ));
    }

    #[tokio::test]
    async fn reference_mode_with_emit_metadata_writes_sidecar_and_schema_manifest() {
        use asterism_dispatch_sdk::DispatchContext;

        let tmp = tempfile::tempdir().expect("tempdir");
        let out = tmp.path().join("dispatch");

        let inputs = vec![card_fixture("/tmp/photo-a.png")];
        let params = serde_json::json!({
            "output_dir": out.display().to_string(),
            "mode": "reference",
            "emit_metadata": true,
            "emit_schema_manifest": true,
            "metadata_fields": ["id", "source_locator", "labels"]
        });
        let ctx = DispatchContext {
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            pursuit_id: None,
            persona_id: "p1",
            action: ACTION_WRITE,
            params: &params,
            inputs: &inputs,
        };

        let exp = FileExporter::new();
        let handle = exp.dispatch(ctx).await.expect("dispatch");
        let derived = exp.harvest(ctx, &handle).await.expect("harvest");

        assert_eq!(derived.len(), 1);
        // Reference mode leaves the derived locator pointing at the
        // original source, but the sidecar lands next to where the
        // payload would sit.
        assert_eq!(derived[0].locator, "/tmp/photo-a.png");

        let sidecar = out.join("photo-a.png.meta.json");
        assert!(
            sidecar.exists(),
            "sidecar file exists at {}",
            sidecar.display()
        );
        let body: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
        // metadata_fields allowlist keeps only the requested card
        // fields — plus the identity block, which is not a card field
        // and answers a different question (which export produced
        // this) than the allowlist governs (what to disclose).
        let obj = body.as_object().unwrap();
        assert_eq!(obj.len(), 4);
        assert!(obj.contains_key("id"));
        assert!(obj.contains_key("source_locator"));
        assert!(obj.contains_key("labels"));
        assert!(obj.contains_key("_asterism"));

        let manifest = out.join("_schema").join("asset_card.json");
        assert!(manifest.exists(), "schema manifest exists");
    }

    #[tokio::test]
    async fn emit_metadata_defaults_on_so_an_omitting_caller_still_gets_the_sidecar() {
        // The default is the claim under test: a params payload that
        // never mentions `emit_metadata` must still produce the
        // sidecar, because the Re-In roundtrip depends on it and a
        // caller opt-in default was rejected.
        use asterism_dispatch_sdk::DispatchContext;

        let tmp = tempfile::tempdir().expect("tempdir");
        let out = tmp.path().join("dispatch");

        let inputs = vec![card_fixture("/tmp/photo-a.png")];
        let params = serde_json::json!({
            "output_dir": out.display().to_string(),
            "mode": "reference"
        });
        let ctx = DispatchContext {
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            pursuit_id: None,
            persona_id: "p1",
            action: ACTION_WRITE,
            params: &params,
            inputs: &inputs,
        };

        let exp = FileExporter::new();
        let handle = exp.dispatch(ctx).await.expect("dispatch");
        exp.harvest(ctx, &handle).await.expect("harvest");

        let sidecar = out.join("photo-a.png.meta.json");
        assert!(
            sidecar.exists(),
            "default-on emit_metadata wrote {}",
            sidecar.display()
        );
    }

    #[tokio::test]
    async fn the_sidecar_names_the_dispatch_it_came_out_of() {
        // A returning artefact has to be able to say which hop it
        // travelled through. The card alone only says what was
        // exported, so the identity block carries the dispatch id —
        // that is what `derived_from: dispatch:<id>` resolves against.
        use asterism_dispatch_sdk::DispatchContext;

        let tmp = tempfile::tempdir().expect("tempdir");
        let out = tmp.path().join("dispatch");
        let inputs = vec![card_fixture("/tmp/photo-a.png")];
        let params = serde_json::json!({
            "output_dir": out.display().to_string(),
            "mode": "reference",
            "emit_metadata": true
        });
        let ctx = DispatchContext {
            selection_id: "sel-1",
            dispatch_id: "0198c1c2-0000-7000-8000-000000000001",
            pursuit_id: Some("0198c1c2-0000-7000-8000-0000000000aa"),
            persona_id: "p1",
            action: ACTION_WRITE,
            params: &params,
            inputs: &inputs,
        };

        let exp = FileExporter::new();
        let handle = exp.dispatch(ctx).await.expect("dispatch");
        let _ = exp.harvest(ctx, &handle).await.expect("harvest");

        let body: serde_json::Value = serde_json::from_slice(
            &std::fs::read(out.join("photo-a.png.meta.json")).expect("sidecar"),
        )
        .unwrap();
        let identity = body.get("_asterism").expect("identity block");
        assert_eq!(
            identity.get("schema").and_then(|v| v.as_str()),
            Some("asterism.sidecar/1"),
            "the block is versioned — this file can come back months later"
        );
        assert_eq!(
            identity.get("dispatch_id").and_then(|v| v.as_str()),
            Some("0198c1c2-0000-7000-8000-000000000001")
        );
        // The line of work travels beside the hop (#29).
        assert_eq!(
            identity.get("pursuit_id").and_then(|v| v.as_str()),
            Some("0198c1c2-0000-7000-8000-0000000000aa")
        );
        assert_eq!(
            identity.get("exporter_slug").and_then(|v| v.as_str()),
            Some(SLUG)
        );
        // The source id stays available as the fallback target for a
        // reader that cannot resolve the dispatch.
        assert_eq!(
            identity.get("source_asset_id").and_then(|v| v.as_str()),
            body.get("id").and_then(|v| v.as_str())
        );
    }

    #[tokio::test]
    async fn instruction_mode_writes_single_json_and_returns_one_derived() {
        use asterism_dispatch_sdk::DispatchContext;

        let tmp = tempfile::tempdir().expect("tempdir");
        let out = tmp.path().join("dispatch");

        let inputs = vec![card_fixture("/tmp/a.png"), card_fixture("/tmp/b.png")];
        let params = serde_json::json!({
            "output_dir": out.display().to_string(),
            "mode": "instruction",
            "instruction": { "workflow": "wf-1", "prompt": "hello" },
        });
        let ctx = DispatchContext {
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            pursuit_id: None,
            persona_id: "p1",
            action: ACTION_WRITE,
            params: &params,
            inputs: &inputs,
        };

        let exp = FileExporter::new();
        let handle = exp.dispatch(ctx).await.expect("dispatch");
        let derived = exp.harvest(ctx, &handle).await.expect("harvest");

        assert_eq!(derived.len(), 1);
        let d = &derived[0];
        assert_eq!(d.modality, "instruction");
        assert!(d.labels.contains(&"file:instruction".to_string()));

        let written = std::path::Path::new(&d.locator);
        assert!(written.exists(), "instruction file exists on disk");
        assert_eq!(written.file_name().unwrap(), "disp-1.json");
        let body: serde_json::Value =
            serde_json::from_slice(&std::fs::read(written).unwrap()).unwrap();
        assert_eq!(body["dispatch_id"], "disp-1");
        assert_eq!(body["instruction"]["workflow"], "wf-1");
        assert_eq!(body["inputs"].as_array().unwrap().len(), 2);
        // Card fixture stamps the same id on every instance — the
        // point of the assertion is the AssetCardDto shape landed
        // as `inputs[i]`.
        assert_eq!(body["inputs"][0]["id"], "asset-uuid");
        assert_eq!(body["inputs"][0]["source_locator"], "/tmp/a.png");
        assert_eq!(body["inputs"][1]["source_locator"], "/tmp/b.png");
    }

    /// The shipped example is what `asterism-server schema print
    /// exporter:file:params` hands a caller, so it has to survive the
    /// same `serde_json::from_value` `dispatch` runs on `ctx.params`.
    /// The http exporter's copy of this file had drifted away from its
    /// struct with nothing failing; this crate's example is currently
    /// in sync, and this is what keeps it that way.
    #[test]
    fn params_example_deserialises_into_the_current_struct() {
        let params: FileDispatchParams = serde_json::from_str(params_example_json())
            .expect("schema/file_params.example.json must parse as FileDispatchParams");
        assert_eq!(params.mode, WriteMode::Instruction);
        assert_eq!(
            params.filename_template.as_deref(),
            Some("{{dispatch_id}}.json")
        );
        // Instruction mode is the only mode that reads this field, and
        // the example advertises that mode — an example that dropped
        // the blob would document a no-op dispatch.
        assert!(params.instruction.is_some());
    }

    #[test]
    fn write_mode_defaults_to_reference_and_serdes_as_slug() {
        assert_eq!(WriteMode::default(), WriteMode::Reference);
        let round_trip: WriteMode = serde_json::from_value(serde_json::json!("copy")).unwrap();
        assert_eq!(round_trip, WriteMode::Copy);
    }
}
