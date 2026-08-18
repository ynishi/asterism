//! Pipeline job handlers: `cover_gen`, `auto_tag`, `edge_rebuild`.
//!
//! Every handler is idempotent and can be re-derived from persisted
//! state — if an enqueue is lost the missing signal (for example a null
//! cover) can trigger another run. v1 uses simple heuristics for cover
//! text and keywords; higher-quality generation via an LLM is planned
//! for later modalities.

use asterism_core::application_support::duplicate_detection::{
    Detection, DetectionOrigin, DetectionPorts, detect_duplicate,
};
use asterism_core::domain::asset::{Asset, ContentFlags};
use asterism_core::domain::constellation::plan_edges;
use asterism_core::domain::content_hash::{self, UNHASHABLE};
use asterism_core::domain::derived_text::derive_text;
use asterism_core::domain::duplicate_conflict::DuplicateAxis;
use asterism_core::domain::provenance;
use asterism_core::domain::render::render_policy;
use asterism_core::domain::repository::{
    AssetBodyRepository, AssetCommentRepository, AssetRepository, DimsProbe, DimsScope,
    DimsWritePolicy, EdgeRepository, IndexDoc, JobQueue, MaterialFingerprint, ModalityRepository,
    SeriesRepository, SourceTextReader, TagRepository, TextLocator, ThumbRepository,
};
use asterism_core::domain::series::SeriesKey;
use asterism_core::domain::source_locator::SourceLocator;
use asterism_core::domain::value::{
    AssetId, AssetRole, CoverTemplate, CoverText, Keyword, MimeType, StrategyId,
};
use asterism_core::error::DomainError;

use super::JobEnv;
use crate::fingerprint::{MAX_CONTENT_WALK_BYTES, hash_artefact};

/// JPEG quality used for cache thumbnails (0..=100). Higher is not
/// worth the extra bytes at grid sizes.
#[cfg(not(target_os = "macos"))]
const THUMB_JPEG_QUALITY: u8 = 82;

/// Ceiling on concurrent thumbnail decodes across the whole process.
///
/// `thumb_gen` is a CPU-bound decode + resize (JPEG DCT + Lanczos3 on
/// a full ~19 MB RGB buffer), not an IO wait, so any parallelism
/// pushes real cores toward 100 %. During stress imports that made
/// the host unusable for other work — a plain browser scroll or an
/// editor keystroke would jank while the queue drained. Capping
/// concurrent decodes at 1 keeps the wave to a single core and
/// leaves the rest of the box responsive; total wall time for the
/// wave is unchanged from the "pin every core" case on typical
/// desktops because grid paint only waits for the current batch's
/// 128 px thumbs, not the full backlog.
#[cfg(target_os = "macos")]
static THUMB_DECODE_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
#[cfg(not(target_os = "macos"))]
static THUMB_DECODE_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// One preview transcode at a time. A rendition re-encodes every
/// frame — minutes of a core for a long clip — and it is triggered by
/// a human opening one detail pane, so there is no burst to absorb;
/// serialising keeps a queue of stale requests from pinning the CPU.
static PREVIEW_TRANSCODE_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// Fires the Query Group invalidator for a persona whose rule inputs
/// a handler just rewrote (W4-a: `auto_tag` writes the `tag_ids`
/// dimension, `index_rebuild` the `search_text` dimension). No-op when
/// the late-bound cell is empty (tests / preview tooling). Bursty
/// callers (import chains, backfill pages) are safe — the invalidator
/// debounces per persona. The `query_group_refresh` handler must NOT
/// call this (self-trigger loop, job-write exclusion).
fn notify_query_groups(env: &JobEnv, persona_id: asterism_core::domain::value::PersonaId) {
    if let Some(invalidator) = env.deps.query_group_invalidator.get() {
        invalidator.notify_persona(persona_id);
    }
}

/// Loads the target asset from the payload; returns `Ok(None)` when
/// the asset has been deleted (the handler treats it as skipped).
async fn load_target(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<Option<Asset>, DomainError> {
    let asset_id = payload
        .get("asset_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DomainError::Validation("job payload missing asset_id".into()))?;
    let uuid = uuid::Uuid::parse_str(asset_id)
        .map_err(|_| DomainError::Validation(format!("invalid asset_id: {asset_id:?}")))?;
    env.deps
        .assets
        .find(&asterism_core::domain::value::AssetId::from_uuid(uuid))
        .await
}

/// Character-boundary-safe truncation.
fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Derives a cover from the original content using the resolved
/// [`CoverTemplate`].
///
/// The template is resolved from the Modality master (its
/// `cover_template` override, else the kind's default) by [`cover_gen`];
/// this function only applies it. Slug-literal branching is gone — the
/// template *is* the behaviour selector.
fn derive_cover(template: CoverTemplate, content: Option<&str>, locator: &SourceLocator) -> String {
    // The fallback wants a name a person would recognise, so it takes
    // the file stem where there is a file to take one from — the
    // container's, for a record, since that is the artefact on disk.
    // A remote or a logical name has no stem to take: its own rendering
    // *is* the name, and running `file_stem` over it would cut a URL at
    // its last dot. The rendering taken is the display one — a cover is
    // read by a person, and the storage form is a tagged JSON object.
    let fallback = || {
        let path = match locator {
            SourceLocator::File(path) => Some(path.as_path()),
            SourceLocator::Record(record) => Some(record.container().as_path()),
            SourceLocator::Remote(_) | SourceLocator::Logical(_) => None,
        };
        path.and_then(std::path::Path::file_stem)
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| locator.to_display())
    };
    let Some(content) = content else {
        return fallback();
    };
    let lines: Vec<&str> = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return fallback();
    }
    let strip_heading = |line: &str| line.trim_start_matches('#').trim().to_string();

    let cover = match template {
        // Dialogue — the first one or two non-empty lines verbatim.
        CoverTemplate::Dialogue => lines
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(" / "),
        // Work product — the title (optionally with the first body line).
        CoverTemplate::WorkProduct => {
            let title = lines
                .iter()
                .find(|l| l.starts_with('#'))
                .map(|l| strip_heading(l))
                .unwrap_or_else(|| lines[0].to_string());
            match lines.iter().find(|l| !l.starts_with('#')) {
                Some(body) if *body != title => format!("{title} — {body}"),
                _ => title,
            }
        }
        // Terminal Tape — first prompt line starting with '❯', or first line.
        CoverTemplate::Tape => lines
            .iter()
            .find(|l| l.starts_with('❯'))
            .copied()
            .unwrap_or(lines[0])
            .to_string(),
        // Generic fallback — the first meaningful line.
        CoverTemplate::FirstLine => strip_heading(lines[0]),
    };
    truncate_chars(&cover, 120)
}

/// Auto-generates the card cover. Idempotent — if the cover column is
/// already populated the handler skips.
///
/// Writes only the cover column (partial `UPDATE`) so that concurrent
/// handlers do not race through a read-modify-write cycle. The full-row
/// upsert path was observed clobbering the cover value in practice.
pub async fn cover_gen(env: &JobEnv, payload: &serde_json::Value) -> Result<String, DomainError> {
    let Some(asset) = load_target(env, payload).await? else {
        return Ok("asset gone, skipped".into());
    };
    let content = read_source(&asset).await;
    // Content-flag detection piggybacks on the same source read. It
    // runs every time (not just on first cover_gen) so re-enqueuing
    // the job on an asset with a stale flag set refreshes it — the
    // migration backfill uses the cover snippet as a stand-in, so we
    // still want the full-body pass to happen eventually.
    //
    // Deliberately NOT wired to `notify_query_groups` (W4-a review):
    // `content_flags` / `cover` are not query-rule inputs — the
    // filter contract (`ListAssetsQuery` / `AssetQuery`) has no
    // `has_*` / cover dimension, so a refresh here would re-evaluate
    // to the same memberships. Add the notify alongside the filter
    // dimension if one ever lands.
    if let Some(body) = content.as_deref() {
        let flags = ContentFlags::detect(body);
        env.deps.assets.set_content_flags(&asset.id, flags).await?;
    }
    if asset.cover.is_some() {
        return Ok("cover already generated, flags refreshed".into());
    }
    // A container owns no material, so there is nothing to read and
    // `derive_cover` would fall back to the locator — a bare UUID. Its
    // content is its members, so it reads as its earliest one. Runs
    // again on every re-enqueue until a member actually has a cover
    // (members are ingested before their own `cover_gen` completes).
    if asset.role == AssetRole::Collection {
        return match env.deps.assets.first_member_cover(&asset.id).await? {
            Some(cover) => {
                env.deps
                    .assets
                    .set_cover(&asset.id, &CoverText::new(cover)?)
                    .await?;
                enqueue_reindex(env, &asset.id).await?;
                Ok("cover taken from earliest member".into())
            }
            None => Ok("container has no covered member yet, skipped".into()),
        };
    }
    // Resolve the cover template. Classified assets go through the
    // Modality master: the row's `cover_template` override, else the
    // kind default; an unregistered slug (importer escape hatch, no
    // master row) falls back to the generic first-line template.
    // Unclassified assets (asset-model v4: conversation members and
    // other modality-NULL rows) key on the structural facts instead —
    // a textual material inside a container keeps the Dialogue
    // wording the old `dialogue` master row used to provide.
    let template = match &asset.modality {
        Some(modality) => match env.deps.modalities.find(modality).await? {
            Some(def) => def.cover_template.unwrap_or(CoverTemplate::FirstLine),
            None => CoverTemplate::FirstLine,
        },
        None => {
            let textual = asset
                .materials
                .iter()
                .any(|m| m.mime.as_ref().is_some_and(MimeType::body_text));
            if textual && asset.container_id.is_some() {
                CoverTemplate::Dialogue
            } else {
                CoverTemplate::FirstLine
            }
        }
    };
    let cover = derive_cover(template, content.as_deref(), &asset.source.locator);
    env.deps
        .assets
        .set_cover(&asset.id, &CoverText::new(cover)?)
        .await?;
    enqueue_reindex(env, &asset.id).await?;
    Ok("cover generated".into())
}

/// Re-composes one asset's search document after a handler wrote a
/// field the document is derived from.
///
/// Every fan-out at ingest enqueues `IndexRebuild` alongside the jobs
/// below it, and those jobs then write the fields the document is made
/// of — a cover, keywords, the metadata a container carried. Whichever
/// order the queue drains them in, the document composed first was
/// composed from less than the row now says, so the write is what has
/// to re-enqueue.
async fn enqueue_reindex(env: &JobEnv, asset_id: &AssetId) -> Result<(), DomainError> {
    env.queue
        .enqueue(
            asterism_core::domain::job::JobKind::IndexRebuild,
            serde_json::json!({ "asset_id": asset_id.to_string() }),
        )
        .await?;
    Ok(())
}

/// Reads the original artefact for a filesystem-backed asset. A read
/// failure yields `None` so the handler falls back to metadata; it
/// never fails the job.
async fn read_source(asset: &Asset) -> Option<String> {
    use asterism_core::domain::value::SourceKind;
    if asset.source.kind.as_str() != SourceKind::FS {
        return None;
    }
    // Only a locator that *is* a file has a file to read. A record's
    // container is not it — reading the whole container here would give
    // every message in a log the same cover.
    let path = asset.source.locator.local_path()?;
    tokio::fs::read_to_string(path).await.ok()
}

/// Extracts keywords, materialises channel tags, and links them to the
/// asset.
///
/// v1 heuristic: labels + register annotation + file-stem tokens +
/// leading markdown headings. Capped at 8 to keep the channel space
/// from getting spammed.
pub async fn auto_tag(env: &JobEnv, payload: &serde_json::Value) -> Result<String, DomainError> {
    let Some(asset) = load_target(env, payload).await? else {
        return Ok("asset gone, skipped".into());
    };
    let content = read_source(&asset).await;

    let mut names: Vec<String> = Vec::new();
    let mut push = |candidate: String| {
        let candidate = candidate.trim().to_string();
        if candidate.chars().count() >= 2 && names.len() < 8 && !names.contains(&candidate) {
            names.push(candidate);
        }
    };
    for label in &asset.labels {
        // Reserved system labels (Inbox triage etc.) must not be
        // promoted into user-facing Tags — otherwise the detail
        // pane shows the same word in both Labels and Tags, and
        // clearing the Tag leaves the Label (and therefore the
        // Inbox filter membership) intact. The Tag entity is for
        // organic user tagging; system state stays on Labels.
        if label.as_str() == asterism_core::application::INBOX_LABEL {
            continue;
        }
        push(label.as_str().to_string());
    }
    if let Some(register) = &asset.register_note {
        push(register.as_str().to_string());
    }
    // Keyword mining reads the *file's* name. A record's container name
    // belongs to the container and would tag every record in a log
    // identically; a remote or a logical name has no filename at all.
    if let Some(stem) = asset
        .source
        .locator
        .local_path()
        .and_then(std::path::Path::file_stem)
    {
        for token in stem.to_string_lossy().split(['-', '_', '.', ' ']) {
            if token.chars().any(|c| c.is_alphabetic()) {
                push(token.to_string());
            }
        }
    }
    if let Some(content) = &content {
        for line in content.lines().filter(|l| l.starts_with('#')).take(3) {
            push(line.trim_start_matches('#').trim().to_string());
        }
    }

    let keywords = names
        .iter()
        .cloned()
        .map(Keyword::new)
        .collect::<Result<Vec<_>, _>>()?;
    // Partial `UPDATE` — see `cover_gen` for the rationale.
    env.deps.assets.set_keywords(&asset.id, &keywords).await?;

    for name in &names {
        let tag = env.deps.tags.find_or_create(name).await?;
        env.deps.tags.link(&asset.id, &tag.id).await?;
    }

    // Tag links are rule inputs (the `tag_ids` filter) — refresh the
    // persona's query groups (W4-a). Import batches fire this
    // per asset; the per-persona debounce collapses the burst.
    notify_query_groups(env, asset.persona_id);

    // Chain enqueue: `edge_rebuild` reads the keyword list for the
    // KeywordOverlap axis, so it must run after `auto_tag` finalises
    // the tags (enqueueing both at ingest time races on the ordering).
    env.queue
        .enqueue(
            asterism_core::domain::job::JobKind::EdgeRebuild,
            serde_json::json!({ "asset_id": asset.id.to_string() }),
        )
        .await?;
    // Same ordering argument on the search axis: keywords are one of
    // the sections the derived text is composed from, and the document
    // written at ingest was composed before this handler wrote them.
    enqueue_reindex(env, &asset.id).await?;
    Ok(format!("{} keyword(s) tagged", names.len()))
}

/// Incrementally rebuilds constellation edges for the target asset.
///
/// Candidate fetch is window-scoped (within ±48h of the target or
/// sharing its session id, capped at 200); the weight / label rules
/// live in the pure `plan_edges` domain function.
pub async fn edge_rebuild(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    let Some(asset) = load_target(env, payload).await? else {
        return Ok("asset gone, skipped".into());
    };
    let candidates = env.deps.assets.candidates_near(&asset, 200).await?;
    let edges = plan_edges(&asset, &candidates);
    let count = edges.len();
    env.deps
        .edges
        .replace_synth_edges_of(&asset.id, edges)
        .await?;
    Ok(format!("{count} edge(s) rebuilt"))
}

/// Session reconciliation stub — the precomputed rkyv snapshot was
/// retired when Session became the 1st-class Dialog entity. Aggregates
/// (`message_count` / `started_at_ms` / `ended_at_ms`) are now
/// derived at query time by
/// [`AssetRepository::list_sessions`](asterism_core::domain::repository::AssetRepository::list_sessions)
/// via a LEFT JOIN on the `asset` aggregate, so a snapshot
/// rebuild has no work to do. The handler stays wired so callers
/// enqueueing `SessionRebuild` (the HTTP endpoint / startup path /
/// import chain) still succeed idempotently; a future P2/P3
/// reconciliation pass that writes back the derived columns onto
/// the `session` row can slot in here without a handler-table
/// change. The `sessions:progress` broadcast is preserved for UI
/// compatibility.
pub async fn session_rebuild(
    env: &JobEnv,
    _payload: &serde_json::Value,
) -> Result<String, DomainError> {
    let _ = env
        .deps
        .emitter
        .broadcast("sessions:progress", serde_json::json!({ "phase": "start" }))
        .await;
    // The aggregates are still query-time derived, so there is nothing
    // to reconcile there. What this pass does own is the container
    // cover backfill: a container takes its cover from its earliest
    // member, and ingest only re-enqueues that for members arriving
    // from now on — a container that was already whole never gets a
    // trigger. Idempotent: `cover_gen` no-ops once a cover is set, and
    // a container whose members have no cover yet simply stays in the
    // list for the next run.
    const BACKFILL_LIMIT: u32 = 500;
    let pending = env
        .deps
        .assets
        .containers_without_cover(BACKFILL_LIMIT)
        .await?;
    for id in &pending {
        let _ = env
            .queue
            .enqueue(
                asterism_core::domain::job::JobKind::CoverGen,
                serde_json::json!({ "asset_id": id.to_string() }),
            )
            .await;
    }
    let _ = env
        .deps
        .emitter
        .broadcast(
            "sessions:progress",
            serde_json::json!({
                "phase": "done",
                "ok": true,
                "covers_enqueued": pending.len(),
            }),
        )
        .await;
    Ok(format!(
        "session_rebuild enqueued {} container cover job(s)",
        pending.len()
    ))
}

/// Generates a resized JPEG thumbnail for a visual asset at a
/// specific size and upserts it into `thumb_cache`.
///
/// Payload: `{ "asset_id": <uuid>, "size_px": <u32> }`.
/// Idempotent: `INSERT OR REPLACE` on the `(asset_id, size_px)`
/// key. Images and videos are eligible (a video contributes one
/// extracted frame); anything else returns silently as "not
/// thumbnailable, skipped".
pub async fn thumb_gen(env: &JobEnv, payload: &serde_json::Value) -> Result<String, DomainError> {
    let Some(asset) = load_target(env, payload).await? else {
        return Ok("asset gone, skipped".into());
    };
    // Thumbnail eligibility is a question about the bytes, so it goes
    // to the material's format fact through the single render policy
    // (`asterism_core::domain::render`). This used to branch on the
    // modality instead, with a mime fallback for unclassified rows —
    // and the two disagreed: filing a PNG under `memory` (a `text`
    // kind) made it stop being thumbnailable.
    let mime = primary_mime(&asset).cloned();
    let is_video = matches!(mime, Some(MimeType::Video(_)));
    let policy = render_policy(mime.as_ref(), asset.role, false);
    if !policy.thumbnail {
        return Ok("not thumbnailable, skipped".into());
    }
    // Second layer, on the locator rather than the mime. A record
    // addresses something inside a container (`shot.png#workflow` — a
    // PNG tEXt note), so there is no file of its own for a decoder to
    // open; handing it one failed every such job forever
    // [measured 2026-07-31: 2785 failed rows in the dogfood job_log].
    // `guess_mime` now answers `text/plain` for these and the policy
    // above stops them, but a future classification slip must not be
    // able to reopen a failure loop this cheap to close.
    //
    // `local_path()` is the whole test: it is `Some` exactly when there
    // is a file, and the path it gives is the path — which is the other
    // half of the fix, since the string form of a `file://` locator was
    // never one.
    let Some(path) = asset.source.locator.local_path() else {
        return Ok("no file of its own, not thumbnailable, skipped".into());
    };
    let path_str = path.to_string_lossy().into_owned();
    let size_px = payload
        .get("size_px")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| DomainError::Validation("thumb_gen payload missing size_px".into()))?
        as u32;
    if size_px == 0 {
        return Err(DomainError::Validation(
            "thumb_gen size_px must be > 0".into(),
        ));
    }
    // Serialise decodes against `THUMB_DECODE_SLOTS` so a burst of
    // apalis workers cannot pin an unbounded number of ~19 MB RGB
    // buffers at once.
    let _permit = THUMB_DECODE_SLOTS
        .acquire()
        .await
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("thumb slot acquire: {e}")))?;
    let bytes = tokio::task::spawn_blocking(move || {
        decode_and_encode(&path_str, size_px, is_video, mime.as_ref())
    })
    .await
    .map_err(|e| DomainError::Infra(anyhow::anyhow!("thumb worker join: {e}")))??;
    let byte_len = bytes.len();
    // Extract the dominant-colour palette from the just-generated
    // thumbnail. Only fires on the smallest size we pre-render
    // (128 px) so the extractor pays once per asset regardless of
    // how many sizes the grid asks for. Failures are non-fatal — a
    // corrupted decode does not stop the thumb from being cached.
    if size_px <= 128 && asset.palette.is_none() {
        let thumb_bytes = bytes.clone();
        let palette_result =
            tokio::task::spawn_blocking(move || extract_palette(&thumb_bytes)).await;
        match palette_result {
            Ok(Ok(palette)) => {
                let _ = env.deps.assets.set_palette(&asset.id, Some(palette)).await;
            }
            Ok(Err(err)) => {
                tracing::warn!(
                    event = "diag.palette.skipped",
                    error = %err,
                    "palette_extract skipped"
                );
            }
            Err(join) => {
                tracing::warn!(
                    event = "diag.palette.join_failed",
                    error = %join,
                    "palette_extract worker join failed"
                );
            }
        }
    }
    env.deps.thumbs.upsert(&asset.id, size_px, bytes).await?;
    Ok(format!("thumb {size_px}px cached ({byte_len} bytes)"))
}

/// Transcodes a webview-unplayable video into its preview rendition
/// (`super::preview_ffmpeg`). Payload: `{ "asset_id": <uuid> }`.
///
/// Idempotent: an existing rendition is a completed job. On failure a
/// `.failed` marker carrying the reason is written beside the target
/// so the status endpoint answers "failed: why" instead of keeping
/// the pane on a spinner forever; the marker is cleared before every
/// fresh attempt so a retry starts clean.
pub async fn preview_gen(env: &JobEnv, payload: &serde_json::Value) -> Result<String, DomainError> {
    use super::preview_ffmpeg;
    let Some(asset) = load_target(env, payload).await? else {
        return Ok("asset gone, skipped".into());
    };
    let mime = primary_mime(&asset).cloned();
    if !asterism_core::domain::render::needs_video_preview(mime.as_ref()) {
        return Ok("plays natively, no rendition needed, skipped".into());
    }
    // Same locator-side guard as `thumb_gen`: the transcoder opens a
    // file, and a record names something inside a container rather than
    // a file of its own. Cheaper to refuse here than to let a mime slip
    // turn into a `.failed` marker the pane reports forever.
    let Some(src_path) = asset.source.locator.local_path() else {
        return Ok("no file of its own, no rendition possible, skipped".into());
    };
    let src = src_path.to_string_lossy().into_owned();
    let previews_dir = env.deps.previews_dir.clone();
    let asset_id = asset.id.to_string();
    if preview_ffmpeg::preview_path(&previews_dir, &asset_id).is_file() {
        return Ok("rendition already cached".into());
    }
    std::fs::create_dir_all(&previews_dir)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("previews dir: {e}")))?;
    let _ = std::fs::remove_file(preview_ffmpeg::failed_marker_path(&previews_dir, &asset_id));
    // Touch the staging marker before waiting on the slot, so a pane
    // polling every second sees "in flight" instead of enqueueing a
    // duplicate behind a long transcode. ffmpeg overwrites it (-y),
    // and a crash's stale marker is swept at startup.
    let _ = std::fs::write(
        preview_ffmpeg::part_marker_path(&previews_dir, &asset_id),
        b"",
    );

    let _permit = PREVIEW_TRANSCODE_SLOTS
        .acquire()
        .await
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("preview slot acquire: {e}")))?;
    // Re-check under the slot: a duplicate enqueue that waited here
    // while the first run transcoded finds the file and stops.
    if preview_ffmpeg::preview_path(&previews_dir, &asset_id).is_file() {
        return Ok("rendition already cached".into());
    }
    let dir = previews_dir.clone();
    let id = asset_id.clone();
    let result = tokio::task::spawn_blocking(move || preview_ffmpeg::make_preview(&src, &dir, &id))
        .await
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("preview worker join: {e}")))?;
    // Whatever happened, the staging marker must not outlive the run —
    // a leftover reads as "still transcoding" and mutes re-enqueues.
    let _ = std::fs::remove_file(preview_ffmpeg::part_marker_path(&previews_dir, &asset_id));
    match result {
        Ok(()) => Ok("preview rendition cached".into()),
        Err(err) => {
            let marker = preview_ffmpeg::failed_marker_path(&previews_dir, &asset_id);
            if let Err(write_err) = std::fs::write(&marker, err.to_string()) {
                tracing::warn!(
                    event = "diag.preview.marker_write_failed",
                    error = %write_err,
                    "could not record the preview failure"
                );
            }
            Err(err)
        }
    }
}

/// Page size for one `material_hash` backfill pass. Smaller than the
/// index backfill's page because each item here reads a whole file
/// off disk rather than a row out of the database — a page is sized
/// to hand the worker back quickly, not to drain the backlog.
const MATERIAL_HASH_PAGE: u32 = 50;

/// Fingerprints an original's bytes into `material.content_hash`, then
/// asks what the digest means for the corpus.
///
/// Payload is either `{ "asset_id": <uuid> }` (one asset, from the
/// ingest fan-out) or `{ "batch": true, "cursor": <uuid?> }` (the
/// backfill walk over everything imported before the column existed).
/// **The payload shape is also what tells the two passes apart** for
/// [`DetectionOrigin`] — the branch below is the fact, and a dedicated
/// field would be a second copy of it that whoever enqueued the job
/// could get wrong.
///
/// Failures are per-material and never fatal: a file that has moved
/// or been deleted leaves its hash `NULL`, which reads downstream as
/// "unknown", not as "unique". Returning `Err` for one missing file
/// would abandon the rest of the page.
pub async fn material_hash(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    if payload.get("batch").and_then(|v| v.as_bool()) == Some(true) {
        return material_hash_batch(env, payload).await;
    }
    let Some(asset) = load_target(env, payload).await? else {
        return Ok("asset gone, skipped".into());
    };
    let mut hashed = 0usize;
    let mut skipped = 0usize;
    let mut conflicts = 0usize;
    let mut mismatched = 0usize;
    for material in &asset.materials {
        // A material that already carries an answer on **every** axis
        // has been answered, and the values can only have come from a
        // previous read of the bytes: a caller's declared digest is kept
        // on the row's `_trace` and never written here, precisely so
        // that declaring one cannot satisfy this test and skip the read
        // it is meant to be checked against.
        //
        // The same rule the walk selects by, from the same function —
        // a test that read fewer columns than the walk selects on would
        // skip rows the walk keeps handing back.
        if !content_hash::needs_fingerprint(
            material.content_hash.as_deref(),
            material.content_region_hash.as_deref(),
            material.meta_hash.as_deref(),
        ) {
            continue;
        }
        match hash_material(
            env,
            &asset.id,
            material.ord,
            &material.locator,
            material.mime.as_ref(),
            DetectionOrigin::Ingest,
        )
        .await
        {
            HashOutcome::Hashed {
                conflict,
                declaration_disagreed,
            } => {
                hashed += 1;
                if conflict {
                    conflicts += 1;
                }
                if declaration_disagreed {
                    mismatched += 1;
                }
            }
            HashOutcome::Skipped => skipped += 1,
        }
    }
    // Reading the bytes is also what writes `material.meta_kv` — the
    // canonical metadata object a container carried, which for a
    // generated image is where the prompt is. That is a section of the
    // derived text, and it did not exist when the ingest-time document
    // was composed, so a hashing pass that wrote anything re-indexes.
    if hashed > 0 {
        enqueue_reindex(env, &asset.id).await?;
    }
    Ok(format!(
        "material_hash: hashed={hashed} skipped={skipped} conflicts={conflicts} \
         mismatched={mismatched}"
    ))
}

/// One page of the backfill walk. Chain-enqueues the next page while
/// pages come back full, so a library imported before this column
/// existed drains without a driver — the same shape as the index
/// backfill and the retention sweep.
async fn material_hash_batch(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    // Cursor is the composite `(asset_id, ord)` key the scan orders by.
    // Two wire shapes are accepted: the current
    // `{ "asset_id": <uuid>, "ord": <n> }` object, and the legacy bare
    // uuid string a pre-composite build may have chain-enqueued into
    // the durable queue before an upgrade. The legacy form carried
    // "strictly after this asset" semantics, which `ord: u32::MAX`
    // reproduces exactly.
    let cursor: Option<(AssetId, u32)> = match payload.get("cursor") {
        Some(serde_json::Value::String(s)) => uuid::Uuid::parse_str(s)
            .ok()
            .map(|u| (AssetId::from_uuid(u), u32::MAX)),
        Some(serde_json::Value::Object(o)) => {
            let id = o
                .get("asset_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(AssetId::from_uuid);
            let ord = o.get("ord").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            id.map(|id| (id, ord))
        }
        _ => None,
    };
    let page = env
        .deps
        .assets
        .scan_unhashed_materials(
            cursor.as_ref().map(|(id, ord)| (id, *ord)),
            MATERIAL_HASH_PAGE,
        )
        .await?;
    if page.is_empty() {
        return Ok("material_hash backfill: nothing left to hash".into());
    }
    let last = page
        .last()
        .map(|m| (m.asset_id, m.ord))
        .expect("page is non-empty");
    let full = page.len() as u32 == MATERIAL_HASH_PAGE;
    let mut hashed = 0usize;
    let mut skipped = 0usize;
    let mut conflicts = 0usize;
    let mut mismatched = 0usize;
    for item in page {
        match hash_material(
            env,
            &item.asset_id,
            item.ord,
            &item.locator,
            // The scan row carries the same `material.mime` the
            // per-asset pass reads off the entity, so one artefact
            // fingerprints identically whichever pass reaches it.
            item.mime.as_ref(),
            DetectionOrigin::Backfill,
        )
        .await
        {
            HashOutcome::Hashed {
                conflict,
                declaration_disagreed,
            } => {
                hashed += 1;
                if conflict {
                    conflicts += 1;
                }
                if declaration_disagreed {
                    mismatched += 1;
                }
            }
            HashOutcome::Skipped => skipped += 1,
        }
    }
    // Chain only on a full page. A short page is the end of the walk,
    // and re-enqueueing on it would spin forever over the materials
    // that can never be hashed (container records, dead files) — they
    // stay NULL by design, so "nothing was hashed" is not a stop
    // condition, "nothing was scanned" is.
    if full {
        env.queue
            .enqueue(
                asterism_core::domain::job::JobKind::MaterialHash,
                serde_json::json!({
                    "batch": true,
                    "cursor": { "asset_id": last.0.to_string(), "ord": last.1 },
                }),
            )
            .await?;
    }
    Ok(format!(
        "material_hash backfill page: hashed={hashed} skipped={skipped} \
         conflicts={conflicts} mismatched={mismatched} next_cursor={}#{} more={full}",
        last.0, last.1
    ))
}

/// Page size for the duplicate re-scan.
///
/// Larger than either backfill's because the work per row is a lookup,
/// not a read: every digest it compares is already a column. The page is
/// still a latency knob for the shared SQLite connection — it bounds how
/// long one job holds it.
const DUPLICATE_SCAN_PAGE: u32 = 200;

/// Re-derives duplicate conflicts from fingerprints already on the rows.
///
/// Payload: `{ "batch": true, "cursor": {"asset_id": …, "ord": …} }`.
/// One shape only — there is no per-asset variant because the per-asset
/// derivation already happens, inline, when that asset's digest is
/// written (`detect_after_hash`). This exists for the rows whose moment
/// passed; see [`JobKind::DuplicateScan`].
///
/// **Nothing here is destructive and nothing is folded.** The origin is
/// [`DetectionOrigin::Backfill`], which turns a lane's `Fold` into `Ask`,
/// and the conflict insert is `ON CONFLICT DO NOTHING` over
/// `UNIQUE (pair_lo, pair_hi, axis)` — so a pair already queued is not
/// duplicated, and a pair a person already resolved keeps its answer.
/// That is what makes the job safe to re-run at will.
///
/// [`JobKind::DuplicateScan`]: asterism_core::domain::job::JobKind::DuplicateScan
pub async fn duplicate_scan(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    // Same composite cursor the fingerprint walk carries, read the same
    // way — including the legacy bare-uuid form, so a page chained by an
    // older build survives an upgrade.
    let cursor: Option<(AssetId, u32)> = match payload.get("cursor") {
        Some(serde_json::Value::String(s)) => uuid::Uuid::parse_str(s)
            .ok()
            .map(|u| (AssetId::from_uuid(u), u32::MAX)),
        Some(serde_json::Value::Object(o)) => {
            let id = o
                .get("asset_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(AssetId::from_uuid);
            let ord = o.get("ord").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            id.map(|id| (id, ord))
        }
        _ => None,
    };
    let page = env
        .deps
        .assets
        .scan_fingerprinted_materials(
            cursor.as_ref().map(|(id, ord)| (id, *ord)),
            DUPLICATE_SCAN_PAGE,
        )
        .await?;
    if page.is_empty() {
        return Ok("duplicate_scan: nothing left to re-derive".into());
    }
    let last = page
        .last()
        .map(|m| (m.asset_id, m.ord))
        .expect("page is non-empty");
    let full = page.len() as u32 == DUPLICATE_SCAN_PAGE;
    let mut looked = 0usize;
    let mut raised = 0usize;
    for item in page {
        looked += 1;
        // Through the same swallowing wrapper the hashing pass uses: a
        // failed derivation must not fail a job whose page is otherwise
        // fine, and here there is not even a durable half to protect —
        // the next run reaches the row again.
        if detect_after_hash(
            env,
            &item.asset_id,
            item.ord,
            &item.fingerprint,
            DetectionOrigin::Backfill,
        )
        .await
        {
            raised += 1;
        }
    }
    if full {
        env.queue
            .enqueue(
                asterism_core::domain::job::JobKind::DuplicateScan,
                serde_json::json!({
                    "batch": true,
                    "cursor": { "asset_id": last.0.to_string(), "ord": last.1 },
                }),
            )
            .await?;
    }
    // `raised` counts pairs the detection *agreed* on, which includes
    // ones already on the queue — the insert is idempotent, so a second
    // run over an unchanged library reports the same number and writes
    // nothing. A count that fell to zero on the second pass would mean
    // the queue was being consumed, not that the work was done.
    Ok(format!(
        "duplicate_scan page: looked={looked} agreed={raised} \
         next_cursor={}#{} more={full}",
        last.0, last.1
    ))
}

/// Page size for the derivation walk.
///
/// The size of [`DUPLICATE_SCAN_PAGE`] and for the same reason: the work
/// per row is a parse, not a read. Nothing here opens a file — the inputs
/// are `meta_kv`, the material's mime and the rules, all of them columns,
/// which is the property the [`series`](asterism_core::domain::series)
/// module doc sells the axis on. The page is still a latency knob for
/// the shared SQLite connection: it bounds how long one job holds it.
///
/// Visible to the job module so a test can seed `PAGE + 1` pairs and
/// reach the chain branch. A test that named its own number would be
/// measuring a page size this handler does not use — and a page-size
/// parameter would be a knob production never turns, which is the shape
/// that lets the real value go untested.
pub(super) const SERIES_DERIVE_PAGE: u32 = 200;

/// Derives `material_series` keys — applies every registered
/// [`Strategy`](asterism_core::domain::series::Strategy) to a material's
/// `meta_kv` and files what each rule concluded.
///
/// Two payload shapes, the same split [`material_hash`] uses:
///
/// - `{ "asset_id": <uuid> }` — one asset's materials, enqueued by the
///   fingerprint pass at the moment it writes their `meta_kv`. Without it
///   a file imported now would carry no key until the next start.
/// - `{ "batch": true, "cursor": {"asset_id": …, "ord": …,
///   "strategy_id": …} }` — a pass over the pairs nothing has answered,
///   chain-enqueued while pages come back full.
///
/// **Every pair is answered, including the two answers that are not
/// keys.** That is the whole of why the walk ends: its predicate is "a
/// `(material, rule)` pair with no row", so a pair leaves the population
/// by acquiring one. Filing only derived keys would re-offer every JPEG
/// and every material a rule declines on every page, and the chain — which
/// re-enqueues while a page comes back full — would run for as long as
/// the process lived. See
/// [`JobKind::SeriesDerive`](asterism_core::domain::job::JobKind::SeriesDerive).
pub async fn series_derive(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    if payload.get("batch").and_then(|v| v.as_bool()) == Some(true) {
        return series_derive_batch(env, payload).await;
    }
    let Some(asset) = load_target(env, payload).await? else {
        return Ok("asset gone, skipped".into());
    };
    let rules = env.deps.series.list_strategies().await?;
    let mut tally = DerivedTally::default();
    for material in &asset.materials {
        // The same test the walk's population is defined by
        // (`meta_kv IS NOT NULL`), so the two passes agree about which
        // materials are answerable at all. A column that will not parse
        // is an empty map here for the same reason it is one there: the
        // rules then decline it, which is an answer, rather than leaving
        // the pair for a walk that would offer it again.
        if material.meta_kv.is_none() {
            continue;
        }
        let meta_kv = material.meta_fields().unwrap_or_default();
        for registered in &rules {
            // The listing carries each row's provenance beside the rule
            // (one statement over `series_strategy`, see the port); the
            // derivation reads the rule and nothing else, because
            // neither stamp nor the `system` flag is an input to a key.
            let rule = &registered.strategy;
            let key = asterism_core::domain::series::derive(rule, material.mime.as_ref(), &meta_kv);
            tally
                .record(env, &asset.id, material.ord, &rule.id, key)
                .await;
        }
    }
    Ok(format!("series_derive {}: {tally}", asset.id))
}

/// One page of the derivation walk. Chain-enqueues the next page while
/// pages come back full — the shape the fingerprint and dimension walks
/// use, and the stop condition is theirs too: "nothing was scanned" ends
/// the pass, "nothing was derived" does not.
///
/// # The page holds its `meta_kv` for as long as the page takes
///
/// A pair's metadata is read when the page is selected and its answer may
/// be written up to `SERIES_DERIVE_PAGE` writes later. If a `MaterialHash`
/// rewrites that material's `meta_kv` inside that window *and* the
/// per-asset derive it enqueues lands first, this pass overwrites the
/// fresh key with one derived from metadata that is no longer in the
/// column — and the row's existence keeps the pair out of every later
/// page, so nothing comes back for it. The precondition is a re-hash of an
/// already-derived material, which no path here is known to reach today
/// (the fingerprint walk retires a row once its columns hold answers, and
/// the per-asset pass only re-reads what a caller asked for). It is
/// recorded rather than guarded because closing it costs something real:
/// re-reading `meta_kv` at record time is a row read per pair, and scoping
/// the page to one material is the page.
async fn series_derive_batch(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    // The cursor is the pair's identity — `(asset_id, ord, strategy_id)`.
    // A malformed or absent cursor starts the pass over rather than
    // failing it: the walk is idempotent, and a pass that refused to
    // start is one that leaves the library unanswered.
    let cursor: Option<(AssetId, u32, StrategyId)> = payload
        .get("cursor")
        .and_then(|c| c.as_object())
        .and_then(|c| {
            let asset = c
                .get("asset_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(AssetId::from_uuid)?;
            let strategy = c
                .get("strategy_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(StrategyId::from_uuid)?;
            let ord = c.get("ord").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            Some((asset, ord, strategy))
        });
    let page = env
        .deps
        .series
        .scan_underived(
            cursor
                .as_ref()
                .map(|(asset, ord, strategy)| (asset, *ord, strategy)),
            SERIES_DERIVE_PAGE,
        )
        .await?;
    if page.is_empty() {
        return Ok("series_derive pass: nothing left to derive".into());
    }
    let last = page
        .last()
        .map(|pair| (pair.asset_id, pair.ord, pair.strategy.id))
        .expect("page is non-empty");
    let full = page.len() as u32 == SERIES_DERIVE_PAGE;
    let mut tally = DerivedTally::default();
    for pair in page {
        // The rule travelled with the pair, so this is the one read the
        // page paid for — no second lookup that could see a different
        // library than the one the page was selected from.
        let key = asterism_core::domain::series::derive(
            &pair.strategy,
            pair.mime.as_ref(),
            &pair.meta_kv,
        );
        tally
            .record(env, &pair.asset_id, pair.ord, &pair.strategy.id, key)
            .await;
    }
    if full {
        env.queue
            .enqueue(
                asterism_core::domain::job::JobKind::SeriesDerive,
                serde_json::json!({
                    "batch": true,
                    "cursor": {
                        "asset_id": last.0.to_string(),
                        "ord": last.1,
                        "strategy_id": last.2.to_string(),
                    },
                }),
            )
            .await?;
    }
    Ok(format!(
        "series_derive pass: {tally} next_cursor={}#{}#{} more={full}",
        last.0, last.1, last.2
    ))
}

/// What one pass made of the pairs it was handed, and the one place an
/// answer is filed.
///
/// A struct rather than three counters and an inline `record` call
/// because the two entry points must file identically: the per-asset run
/// and the walk answer the same question about the same rows, and a
/// second spelling of "what to do with the answer" is how the two would
/// come to disagree about which outcomes are worth writing — the
/// disagreement that stops the walk shrinking.
#[derive(Default)]
struct DerivedTally {
    derived: usize,
    empty: usize,
    not_applicable: usize,
    failed: usize,
}

impl DerivedTally {
    /// Files one answer and counts it.
    ///
    /// **A write that fails is logged and counted, never returned.** The
    /// alternative loses the rest of the page to one row, and worse: the
    /// walk is ordered, so a pair that can never be filed — an asset
    /// deleted between the scan and the write — would stop every later
    /// pair from ever being reached. Swallowed, the cursor steps over it
    /// and the pass continues; the pair is offered again next time, which
    /// is the recoverable direction.
    async fn record(
        &mut self,
        env: &JobEnv,
        asset_id: &AssetId,
        ord: u32,
        strategy_id: &StrategyId,
        key: SeriesKey,
    ) {
        // Counted on the way out rather than on the way in, so the
        // numbers are answers *filed*. A pair whose write failed is
        // still in the walk, and reporting it as derived would say the
        // page shrank by more than it did.
        match env
            .deps
            .series
            .record(asset_id, ord, strategy_id, &key, chrono::Utc::now())
            .await
        {
            Ok(()) => match &key {
                SeriesKey::Derived(_) => self.derived += 1,
                SeriesKey::NothingToSelect => self.empty += 1,
                SeriesKey::NotApplicable => self.not_applicable += 1,
            },
            Err(err) => {
                self.failed += 1;
                tracing::warn!(
                    event = "diag.series_derive.record_failed",
                    asset_id = %asset_id,
                    ord,
                    strategy_id = %strategy_id,
                    error = %err,
                    "a derived answer could not be filed; the pair stays in the walk"
                );
            }
        }
    }
}

impl std::fmt::Display for DerivedTally {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "derived={} empty={} not_applicable={} failed={}",
            self.derived, self.empty, self.not_applicable, self.failed
        )
    }
}

/// Page size for the embedded-text recovery walk.
///
/// The same size the hash walk uses, because the shape of the work is
/// the same: open a file, walk its bytes, write one column. It is
/// cheaper per row (no digest over the whole buffer), which makes this
/// page a conservative choice rather than a tuned one.
const MATERIAL_TEXT_PAGE: u32 = 50;

/// Recovers `material.meta_text` for the library that predates the
/// column — [`JobKind::MaterialText`](asterism_core::domain::job::JobKind::MaterialText).
///
/// Batch only. The ingest path fills this column as a side effect of
/// hashing, so the per-asset form would have no caller; what needs a
/// walk is the set that was already on disk when the column arrived.
///
/// # What each row costs, and why the set shrinks
///
/// A row leaves the set as soon as anything is written to it, and
/// something is written for every row the walk can *read* — `{}` when
/// the bytes carry no words is an answer, not a gap. The rows that stay
/// are the ones nothing looked at: a format this recovery does not
/// read, a locator with no file behind it, a file over the walk ceiling
/// or gone from disk. Those are `NULL` because that is true of them,
/// and they cost one page scan on the next startup rather than a
/// re-read.
///
/// # It re-composes what it recovers
///
/// Words on a row are not yet words in a document. A row that gained
/// text here has a search document composed before that text existed,
/// so the asset is queued for re-composition — but only when something
/// was actually found, because a walk over a library of text-free
/// pictures would otherwise enqueue one no-op job per picture.
pub async fn material_text(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    let cursor: Option<(AssetId, u32)> = match payload.get("cursor") {
        Some(serde_json::Value::Object(o)) => {
            let id = o
                .get("asset_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(AssetId::from_uuid);
            let ord = o.get("ord").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            id.map(|id| (id, ord))
        }
        _ => None,
    };
    let page = env
        .deps
        .assets
        .scan_unrecovered_text(
            cursor.as_ref().map(|(id, ord)| (id, *ord)),
            MATERIAL_TEXT_PAGE,
        )
        .await?;
    if page.is_empty() {
        return Ok("material_text backfill: nothing left to recover".into());
    }
    let last = page
        .last()
        .map(|m| (m.asset_id, m.ord))
        .expect("page is non-empty");
    let full = page.len() as u32 == MATERIAL_TEXT_PAGE;

    let mut recovered = 0usize;
    let mut empty = 0usize;
    let mut skipped_format = 0usize;
    let mut unreadable = 0usize;
    // One re-index per asset, not per material: an asset's RAW and its
    // JPEG are two rows of this walk and one document.
    let mut touched: std::collections::HashSet<AssetId> = std::collections::HashSet::new();

    for row in page {
        // Asked before anything is opened — this is what keeps the walk
        // off every video and every text note in the library.
        if !asterism_core::domain::embedded_text::walks_format(row.mime.as_ref()) {
            skipped_format += 1;
            continue;
        }
        let Some(path) = row.locator.local_path() else {
            // A record inside a container or a remote address: there are
            // no bytes here to look in, and saying "read, and empty"
            // would retire a row nothing read.
            unreadable += 1;
            continue;
        };
        match crate::fingerprint::recover_embedded_text(
            &path.to_string_lossy(),
            row.mime.as_ref(),
            crate::fingerprint::MAX_CONTENT_WALK_BYTES,
        ) {
            Ok(Some(rendered)) => {
                let carried_words = rendered != "{}";
                env.deps
                    .assets
                    .set_material_embedded_text(&row.asset_id, row.ord, Some(&rendered))
                    .await?;
                if carried_words {
                    recovered += 1;
                    touched.insert(row.asset_id);
                } else {
                    empty += 1;
                }
            }
            // Over the ceiling: nobody looked, so the row keeps waiting.
            Ok(None) => unreadable += 1,
            Err(err) => {
                unreadable += 1;
                tracing::warn!(
                    event = "diag.material_text.unreadable",
                    asset_id = %row.asset_id,
                    ord = %row.ord,
                    error = %err,
                    "left with no recovered text"
                );
            }
        }
    }

    for asset_id in &touched {
        enqueue_reindex(env, asset_id).await?;
    }

    // Chain only on a full page, for the reason the hash walk gives: a
    // short page is the end of the scan, and the rows this pass cannot
    // answer stay `NULL` by design — "nothing was recovered" is not a
    // stop condition, "nothing was scanned" is.
    if full {
        env.queue
            .enqueue(
                asterism_core::domain::job::JobKind::MaterialText,
                serde_json::json!({
                    "batch": true,
                    "cursor": { "asset_id": last.0.to_string(), "ord": last.1 },
                }),
            )
            .await?;
    }
    Ok(format!(
        "material_text backfill page: recovered={recovered} empty={empty} \
         skipped_format={skipped_format} unreadable={unreadable} \
         reindexed={} next_cursor={}#{} more={full}",
        touched.len(),
        last.0,
        last.1
    ))
}

/// Page size for the dimension backfill.
///
/// Smaller than [`MATERIAL_HASH_PAGE`] because the work per row is
/// smaller: a header read stops at the first few KB of a still, where
/// fingerprinting walks every byte. The page is a latency knob for the
/// shared SQLite connection either way — it bounds how long one job
/// holds it, not how much total work the walk does.
const ASSET_DIMS_PAGE: u32 = 100;

/// Measures `asset.width_px` / `height_px`.
///
/// Two payload shapes, the same split [`material_hash`] uses:
///
/// - `{ "asset_id": <uuid> }` — **one asset, because somebody asked.**
///   A person who replaced the file behind a card; an agent over HTTP
///   naming the rows it wants redone. No scope and no dedup: the request
///   *is* the reason, and it overwrites, because a caller who asks for a
///   re-measure knows something the stored value does not.
/// - `{ "batch": true, "scope": <scope?>, "cursor": <uuid?> }` — a pass
///   over the table, chain-enqueued while pages come back full. `scope`
///   defaults to `unlooked`, which is the startup seed's reading.
///
/// **`scope` rides the payload through the chain.** A pass that read it
/// once and let the next page fall back to the default would apply the
/// caller's intent to the first hundred rows and silently revert for the
/// rest — a force-walk that appears to run and mostly does not.
///
/// Off the ingest path for the reason the fingerprint walk is: it opens
/// files. An asset arriving today is measured by the importer that
/// already holds its bytes, which is both cheaper and better evidence.
pub async fn asset_dims(env: &JobEnv, payload: &serde_json::Value) -> Result<String, DomainError> {
    if payload.get("batch").and_then(|v| v.as_bool()) == Some(true) {
        return asset_dims_batch(env, payload).await;
    }
    let Some(asset_id) = payload
        .get("asset_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(AssetId::from_uuid)
    else {
        return Ok("asset_dims: payload names neither an asset nor a batch".into());
    };
    let Some(asset) = env.deps.assets.find(&asset_id).await? else {
        return Ok(format!("asset_dims: {asset_id} is gone"));
    };
    let outcome = probe_dims(&asset.source.locator).await;
    // `Overwrite`: this run exists because a caller asked for it, and
    // their asking is newer information than whatever is stored.
    env.deps
        .assets
        .record_dims_probe(
            &asset_id,
            outcome,
            DimsWritePolicy::Overwrite,
            chrono::Utc::now(),
        )
        .await?;
    Ok(match outcome {
        DimsProbe::Measured(w, h) => format!("asset_dims {asset_id}: {w}x{h}"),
        DimsProbe::NothingToMeasure => format!("asset_dims {asset_id}: no dimensions in the bytes"),
        // Named apart in the message because it is the one outcome that
        // wrote nothing and will be retried.
        DimsProbe::Unreadable => format!("asset_dims {asset_id}: unreadable, left for a retry"),
    })
}

async fn asset_dims_batch(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    // Absent means the startup seed, which predates the knob. A slug
    // that is present but unknown is refused rather than defaulted —
    // through the same parse the enqueueing verb used, so the two cannot
    // disagree about what a scope is.
    let scope = match payload.get("scope").and_then(|v| v.as_str()) {
        None => DimsScope::Unlooked,
        Some(slug) => DimsScope::parse(slug)?,
    };
    // `All` is the only scope that means to replace answers; the other
    // two are filling gaps and must not step on an ingest measurement
    // that landed since the scan.
    let policy = match scope {
        DimsScope::All => DimsWritePolicy::Overwrite,
        DimsScope::Unlooked | DimsScope::Unmeasured => DimsWritePolicy::FillOnly,
    };
    let cursor: Option<AssetId> = payload
        .get("cursor")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(AssetId::from_uuid);
    let page = env
        .deps
        .assets
        .scan_dims_candidates(scope, cursor.as_ref(), ASSET_DIMS_PAGE)
        .await?;
    if page.is_empty() {
        return Ok("asset_dims pass: nothing left to measure".into());
    }
    let last = page.last().map(|a| a.asset_id).expect("page is non-empty");
    let full = page.len() as u32 == ASSET_DIMS_PAGE;
    let mut measured = 0usize;
    let mut nothing = 0usize;
    let mut unreadable = 0usize;
    for item in page {
        let outcome = probe_dims(&item.locator).await;
        match outcome {
            DimsProbe::Measured(..) => measured += 1,
            DimsProbe::NothingToMeasure => nothing += 1,
            DimsProbe::Unreadable => unreadable += 1,
        }
        env.deps
            .assets
            .record_dims_probe(&item.asset_id, outcome, policy, chrono::Utc::now())
            .await?;
    }
    // Chain only on a full page, same stop condition the fingerprint
    // walk uses: "nothing was scanned" ends the pass, "nothing was
    // measured" does not. The scope travels with the cursor.
    if full {
        env.queue
            .enqueue(
                asterism_core::domain::job::JobKind::AssetDims,
                serde_json::json!({
                    "batch": true,
                    "scope": scope.as_str(),
                    "cursor": last.to_string(),
                }),
            )
            .await?;
    }
    Ok(format!(
        "asset_dims {} pass: measured={measured} nothing={nothing} \
         unreadable={unreadable} next_cursor={last} more={full}",
        scope.as_str()
    ))
}

/// Reads one artefact's coded dimensions, or `None` when there are none
/// to read.
///
/// Three ways to get `None`, and the job treats them alike because the
/// column does: no local bytes (a container record, a remote locator),
/// bytes that could not be read, and bytes no probe recognises (a text
/// note, an AVI).
///
/// **Both branches go through `asterism-media-probe`,** which is what
/// the importers measure through — that is the reason the crate was
/// split out of them.
///
/// # The path, not the bytes
///
/// The `_at` entry points, deliberately. An importer holds the payload
/// already (its `RawItem` carries it), so the slice forms cost it
/// nothing; this job holds a path, and `std::fs::read` here would make
/// the peak allocation the size of the largest artefact in the library —
/// gigabytes, to answer a question the first kilobyte usually settles.
/// The file forms read incrementally instead: `imagesize` and
/// `image::ImageReader` stop at the header, `kamadak-exif` takes 4 KiB
/// and seeks, `matroska` seeks.
///
/// One case still reads a lot and cannot be made not to:
/// `mp4parse::read_mp4` is bounded on `Read` with no `Seek`, so an MP4
/// whose `moov` sits behind its `mdat` — the default layout of most
/// encoders — is walked through to reach the dimensions. Streamed
/// rather than resident, which is the part that was actually in this
/// function's gift.
///
/// # Three outcomes, and the middle one is the reason for the enum
///
/// - **No local bytes** — a container record, a remote locator. Nothing
///   will ever be readable there, so it is `NothingToMeasure`.
/// - **Bytes read, no dimensions in them** — a text note, an AVI.
///   Also `NothingToMeasure`.
/// - **Bytes not readable right now** — an unmounted volume, a file
///   that has moved, a permission that will be back. `Unreadable`,
///   which writes nothing and leaves the row for a later pass.
///
/// The first two were collapsed with the third in the shape this
/// replaced, so a library on an external disk measured once while the
/// disk was out would have been marked permanently unmeasurable.
async fn probe_dims(locator: &SourceLocator) -> DimsProbe {
    // A locator with no local path is answered, not deferred: there is
    // no future in which bytes appear at a place that names none.
    let Some(path) = locator.local_path().map(|p| p.to_path_buf()) else {
        return DimsProbe::NothingToMeasure;
    };
    // Read once, here, so "could the file be opened" is a question this
    // function answers rather than one the probes swallow into `None`.
    // The probes below re-open it — they read incrementally and a
    // pre-read buffer would be the resident copy this path exists to
    // avoid — so this is a liveness check, not the read itself.
    if let Err(err) = tokio::fs::metadata(&path).await {
        tracing::warn!(
            event = "diag.asset_dims.unreadable",
            locator = %locator.to_display(),
            error = %err,
            "asset_dims left the row for a later pass"
        );
        return DimsProbe::Unreadable;
    }
    // Blocking file I/O, off the async worker. Both probes are tried,
    // still image first; neither is expensive on a container it does not
    // recognise — each rejects at the magic bytes — so asking both means
    // the answer does not depend on a file extension being honest.
    let probed = tokio::task::spawn_blocking(move || {
        asterism_media_probe::coded_dims_at(&path)
            .or_else(|| asterism_media_probe::probe_at(&path).and_then(|p| p.dims))
    })
    .await;
    match probed {
        Ok(Some((w, h))) => DimsProbe::Measured(w, h),
        Ok(None) => DimsProbe::NothingToMeasure,
        // The blocking task died rather than answered. Nothing was
        // learned about the bytes, so nothing is recorded about them.
        Err(join) => {
            tracing::warn!(
                event = "diag.asset_dims.join_failed",
                locator = %locator.to_display(),
                error = %join,
                "asset_dims probe did not complete"
            );
            DimsProbe::Unreadable
        }
    }
}

/// Page size for one `chapter_scan` backfill pass.
///
/// Sized like the fingerprint walk's rather than the duplicate re-scan's,
/// because the unit of work is the same kind of thing: an external
/// process against a file on disk. Reading `-f ffmetadata` costs a demux
/// of the container header rather than a pass over its frames, so a page
/// is cheaper than a page of hashing — but it is still fifty spawns, and
/// the page exists to hand the worker back, not to drain the backlog.
const CHAPTER_SCAN_PAGE: u32 = 50;

/// What one material's chapter reading came to.
enum ChapterOutcome {
    /// The imported band was written. `sections` is what landed in it,
    /// `refused` how many the file declared that could not be
    /// represented (see `chapter_ffmetadata`).
    ///
    /// **Zero sections is a normal `Filed`**, and filing it is what
    /// takes the material out of the backfill walk — see
    /// [`JobKind::ChapterScan`](asterism_core::domain::job::JobKind::ChapterScan)
    /// on the band being the stamp.
    Filed { sections: usize, refused: usize },
    /// The material has no playback timeline, so there is nothing a
    /// chapter could divide. No band, and none wanted.
    NotTimed,
    /// Nothing was learned about the file. No band, so the material
    /// stays in the walk for a later pass.
    Unreadable,
}

/// Reads one material's declared chapter list and files it.
///
/// Eligibility is re-asked here through
/// [`MimeType::carries_chapters`] rather than trusted from the caller:
/// the per-asset route walks an entity and the backfill route walks a
/// SQL `LIKE`, and the handler is where the two have to mean the same
/// thing.
///
/// # A locator with no local bytes files an empty band
///
/// The same judgement `probe_dims` records for the same shape of row: a
/// container record or a remote locator names no place bytes will ever
/// appear, so "nothing to read" is a permanent answer rather than a
/// deferred one, and filing it is what keeps the walk from re-offering
/// the row on every pass. It is the one case where an empty band means
/// "there was nothing to read" instead of "the file declares nothing",
/// and the two are indistinguishable to a reader — which is acceptable
/// because both mean the same thing to a surface: no chapters.
async fn scan_material_chapters(
    env: &JobEnv,
    asset_id: &AssetId,
    ord: u32,
    locator: &SourceLocator,
    mime: Option<&MimeType>,
) -> ChapterOutcome {
    if !mime.is_some_and(MimeType::carries_chapters) {
        return ChapterOutcome::NotTimed;
    }
    let reading = match locator.local_path().map(|p| p.to_path_buf()) {
        None => crate::jobs::chapter_ffmetadata::ChapterReading::default(),
        Some(path) => {
            // Blocking process spawn and pipe read, off the async
            // worker — the same shape the frame grab beside it uses.
            let probe = tokio::task::spawn_blocking(move || {
                crate::jobs::chapter_ffmetadata::read_chapters(&path.to_string_lossy())
            })
            .await;
            match probe {
                Ok(crate::jobs::chapter_ffmetadata::ChapterProbe::Read(reading)) => reading,
                Ok(crate::jobs::chapter_ffmetadata::ChapterProbe::Unreadable(why)) => {
                    tracing::warn!(
                        event = "diag.chapter_scan.unreadable",
                        locator = %locator.to_display(),
                        detail = %why,
                        "chapter_scan left the material for a later pass"
                    );
                    return ChapterOutcome::Unreadable;
                }
                // The blocking task died rather than answered, so
                // nothing is known about the file and nothing is
                // recorded about it.
                Err(join) => {
                    tracing::warn!(
                        event = "diag.chapter_scan.join_failed",
                        locator = %locator.to_display(),
                        error = %join,
                        "chapter_scan probe did not complete"
                    );
                    return ChapterOutcome::Unreadable;
                }
            }
        }
    };
    for why in &reading.refused {
        tracing::info!(
            event = "diag.chapter_scan.section_refused",
            locator = %locator.to_display(),
            detail = %why,
            "a declared section could not be represented on the timeline"
        );
    }
    // The single door into an imported band (`chapter_intake`): it
    // resolves the layer, replaces the contents atomically, and leaves
    // a person's own bands alone. A failure here writes nothing, which
    // is why it reports `Unreadable` — no band means the material is
    // still in the walk, so the page that failed is retried rather than
    // silently recorded as chapterless.
    match asterism_core::application_support::replace_imported_chapters(
        &env.deps.material_layers,
        &env.deps.chapter_marks,
        asset_id,
        ord,
        &reading.chapters,
    )
    .await
    {
        Ok(_) => ChapterOutcome::Filed {
            sections: reading.chapters.len(),
            refused: reading.refused.len(),
        },
        Err(err) => {
            tracing::warn!(
                event = "diag.chapter_scan.write_failed",
                locator = %locator.to_display(),
                error = %err,
                "the imported chapter band was not written"
            );
            ChapterOutcome::Unreadable
        }
    }
}

/// Running totals for one `chapter_scan` run, in the shape its message
/// is built from.
#[derive(Default)]
struct ChapterTally {
    filed: usize,
    sections: usize,
    refused: usize,
    not_timed: usize,
    unreadable: usize,
}

impl ChapterTally {
    fn absorb(&mut self, outcome: ChapterOutcome) {
        match outcome {
            ChapterOutcome::Filed { sections, refused } => {
                self.filed += 1;
                self.sections += sections;
                self.refused += refused;
            }
            ChapterOutcome::NotTimed => self.not_timed += 1,
            ChapterOutcome::Unreadable => self.unreadable += 1,
        }
    }

    fn report(&self) -> String {
        format!(
            "filed={} sections={} refused={} not_timed={} unreadable={}",
            self.filed, self.sections, self.refused, self.not_timed, self.unreadable
        )
    }
}

/// Reads a container's own chapter list into its imported structure
/// band.
///
/// Payload is either `{ "asset_id": <uuid> }` (one asset, from the
/// ingest fan-out) or `{ "batch": true, "cursor": {…} }` (the walk over
/// materials no reading has reached yet).
///
/// Failures are per-material and never fatal, for the reason the
/// fingerprint job gives: a file that has moved leaves its band
/// unwritten, which reads downstream as "not scanned yet" rather than
/// as "declares nothing", and returning `Err` for one of them would
/// abandon the rest of the page.
pub async fn chapter_scan(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    if payload.get("batch").and_then(|v| v.as_bool()) == Some(true) {
        return chapter_scan_batch(env, payload).await;
    }
    let Some(asset) = load_target(env, payload).await? else {
        return Ok("asset gone, skipped".into());
    };
    let mut tally = ChapterTally::default();
    for material in &asset.materials {
        tally.absorb(
            scan_material_chapters(
                env,
                &asset.id,
                material.ord,
                &material.locator,
                material.mime.as_ref(),
            )
            .await,
        );
    }
    Ok(format!("chapter_scan: {}", tally.report()))
}

/// One page of the walk over materials that have no imported structure
/// band yet.
///
/// Chain-enqueues the next page while pages come back full, the same
/// shape as the fingerprint and dimension walks — and it stops for good
/// rather than merely stopping early, because a completed reading always
/// leaves the band its predicate selects on the absence of.
async fn chapter_scan_batch(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    // The composite `(asset_id, ord)` key the scan orders by. Only the
    // object shape is accepted: unlike the fingerprint walk, this job
    // has never shipped a bare-uuid cursor, so there is no durable queue
    // row anywhere carrying one.
    let cursor: Option<(AssetId, u32)> = match payload.get("cursor") {
        Some(serde_json::Value::Object(o)) => {
            let id = o
                .get("asset_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(AssetId::from_uuid);
            let ord = o.get("ord").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            id.map(|id| (id, ord))
        }
        _ => None,
    };
    let page = env
        .deps
        .assets
        .scan_chapter_scan_candidates(
            cursor.as_ref().map(|(id, ord)| (id, *ord)),
            CHAPTER_SCAN_PAGE,
        )
        .await?;
    if page.is_empty() {
        return Ok("chapter_scan pass: nothing left to read".into());
    }
    let last = page
        .last()
        .map(|m| (m.asset_id, m.ord))
        .expect("page is non-empty");
    let full = page.len() as u32 == CHAPTER_SCAN_PAGE;
    let mut tally = ChapterTally::default();
    for item in page {
        tally.absorb(
            scan_material_chapters(
                env,
                &item.asset_id,
                item.ord,
                &item.locator,
                item.mime.as_ref(),
            )
            .await,
        );
    }
    // Chain only on a full page. "Nothing was filed" is not a stop
    // condition — a page of unreadable files would end the walk while
    // leaving every one of them unanswered — but "nothing was scanned"
    // is, and unlike the hashing walk this one really does empty: the
    // rows that could be read leave the predicate on the way past.
    if full {
        env.queue
            .enqueue(
                asterism_core::domain::job::JobKind::ChapterScan,
                serde_json::json!({
                    "batch": true,
                    "cursor": { "asset_id": last.0.to_string(), "ord": last.1 },
                }),
            )
            .await?;
    }
    Ok(format!(
        "chapter_scan pass: {} next_cursor={}#{} more={full}",
        tally.report(),
        last.0,
        last.1
    ))
}

/// Whether one material ended up with a fingerprint.
enum HashOutcome {
    Hashed {
        /// Whether the digest turned out to be held by another asset —
        /// counted separately from the write so the job's message says
        /// how much of the page was news.
        conflict: bool,
        /// Whether the registering caller had declared a digest and the
        /// bytes disagreed with it.
        ///
        /// Counted into the job's own return message, not only logged:
        /// that string is stored on the job row, so a person asking
        /// "did anything come out of the last import" gets the answer
        /// from the queue rather than from having had the log open at
        /// the time.
        declaration_disagreed: bool,
    },
    Skipped,
}

/// Reads one material's bytes, records the digest, and asks what the
/// digest means. Every failure mode — a locator with no bytes of its
/// own, a file that has moved, an unreadable chunk — logs and returns
/// `Skipped`.
async fn hash_material(
    env: &JobEnv,
    asset_id: &AssetId,
    ord: u32,
    locator: &SourceLocator,
    mime: Option<&MimeType>,
    origin: DetectionOrigin,
) -> HashOutcome {
    // The one question, and it returns the path rather than a `bool`.
    // The predicate this replaced answered `true` for every `file://`
    // locator and then handed the *spelling* to `File::open`, which is
    // not a path: the open failed, no marker was written, and the row
    // came back on the next backfill pass and the one after that. Here
    // `file:///pics/a.png` has already become the path it names, and
    // `file://pics/a.png` — rootless, openable by nobody — takes the
    // marker branch below and leaves the walk.
    let Some(path) = locator.local_path() else {
        // Record the answer instead of leaving the row NULL. "There
        // are no bytes to read here" is a permanent fact about a
        // container record or a remote locator, and a NULL would put
        // the row back in front of every future backfill pass — a walk
        // that never shrinks, and a "still fingerprinting" notice that
        // never clears.
        //
        // Every column takes the same marker: the statement is about
        // the locator, so it is equally true on every axis, and
        // answering one would leave the row in the walk for the others.
        // `meta_kv` stays empty, because there is no object — writing
        // `{}` would say a container was read and carried nothing — and
        // `meta_raw` for the same reason one layer down: there is no
        // container to have carried bytes.
        let _ = env
            .deps
            .assets
            .set_material_fingerprint(
                asset_id,
                ord,
                &MaterialFingerprint {
                    file: UNHASHABLE.to_string(),
                    content: UNHASHABLE.to_string(),
                    meta: UNHASHABLE.to_string(),
                    // Nothing was read, so nobody has looked: `NULL`
                    // rather than the `{}` that would retire the row
                    // from a later pass.
                    meta_text: None,
                    meta_kv: None,
                    meta_raw: None,
                },
            )
            .await;
        return HashOutcome::Skipped;
    };
    // The path the type gives, not the string the column held.
    let path_str = path.to_string_lossy().into_owned();
    let claimed = mime.cloned();
    let read = tokio::task::spawn_blocking(move || {
        hash_artefact(&path_str, claimed.as_ref(), MAX_CONTENT_WALK_BYTES)
    })
    .await;
    let fingerprint = match read {
        Ok(Ok(fingerprint)) => fingerprint,
        Ok(Err(err)) => {
            tracing::warn!(
                event = "diag.material_hash.unreadable",
                locator = %locator.to_display(),
                error = %err,
                "material_hash skipped"
            );
            return HashOutcome::Skipped;
        }
        Err(join) => {
            tracing::warn!(
                event = "diag.material_hash.join_failed",
                locator = %locator.to_display(),
                error = %join,
                "material_hash worker join failed"
            );
            return HashOutcome::Skipped;
        }
    };
    match env
        .deps
        .assets
        .set_material_fingerprint(asset_id, ord, &fingerprint)
        .await
    {
        Ok(()) => {
            let outcome = HashOutcome::Hashed {
                declaration_disagreed: check_declaration(env, asset_id, ord, &fingerprint).await,
                // Both axes, from the one read that produced both. The
                // detector walks them strongest first and stops at the
                // first agreement, so a byte-identical pair is reported
                // once — on `Artefact` — rather than once per axis.
                conflict: detect_after_hash(env, asset_id, ord, &fingerprint, origin).await,
            };
            derive_series_after_hash(env, asset_id, &fingerprint).await;
            stamp_after_hash(env, asset_id, &fingerprint).await;
            outcome
        }
        Err(err) => {
            tracing::warn!(
                event = "diag.material_hash.write_failed",
                locator = %locator.to_display(),
                error = %err,
                "material_hash write failed"
            );
            HashOutcome::Skipped
        }
    }
}

/// Asks for the series keys of a material whose `meta_kv` has just
/// landed, and **swallows a failed enqueue**.
///
/// Only when the fingerprint carries a metadata object: that is the
/// walk's whole population (`meta_kv IS NOT NULL`), so a marker or a
/// container with nothing to read has no pair for any rule to answer and
/// the job would find nothing to do.
///
/// The enqueue is per *asset* while this function is reached per
/// *material*, so an asset with two originals asks twice and the handler
/// answers for both each time. Idempotent — `record` replaces the row for
/// a `(material, rule)` pair — so the duplicate costs a second derivation
/// off rows already loaded, and the alternative (a per-material payload)
/// would be a second shape for a job the walk already addresses by asset.
///
/// Swallowed for the reason [`detect_after_hash`] is: the digest and the
/// `meta_kv` beside it are committed, the fingerprint walk will not offer
/// this row again, and a lost enqueue costs a key that arrives at the
/// next start rather than an observation thrown away. It is said out loud
/// instead.
///
/// **Both origins.** The backfill walk enqueues one of these per
/// fingerprinted material carrying metadata, which is a queue row per
/// image of a library that has never been hashed — and it is what makes
/// the axis converge without a restart, because the derivation walk
/// enqueued at the same startup may well drain before the hashing walk
/// writes the `meta_kv` it would have read.
async fn derive_series_after_hash(
    env: &JobEnv,
    asset_id: &AssetId,
    fingerprint: &MaterialFingerprint,
) {
    if fingerprint.meta_kv.is_none() {
        return;
    }
    if let Err(err) = env
        .queue
        .enqueue(
            asterism_core::domain::job::JobKind::SeriesDerive,
            serde_json::json!({ "asset_id": asset_id.to_string() }),
        )
        .await
    {
        tracing::warn!(
            event = "diag.series_derive.enqueue_failed",
            asset_id = %asset_id,
            error = %err,
            "the metadata landed but nothing was asked to derive its series keys"
        );
    }
}

/// Whether a dispatch in this library produced the artefact `extra`
/// belongs to.
///
/// **This is the whole guard between the stamping path and the user's
/// own files.** Stamping rewrites bytes; doing it to an artefact a
/// dispatch made is the feature, and doing it to one somebody imported
/// is this application editing a file it was only ever asked to index.
/// A pure function over the value so the rule can be pinned without a
/// database, a queue or a file.
///
/// The marker is the trace `reify_one` writes and nothing else does.
/// `_derived` — where the merge puts an exporter's own non-object
/// `extra` — deliberately does not count: it says an exporter had
/// something to say, not that this library made the file.
fn produced_by_dispatch(extra: &serde_json::Value) -> bool {
    extra
        .get(asterism_core::domain::dispatch::DISPATCH_TRACE_KEY)
        .is_some_and(|trace| trace.is_object())
}

/// The run that produced an artefact, for the manifest's own
/// `dispatch_id`.
///
/// `None` when the trace is absent or does not name one, which is the
/// same answer a re-apply months later gives: the field is omitted
/// rather than filled with something that is not a dispatch id.
fn dispatch_id_of(extra: &serde_json::Value) -> Option<String> {
    extra
        .get(asterism_core::domain::dispatch::DISPATCH_TRACE_KEY)?
        .get("dispatch_id")?
        .as_str()
        .map(str::to_string)
}

/// Writes the AI disclosure into a file this library produced.
///
/// Enqueued by [`stamp_after_hash`] once the artefact's metadata has
/// landed — see there for why this cannot happen when the export
/// finishes, and why it is confined to artefacts a dispatch made.
///
/// # Why a failure here is not a failed job
///
/// A stamp that does not land leaves an artefact that exists and is not
/// marked. The file is fine; what is missing is a statement about it,
/// and the statement can be made again — the disclosure is derived from
/// stored rows, so re-running this job re-derives it. Failing the job
/// would put a retry loop around a file rewrite and would put an export
/// that produced exactly what it was asked for into a red state over
/// metadata.
///
/// The two halves of a disclosure fail independently and the outcome
/// says which landed, so a partial result is reported as one rather
/// than collapsed into success or failure.
pub async fn disclosure_stamp(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    let Some(service) = env.deps.disclosure.get() else {
        return Ok("no writer configured, skipped".into());
    };
    let Some(asset) = load_target(env, payload).await? else {
        return Ok("asset gone, skipped".into());
    };
    // The run that made it, for the manifest's own `dispatch_id` — the
    // field that has been absent from every manifest written so far
    // because nothing had one to pass.
    let dispatch_id = dispatch_id_of(&asset.extra);
    let Some(path) = asset.source.locator.local_path() else {
        // A container record or a remote locator has no file to write
        // into. An answer, not a failure — the same reading the hashing
        // walk gives the same locator.
        return Ok("no local file, skipped".into());
    };

    match service
        .apply_to(&asset.id, path, dispatch_id.as_deref())
        .await
    {
        Ok(outcome) => {
            let failures = outcome.failures();
            if !failures.is_empty() {
                tracing::warn!(
                    event = "diag.disclosure_stamp.partial",
                    asset_id = %asset.id,
                    discloses = outcome.discloses(),
                    failures = ?failures,
                    "part of the disclosure did not land"
                );
            }
            note_disclosure(env, &asset.id, &outcome).await;
            Ok(format!(
                "disclosed={} failures={}",
                outcome.discloses(),
                failures.len()
            ))
        }
        Err(err) => {
            tracing::warn!(
                event = "diag.disclosure_stamp.failed",
                asset_id = %asset.id,
                error = %err,
                "the artefact exists and carries no mark"
            );
            Ok(format!("not stamped: {err}"))
        }
    }
}

/// Records what became of an artefact's disclosure on the row.
///
/// # Why the row and not only the log
///
/// The database is the source of truth for what this library holds, and
/// until this existed the answer to "which artefacts carry a mark" was
/// in a log line. A mark is not visible in the row it belongs to — it
/// is in the file's bytes, and the file can come back from a downstream
/// conversion with it gone — so without a note there is nothing to
/// re-apply *from* and nothing to ask.
///
/// Through the narrow write, not a save: the caller is a worker, and a
/// read-modify-save of the whole entity would discard a rating or a tag
/// applied while the job ran.
///
/// # Why a failure here changes nothing
///
/// The mark is already in the file, or already not. Failing the job
/// over the bookkeeping would retry a file rewrite to fix a row, which
/// is the larger operation of the two. It is said out loud instead.
async fn note_disclosure(
    env: &JobEnv,
    asset_id: &AssetId,
    outcome: &asterism_core::domain::disclosure::Stamped,
) {
    let mut note = outcome.to_note();
    // When, added here because the outcome type is produced in places
    // that write nothing down and has no business holding a clock.
    note["at"] = serde_json::json!(chrono::Utc::now().timestamp_millis());
    match env
        .deps
        .assets
        .note_trace_field(
            asset_id,
            asterism_core::domain::disclosure::DISCLOSURE_NOTE_KEY,
            note,
        )
        .await
    {
        Ok(true) => {}
        Ok(false) => tracing::warn!(
            event = "diag.disclosure_stamp.note_skipped",
            asset_id = %asset_id,
            "extra column could not carry the disclosure note"
        ),
        Err(err) => tracing::warn!(
            event = "diag.disclosure_stamp.note_failed",
            asset_id = %asset_id,
            error = %err,
            "the file was stamped and the row does not say so"
        ),
    }
}

/// Asks for the AI disclosure of an artefact **this library produced**,
/// once its metadata has landed, and swallows a failed enqueue.
///
/// # Why here and not at export time
///
/// The disclosure is derived from stored container metadata, and a
/// dispatch mints its outputs without any: `reify` builds the material
/// from the exporter's string and enqueues the hashing that fills
/// `meta_kv` in. A stamp taken when the export finished would read an
/// empty evidence set, establish nothing, and write nothing — it would
/// succeed on every file and mark none of them. This is the first
/// moment there is anything to disclose, which is why the order is a
/// chain rather than a hope.
///
/// # Why only what this library produced
///
/// Stamping rewrites the file. For an artefact a dispatch made, that is
/// the feature; for one the user imported, it is this application
/// editing somebody's original because it happened to walk past it. The
/// dispatch trace
/// ([`DISPATCH_TRACE_KEY`](asterism_core::domain::dispatch::DISPATCH_TRACE_KEY))
/// is what separates the two, and it costs the one read that answers
/// it — paid here rather than in the handler so that a library-wide
/// backfill does not put a job on the queue per asset to have it skip.
///
/// Swallowed for the reason [`derive_series_after_hash`] gives: the
/// fingerprint is committed and the walk will not offer this row again,
/// so a lost enqueue costs a mark that arrives at the next start rather
/// than an observation thrown away. It is said out loud instead.
async fn stamp_after_hash(env: &JobEnv, asset_id: &AssetId, fingerprint: &MaterialFingerprint) {
    if fingerprint.meta_kv.is_none() {
        return;
    }
    // No writer configured is not a failure and not a warning: it is
    // the state a build that has not asked for stamping is in.
    if env.deps.disclosure.get().is_none() {
        return;
    }
    let produced_here = matches!(
        env.deps.assets.find(asset_id).await,
        Ok(Some(asset)) if produced_by_dispatch(&asset.extra)
    );
    if !produced_here {
        return;
    }
    if let Err(err) = env
        .queue
        .enqueue(
            asterism_core::domain::job::JobKind::DisclosureStamp,
            serde_json::json!({ "asset_id": asset_id.to_string() }),
        )
        .await
    {
        tracing::warn!(
            event = "diag.disclosure_stamp.enqueue_failed",
            asset_id = %asset_id,
            error = %err,
            "the metadata landed but nothing was asked to write the disclosure"
        );
    }
}

/// Answers the digest a caller declared at registration against the one
/// the bytes just produced **on the axis the claim named**, and records
/// the verdict on the row.
///
/// The axis comes off the declared value's own tag
/// ([`content_hash::axis_of`]), which is the whole reason the tag is
/// part of the value: a `cr1-sha256:` claim compared against the file
/// digest would report a mismatch on every well-formed declaration,
/// and the person reading the alarm would go and look at a file that
/// is fine.
///
/// A claim whose axis this pass did not measure — a content-axis
/// declaration on an artefact that fell to a marker — is **not**
/// checked and gets no verdict: see [`declared_axis_value`] for why a
/// comparison there would be a false alarm rather than a finding.
///
/// Returns whether the two disagreed. **A disagreement changes nothing
/// else**: the recomputed value stays on the material, the asset is not
/// deleted, quarantined or held back, and detection runs on the real
/// digest exactly as it would have. What the caller said is a statement
/// about the file; the file is the file.
///
/// # Why the read happens here
///
/// This function sits at the one place both passes converge. The
/// per-asset run has the entity in hand and the backfill walk has only
/// a scan row, so reading the claim in each caller would mean two
/// sources for one fact — the shape [`DetectionOrigin`] exists to avoid
/// on the neighbouring question. **The backfill needs it just as much:**
/// a declared registration reaches that walk whenever its own hash job
/// did not finish the work — the ingest-time enqueue is best-effort and
/// its failure is swallowed, an unreadable or moved file leaves the
/// columns NULL, and a worker killed mid-page leaves them NULL too. The
/// walk selects the rows whose fingerprint columns hold no answer
/// (`content_hash::needs_fingerprint`, over both axes) and knows nothing
/// about when the row was registered, so it picks those up with the
/// claim still on them.
///
/// The cost is one row read by primary key, immediately after reading
/// the whole file off disk. Carrying the claim down from the two
/// callers instead would save it and reintroduce the duplicated fact.
///
/// # Only the primary material
///
/// The declaration travels on `AddAssetCommand`, which names one
/// artefact and mints `ord = 0` from it. A secondary original (the RAW
/// beside the JPEG, when that wave lands) is a different set of bytes
/// that this command never spoke about, so checking it against the
/// claim would report a mismatch about a file nobody described.
///
/// # Every failure here is swallowed
///
/// Same rule as [`detect_after_hash`] below and for the same reason:
/// the digest is committed, and the backfill finds work by asking
/// whether the fingerprint columns hold an answer
/// (`content_hash::needs_fingerprint`), so nothing would ever come back
/// to redo this. A bookkeeping note that could not be written must not
/// take the observation with it — it is said out loud instead.
async fn check_declaration(
    env: &JobEnv,
    asset_id: &AssetId,
    ord: u32,
    fingerprint: &MaterialFingerprint,
) -> bool {
    if ord != 0 {
        return false;
    }
    let asset = match env.deps.assets.find(asset_id).await {
        Ok(Some(asset)) => asset,
        // Gone between the hash write and now, or unreadable. Neither
        // is a claim that disagreed.
        Ok(None) => return false,
        Err(err) => {
            tracing::warn!(
                event = "diag.material_hash.declaration_unreadable",
                asset_id = %asset_id,
                error = %err,
                "could not read the row to check its declared digest"
            );
            return false;
        }
    };
    let Some(declared) = asset
        .extra
        .get(provenance::TRACE_KEY)
        .and_then(|trace| trace.get(content_hash::DECLARED_HASH_NOTE_KEY))
        .and_then(|note| note.get("value"))
        .and_then(|value| value.as_str())
    else {
        return false;
    };
    let Some((axis, recomputed)) = declared_axis_value(declared, fingerprint) else {
        return false;
    };
    let disagreed = declared != recomputed;
    let note = content_hash::declaration_verdict(
        declared,
        recomputed,
        chrono::Utc::now().timestamp_millis(),
    );
    match env
        .deps
        .assets
        .note_trace_field(asset_id, content_hash::DECLARED_HASH_NOTE_KEY, note)
        .await
    {
        Ok(true) => {}
        Ok(false) => tracing::warn!(
            event = "diag.material_hash.declaration_note_skipped",
            asset_id = %asset_id,
            "extra column could not carry the declared-digest verdict"
        ),
        Err(err) => tracing::warn!(
            event = "diag.material_hash.declaration_note_failed",
            asset_id = %asset_id,
            error = %err,
            "could not record the declared-digest verdict"
        ),
    }
    if disagreed {
        // `action.`, not `diag.`: nothing in this process malfunctioned
        // — a file's bytes and the caller that registered them are two
        // statements about the corpus that do not agree, which is the
        // finding the declaration was accepted in order to produce.
        // Warn level because it wants reading; the durable half is the
        // note on the row, and the count reaches the job's own message.
        tracing::warn!(
            event = "action.material_hash.declaration_mismatch",
            asset_id = %asset_id,
            axis = %axis.as_str(),
            specified = %declared,
            got = %recomputed,
            "the registered digest is not what the bytes hash to"
        );
    }
    disagreed
}

/// The value a claim is checked against: the recomputed digest on the
/// axis the claim's own tag names — **and only when this pass actually
/// produced one on that axis**.
///
/// `None` has two causes and one meaning: nothing to check against, so
/// the claim stays unverified.
///
/// The first is a claim with no tag, which cannot have come through
/// `parse_declaration` and can therefore only be a hand-written
/// `_trace`. Picking an axis for it would be this function inventing
/// the one thing the claim failed to say.
///
/// The second is a **marker** where the digest would have been:
/// `unsupported:too-large` on a PNG past the size gate,
/// `unsupported:<mime>` on a format with no walker,
/// `unsupported:empty-span` on one that walked to nothing. Those are
/// not smaller digests, they are the record of a measurement that did
/// not happen — and comparing a claim against one manufactures a
/// disagreement between the caller and *nothing*. It would report "the
/// registered digest is not what the bytes hash to" about bytes this
/// build never hashed, and the person reading that alarm goes and looks
/// at a file that is fine. The caller most likely to trip it is the one
/// doing everything right: a correct `cr1-sha256:` declaration on a PNG
/// that happens to be over 64 MiB.
///
/// Not checking is not a gap left open — it is the state the note
/// vocabulary already has a spelling for. `declaration_claim` carries no
/// `verified` field precisely so that "nobody has checked this yet" is
/// representable, and a claim whose axis this build cannot measure is in
/// exactly that state. Should the gate be raised, or a walker for the
/// format land, the row returns to the fingerprint walk and the check
/// arrives then.
///
/// The test is the *prefix*, not `is_duplicate_key`: the empty-file
/// digest is excluded from duplicate grouping and is still a true
/// digest that a caller may declare and this job must confirm.
fn declared_axis_value<'a>(
    declared: &str,
    fingerprint: &'a MaterialFingerprint,
) -> Option<(DuplicateAxis, &'a str)> {
    let axis = content_hash::axis_of(declared)?;
    let recomputed = match axis {
        DuplicateAxis::Artefact => fingerprint.file.as_str(),
        DuplicateAxis::Content => fingerprint.content.as_str(),
        DuplicateAxis::Meta => fingerprint.meta.as_str(),
    };
    (content_hash::axis_of(recomputed) == Some(axis)).then_some((axis, recomputed))
}

/// Runs duplicate detection over a fingerprint that has just been
/// written, and **swallows every error it produces**, returning whether
/// a match was found.
///
/// The swallowing is the point of the function existing separately.
/// A hash is an observation about bytes and it is already committed; a
/// conflict is a derivation from it. Propagating a failed derivation
/// would fail the job whose successful half is already durable — and the
/// backfill walk finds work by asking whether the fingerprint columns
/// hold an answer (`content_hash::needs_fingerprint`), so once both are
/// written nothing comes back to redo the lookup for this row, while
/// the failure itself would keep the whole page from reporting what it
/// did hash. An unraised conflict, by contrast, is raised again the next
/// time either side of the pair is fingerprinted.
async fn detect_after_hash(
    env: &JobEnv,
    asset_id: &AssetId,
    ord: u32,
    fingerprint: &MaterialFingerprint,
    origin: DetectionOrigin,
) -> bool {
    let outcome = detect_duplicate(
        DetectionPorts {
            assets: &env.deps.assets,
            edges: &env.deps.edges,
            queue: &env.queue,
        },
        asset_id,
        ord,
        fingerprint,
        origin,
        chrono::Utc::now(),
    )
    .await;
    conflict_reported(asset_id, outcome)
}

/// Logs one detection result and reduces it to "was this news".
///
/// Split out of [`detect_after_hash`] so the swallowing is a value a
/// test can hold rather than a control-flow shape only a running job
/// exercises: the return type is `bool`, so there is no path by which a
/// failed derivation can become the job's verdict.
fn conflict_reported(asset_id: &AssetId, outcome: Result<Detection, DomainError>) -> bool {
    match outcome {
        Ok(Detection::NotApplicable) | Ok(Detection::Unique) => false,
        Ok(found) => {
            tracing::info!(
                event = "action.duplicate.detected",
                asset_id = %asset_id,
                outcome = %found.describe(),
                "duplicate detection"
            );
            true
        }
        Err(err) => {
            tracing::warn!(
                event = "diag.duplicate.detection_failed",
                asset_id = %asset_id,
                error = %err,
                "duplicate detection failed after the hash was written"
            );
            false
        }
    }
}

/// Folds one asset into another — the structural half of resolving a
/// duplicate.
///
/// Payload: `{ "asset_id": "<uuid>", "keeper_id": "<uuid>" }`. The
/// first is the row that becomes a headstone, the second the row that
/// stays.
///
/// # Why this is not part of `material_hash`
///
/// The fingerprint is what raises the conflict, so folding from inside
/// the hash handler would save a queue hop. It would also make a
/// partial failure permanent: the backfill walk finds work by asking
/// whether the fingerprint columns hold an answer
/// (`content_hash::needs_fingerprint`), so once both are written the row
/// is one the walk will never look at again — a run that died between the hash
/// write and the fold would leave a state nothing comes back for, and
/// this job engine has no retries (`JobKind::AssetFold`). Split, each
/// half is either done or never started. The second reason is plainer:
/// hashing is a fact about bytes, folding is a decision about identity.
///
/// # What it does to the keeper
///
/// The structure that pointed at the folded row moves, and the columns
/// the two rows both carry are combined by the rules
/// `AssetRepository::fold_into` documents: sets union, `rating` takes
/// the larger of the two, a restricted side wins, notes are joined.
/// Every other column keeps the keeper's value — and what the headstone
/// held there is written to the keeper's `_trace.absorbed`, because
/// "the keeper's title stands" is a decision, not an observation that
/// the two titles were the same.
///
/// # Order of the two side effects
///
/// The database transaction commits **first**; the search document and
/// the Query Group invalidation follow. That direction is chosen for
/// how it fails:
///
/// - **This way**, a crash after the commit leaves a Tantivy document
///   naming a row that is now a headstone. It cannot resurrect it — the
///   search path intersects its hits with the SQL population, which
///   excludes folded rows — so the cost is a stale document until the
///   next reindex. That is the same judgement
///   `AssetService::unindex_removed_assets` already records for trash
///   and purge, in the same words: the stale document is the
///   recoverable direction.
/// - **The other way**, deleting the document first, a fold that then
///   failed to commit would leave a **live** asset missing from search.
///   Nothing restores that: the index backfill looks for rows with no
///   cached body, and this row's body cache is intact, so it would stay
///   unfindable until somebody rebuilt the whole index by hand.
///
/// The invalidation goes last for the same reason it is not in the
/// transaction: it is a notification, and a missed one costs a stale
/// Query Group membership until the next refresh.
///
/// # Why a row that is already a headstone is not simply skipped
///
/// [`FoldRefusal::AlreadyFolded`] means the transaction has nothing left
/// to do, and it used to mean this job had nothing left to do either.
/// Both of the effects above are about the *state* of the row — a
/// headstone must not be retrievable, and a persona holding a fresh one
/// has stale Query Group memberships — so they are owed whether or not
/// this particular invocation is the one that stood it up. Two callers
/// arrive in exactly that state:
///
/// - [`AssetService::merge_assets`](asterism_core::application::AssetService::merge_assets),
///   whose transaction folds the rows itself and then enqueues this job
///   for the half that lives outside a transaction. Without this branch
///   the manual merge would enqueue a job that reports "already folded"
///   and cleans nothing — the bug the enqueue was added to fix.
/// - A re-run after a crash between the commit and the removal, which is
///   the recoverable direction the section above chooses deliberately.
///   This is what makes it recoverable rather than merely survivable.
///
/// Every other refusal is still a skip: the row is not a headstone, so
/// there is no document that should not be there.
pub async fn asset_fold(env: &JobEnv, payload: &serde_json::Value) -> Result<String, DomainError> {
    use asterism_core::domain::repository::{FoldOutcome, FoldRefusal};

    let headstone = payload_asset_id(payload, "asset_id")?;
    let keeper = payload_asset_id(payload, "keeper_id")?;

    match env.deps.assets.fold_into(&headstone, &keeper).await? {
        FoldOutcome::Skipped(FoldRefusal::AlreadyFolded) => {
            let unindexed = retire_headstone(env, &headstone).await?;
            Ok(format!(
                "asset_fold: {headstone} was already folded; \
                 stood down outside the transaction (unindexed={unindexed})"
            ))
        }
        FoldOutcome::Skipped(refusal) => Ok(format!(
            "asset_fold skipped: {headstone} into {keeper}: {}",
            refusal.as_str()
        )),
        FoldOutcome::Folded(report) => {
            let unindexed = retire_headstone(env, &headstone).await?;

            // The other side of the fold. Everything the headstone held
            // that is *text* — its keywords, its labels, the comment
            // thread that followed it — is on the keeper now, so the
            // keeper's document describes a row that has since grown.
            // This is the automatic path's half of what `merge_assets`
            // does by hand for the ruled path; without it a fold reached
            // through duplicate detection leaves the absorbed words
            // unfindable under the row that now holds them.
            if let Err(err) = enqueue_reindex(env, &keeper).await {
                tracing::warn!(
                    event = "diag.fold.keeper_reindex_failed",
                    asset_id = %keeper,
                    error = %err,
                    "the keeper of a fold was not queued for re-composition"
                );
                // The same fallback `AssetService::reindex` and
                // `AssetCommentService::reindex` take, for the same
                // reason: the backfill walk selects bodies composed by
                // an *older* reading, and this keeper's body carries
                // the current stamp, so the walk passes straight over
                // it. Clearing the stamp is what puts it back in front
                // of the walk. If that write fails too, the queue and
                // the database are both refusing writes and there is
                // nothing further to try from here.
                if let Err(err) = env.deps.asset_bodies.unstamp(&keeper).await {
                    tracing::warn!(
                        event = "diag.fold.keeper_unstamp_failed",
                        asset_id = %keeper,
                        error = %err,
                        "a fold's keeper keeps a document composed before it absorbed"
                    );
                }
            }

            Ok(format!(
                "asset_fold: {headstone} into {keeper} \
                 (edges {}→keeper, {} dropped; buckets {}; children {}; tags {}; \
                 comments {}; threads {}; columns merged {}; values discarded {}; \
                 unindexed={unindexed})",
                report.edges_repointed,
                report.edges_dropped,
                report.buckets_moved,
                report.children_repointed,
                report.tags_moved,
                report.comments_moved,
                report.threads_reanchored,
                report.columns_merged,
                report.values_discarded,
            ))
        }
    }
}

/// The half of a fold that lives outside the transaction: take the
/// headstone out of retrieval and tell its persona's Query Groups.
/// Returns whether the document is gone, for the job's log line.
///
/// One function because two branches of [`asset_fold`] owe it — the one
/// that folded the row and the one that found it already folded — and
/// the day a third effect is added, "which entry point remembered it"
/// must not be a question anyone can answer differently.
///
/// Index failure is logged and reported as `false`, never propagated:
/// the fold has already happened, so an error here would tell the caller
/// nothing landed while the headstone stands.
async fn retire_headstone(
    env: &JobEnv,
    headstone: &asterism_core::domain::value::AssetId,
) -> Result<bool, DomainError> {
    let unindexed = match env.deps.search_index.remove(headstone).await {
        Ok(()) => env.deps.search_index.flush().await.is_ok(),
        Err(err) => {
            tracing::warn!(
                event = "diag.fold.unindex_failed",
                asset_id = %headstone,
                error = %err,
                "retrieval index remove failed after a fold"
            );
            false
        }
    };

    // The durable half of the same removal. The body cache is what a
    // Tantivy rebuild reads, so a headstone that keeps its body comes
    // back as a hit the next time the index is rebuilt — the removal
    // above would be undone by the very mechanism that exists to repair
    // the index.
    if let Err(err) = env.deps.asset_bodies.delete(headstone).await {
        tracing::warn!(
            event = "diag.fold.body_delete_failed",
            asset_id = %headstone,
            error = %err,
            "a headstone kept its cached body after a fold"
        );
    }

    // Group membership and tag links both moved, and both are Query
    // Group rule inputs — so is the row leaving the live population at
    // all.
    if let Some(asset) = env.deps.assets.find(headstone).await? {
        notify_query_groups(env, asset.persona_id);
    }
    Ok(unindexed)
}

/// Reads a required asset id out of a job payload.
fn payload_asset_id(payload: &serde_json::Value, field: &str) -> Result<AssetId, DomainError> {
    let raw = payload
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| DomainError::Validation(format!("job payload missing {field}")))?;
    uuid::Uuid::parse_str(raw)
        .map(AssetId::from_uuid)
        .map_err(|_| DomainError::Validation(format!("invalid {field}: {raw:?}")))
}

/// The format fact of the asset's primary material (`ord == 0`).
///
/// The single place "what kind of bytes is this?" is answered on the
/// thumbnail path, so the eligibility test and the extractor choice
/// cannot disagree about an asset — reading the material list twice
/// with two different predicates is how a video ends up accepted as
/// thumbnailable and then handed to a still decoder.
fn primary_mime(asset: &asterism_core::domain::asset::Asset) -> Option<&MimeType> {
    asset
        .materials
        .iter()
        .find(|m| m.ord == 0)
        .and_then(|m| m.mime.as_ref())
}

/// Extracts a 5-colour dominant palette from a JPEG-encoded thumbnail
/// via `color-thief`. The bytes are decoded once more (already tiny at
/// 128 px) and fed to the quantiser; the return is 5 lowercase hex
/// strings.
fn extract_palette(jpeg: &[u8]) -> anyhow::Result<Vec<String>> {
    use image::codecs::jpeg::JpegDecoder;
    use image::{DynamicImage, GenericImageView};
    let decoder = JpegDecoder::new(std::io::Cursor::new(jpeg))?;
    let img = DynamicImage::from_decoder(decoder)?;
    let (w, h) = img.dimensions();
    let rgba = img.into_rgba8();
    let raw = rgba.as_raw().clone();
    let palette = color_thief::get_palette(
        &raw,
        color_thief::ColorFormat::Rgba,
        // `quality` is inversely related to work: 10 is the default
        // "good enough" tradeoff, 1 is the highest quality but
        // slowest. 128 px thumbs give tiny work either way.
        10,
        // We ask for 5 colours; color-thief may return fewer for
        // near-monochrome images.
        6, // pass 6 to get up to 5 back — see crate docs
    )
    .map_err(|e| anyhow::anyhow!("color-thief: {e:?}"))?;
    let _ = (w, h);
    Ok(palette
        .into_iter()
        .take(5)
        .map(|c| format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b))
        .collect())
}

/// Decodes + resizes + re-encodes a source artefact to a `size_px`
/// longer-edge JPEG thumbnail. Runs inside `spawn_blocking`.
///
/// `is_video` picks the extraction family and `mime` the route within
/// it — every route returns the same JPEG blob:
///
/// - **Video, macOS, AVFoundation-native formats**:
///   `thumb_video::make_thumb` pulls one frame via AVFoundation.
/// - **Video, formats macOS cannot demux (webm / mkv / avi)**:
///   `thumb_ffmpeg::make_thumb` shells out to an installed ffmpeg
///   binary (`thumb_ffmpeg::route_for` owns the split). Without one
///   the failure names the fix instead of leaving a silent empty
///   tile.
/// - **Video, other platforms**: the ffmpeg route for everything —
///   there is no AVFoundation to prefer.
/// - **Image, macOS**: `thumb_macos::make_thumb` drives Apple's
///   ImageIO framework. On Apple Silicon this route uses the hardware
///   JPEG decoder, so a full import wave costs almost no CPU on the
///   main cores.
/// - **Image, other platforms**: `make_thumb`, the pure-Rust
///   `image`-crate path.
fn decode_and_encode(
    path_str: &str,
    size_px: u32,
    is_video: bool,
    mime: Option<&MimeType>,
) -> Result<Vec<u8>, DomainError> {
    #[cfg(target_os = "macos")]
    {
        use super::thumb_ffmpeg::{self, VideoThumbRoute};
        if is_video {
            return match thumb_ffmpeg::route_for(mime) {
                VideoThumbRoute::Native => super::thumb_video::make_thumb(path_str, size_px),
                VideoThumbRoute::ExternalFfmpeg => thumb_ffmpeg::make_thumb(path_str, size_px),
            };
        }
        super::thumb_macos::make_thumb(path_str, size_px)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = mime;
        if is_video {
            return super::thumb_ffmpeg::make_thumb(path_str, size_px);
        }
        make_thumb(path_str, size_px)
    }
}

/// Decodes an image on disk, resizes it to the target size on the
/// longer edge, and JPEG-encodes it. Runs inside `spawn_blocking`
/// because `image` is synchronous.
#[cfg(not(target_os = "macos"))]
fn make_thumb(path_str: &str, target_px: u32) -> Result<Vec<u8>, DomainError> {
    use image::codecs::jpeg::JpegEncoder;
    use image::{ColorType, GenericImageView, ImageReader};

    let path = std::path::Path::new(path_str);
    let reader = ImageReader::open(path)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("thumb open {}: {e}", path.display())))?
        .with_guessed_format()
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("thumb guess {}: {e}", path.display())))?;
    let img = reader
        .decode()
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("thumb decode {}: {e}", path.display())))?;
    let (w, h) = img.dimensions();
    let scaled = if w.max(h) <= target_px {
        img
    } else {
        let (nw, nh) = if w >= h {
            (
                target_px,
                (h as f64 * target_px as f64 / w as f64).round() as u32,
            )
        } else {
            (
                (w as f64 * target_px as f64 / h as f64).round() as u32,
                target_px,
            )
        };
        img.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3)
    };
    let rgb = scaled.to_rgb8();
    let mut buffer = Vec::with_capacity(32 * 1024);
    JpegEncoder::new_with_quality(&mut buffer, THUMB_JPEG_QUALITY)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ColorType::Rgb8.into(),
        )
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("thumb encode {}: {e}", path.display())))?;
    Ok(buffer)
}

/// Backfill page size for `index_rebuild --batch`. Chosen so a
/// full scan of 100 k assets completes in ~50 batches (each ~seconds
/// wall-clock on a warm-cache SSD) [estimated]. Larger pages amortise
/// the tantivy `commit` cost but grow the memory footprint of the
/// resolved-body buffer.
const INDEX_BACKFILL_PAGE: u32 = 200;

/// Rebuilds the Tantivy full-text index for one asset (single-doc
/// mode) or performs a backfill pass over assets that do not yet
/// have an `asset_body` row (`{"batch": true}` payload).
///
/// Payload shapes:
/// - `{ "asset_id": "<uuid>" }` — single doc mode. Resolves the
///   body via [`SourceTextReader`], upserts `asset_body`, adds a
///   Tantivy document, and commits. Enqueued by `AssetService::add`
///   at ingest time.
/// - `{ "batch": true }` — backfill mode. Scans for assets whose cached
///   body is missing or was composed by an older reading of the asset,
///   processes one page (`INDEX_BACKFILL_PAGE`), then chain-enqueues
///   itself for the next page when the page was full. Idempotent —
///   re-running against a fully-composed DB is a no-op
///   (`scan_stale_body` returns empty).
///
/// Failure per asset (locator unreadable, tantivy write error) is
/// logged and skipped — the row simply lands without a body cache
/// entry and the next backfill picks it up.
pub async fn index_rebuild(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    // Batch mode branches early — no `asset_id` field is required.
    if payload
        .get("batch")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return index_rebuild_batch(env, payload).await;
    }

    let Some(asset) = load_target(env, payload).await? else {
        return Ok("asset gone, skipped".into());
    };
    // A trashed asset must not come back as a search hit. The check
    // belongs here rather than at enqueue time because the queue is
    // asynchronous: an asset can be trashed after its job was enqueued
    // (import → trash, or restore → trash), and `AssetRepository::find`
    // deliberately returns trashed rows.
    if asset.trashed_at.is_some() {
        return Ok(format!("asset {} is in the trash, skipped", asset.id));
    }
    // Same shape, second axis: a folded row must not come back as a
    // search hit either, and the queue is asynchronous here too — the
    // fold that resolved a duplicate can land after this job was
    // enqueued (ingest → hash → conflict → fold is exactly that
    // sequence). The keeper carries the text; indexing the headstone as
    // well would put two hits on screen for one asset, one of which the
    // grid cannot show.
    if let Some(keeper) = &asset.folded_into {
        return Ok(format!(
            "asset {} was folded into {keeper}, skipped",
            asset.id
        ));
    }
    let locator = asset.source.locator.clone();
    // The original's bytes are one *section* of the document, not the
    // document. Reading them is still gated on the format — a picture
    // reaching the text reader came back as its own bytes spelled as
    // lossy UTF-8, indexed and tokenised [measured 2026-08-05] — but a
    // failed gate no longer ends the job: the words about a picture
    // (title / cover / labels / keywords / material metadata / declared
    // meta / comments) are what `derive_text` composes, and they exist
    // whether or not the file is readable as text.
    let file_body = match TextLocator::new(locator.clone(), primary_mime(&asset)) {
        Some(text) => env
            .deps
            .source_texts
            .read_batch(std::slice::from_ref(&text))
            .await?
            .into_iter()
            .next()
            .flatten(),
        None => None,
    };
    let comment_bodies: Vec<String> = env
        .deps
        .comments
        .list_by_asset(&asset.id)
        .await?
        .into_iter()
        .map(|c| c.body)
        .collect();
    let Some(body) = derive_text(&asset, file_body.as_deref(), &comment_bodies) else {
        retract_document(env, &asset.id, asset.persona_id).await?;
        return Ok(format!(
            "no derivable text for {}, skipped",
            locator.to_display()
        ));
    };
    env.deps.asset_bodies.upsert(&asset.id, &body).await?;
    env.deps
        .search_index
        .upsert(&IndexDoc {
            asset_id: asset.id,
            persona_id: asset.persona_id,
            text: Some(body.clone()),
        })
        .await?;
    env.deps.search_index.flush().await?;
    // A fresh retrieval document changes what search_text-carrying
    // query groups can match (W4-a).
    notify_query_groups(env, asset.persona_id);
    Ok(format!("indexed asset {} ({} bytes)", asset.id, body.len()))
}

/// Takes one asset out of the search surface entirely — the cached
/// body and the retrieval document together.
///
/// Called from both composing paths when an asset derives to nothing,
/// so that the single-doc job and the backfill page leave a row in the
/// same state. They did not, briefly: the page dropped the body and
/// left the document, which is the worse half of the pair to leave
/// behind, since Tantivy is what search actually answers from.
///
/// **The delete leads and its answer gates the rest.** A row that had
/// no body had no document either — the two are written by the same
/// handler in the same breath — so on a walk over a library of rows
/// that never had anything to say, an unconditional retraction is one
/// index write, one flush and one Query Group notification per row, all
/// of them saying nothing changed. Asking the cache first turns that
/// into one cheap `DELETE` that matches nothing.
async fn retract_document(
    env: &JobEnv,
    asset_id: &AssetId,
    persona_id: asterism_core::domain::value::PersonaId,
) -> Result<(), DomainError> {
    if !env.deps.asset_bodies.delete(asset_id).await? {
        return Ok(());
    }
    env.deps.search_index.remove(asset_id).await?;
    env.deps.search_index.flush().await?;
    notify_query_groups(env, persona_id);
    Ok(())
}

/// One page of the backfill scan. Chain-enqueues itself while the
/// last page was full so the whole backlog drains without a driver
/// process.
///
/// The scan (`scan_stale_body`) takes two states, and it needs both.
/// "No `asset_body` row" is every picture ever imported, because the
/// pre-derivation handler refused them all. "A body stamped with an
/// older
/// [`COMPOSITION_VERSION`](asterism_core::domain::derived_text::COMPOSITION_VERSION)"
/// is the other half, and leaving it out is what made this walk look
/// finished while it was not: a **text** asset indexed before derivation
/// existed already had a body (the file's bytes), so it was invisible
/// here and its own title, keywords and comment thread never reached its
/// document.
///
/// Raising that constant is therefore the supported way to re-compose a
/// library after teaching `derive_text` a new section — the walk finds
/// every row exactly once and stops, with no predicate that means "read
/// every source file again".
async fn index_rebuild_batch(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    // Cursor is opaque UUID text — comes back from the previous
    // page enqueue. Absent = start from scratch.
    let cursor_str = payload.get("cursor").and_then(|v| v.as_str());
    let cursor_id = match cursor_str {
        Some(s) => Some(
            uuid::Uuid::parse_str(s)
                .map(asterism_core::domain::value::AssetId::from_uuid)
                .map_err(|_| DomainError::Validation(format!("invalid cursor uuid: {s:?}")))?,
        ),
        None => None,
    };
    let page = env
        .deps
        .assets
        .scan_stale_body(cursor_id.as_ref(), INDEX_BACKFILL_PAGE)
        .await?;
    if page.is_empty() {
        return Ok("index backfill: done (no more assets)".into());
    }
    // Batch-resolve source texts by container so a 200-message
    // Claude Code session costs one file pass. Rows whose bytes are
    // not text are not *read* — the scan returns the format for
    // exactly that decision — but they stay in the page: a picture has
    // no file body and is still the case this walk exists to index.
    let last_id_str: String = page.last().unwrap().asset_id.to_string();
    let rows: Vec<(AssetId, Option<TextLocator>)> = page
        .into_iter()
        .map(|row| {
            let asset_id = row.asset_id;
            (asset_id, TextLocator::new(row.locator, row.mime.as_ref()))
        })
        .collect();
    let locators: Vec<TextLocator> = rows.iter().filter_map(|(_, l)| l.clone()).collect();
    let mut file_bodies = env
        .deps
        .source_texts
        .read_batch(&locators)
        .await?
        .into_iter();
    let mut indexed = 0u64;
    let mut skipped_no_text = 0u64;
    // Rows that were trashed or folded between the scan and their turn
    // in the page. Counted rather than folded into `skipped_no_text`
    // because they are a different event: those rows have plenty to say
    // and are simply no longer part of the live population.
    let mut skipped_gone = 0u64;
    // Personas whose search dimension this page changed — notified
    // once after the commit (W4-a); the per-persona debounce
    // additionally collapses across chained pages.
    let mut touched_personas = std::collections::HashSet::new();
    for (asset_id, locator) in rows {
        // `read_batch` answers one slot per locator handed in, in
        // order, so the reader's answers are drawn only for the rows
        // that contributed a locator — pulling one per row would
        // desynchronise the two sequences at the first picture.
        let file_body = match locator {
            Some(_) => file_bodies.next().flatten(),
            None => None,
        };
        // The slim scan row carries the locator and the mime and
        // nothing else; composing needs the whole asset (labels,
        // keywords, cover, materials and their metadata), so the row
        // is hydrated here. One `find` per asset is the cost of the
        // walk seeing what the single-doc path sees.
        let Some(asset) = env.deps.assets.find(&asset_id).await? else {
            continue;
        };
        // The same two guards the single-doc path carries, for the same
        // reason and against a shorter race: the scan excluded trashed
        // rows and headstones in SQL, but a page is composed one row at
        // a time after that query returned, and a trash or a fold
        // landing in between would otherwise put a row into the index
        // that the grid cannot show.
        if asset.trashed_at.is_some() || asset.folded_into.is_some() {
            skipped_gone += 1;
            continue;
        }
        let persona_id = asset.persona_id;
        let comment_bodies: Vec<String> = env
            .deps
            .comments
            .list_by_asset(&asset_id)
            .await?
            .into_iter()
            .map(|c| c.body)
            .collect();
        let Some(body) = derive_text(&asset, file_body.as_deref(), &comment_bodies) else {
            skipped_no_text += 1;
            // Now that the scan reaches rows that already have a body,
            // "nothing to say" can be a *change* rather than a state the
            // row was always in — the words that produced the cached
            // body may have been deleted since. The retraction is the
            // single-doc path's, verbatim, so a row ends this walk in
            // the state that path would have left it in: no body **and**
            // no document. A row that never had either pays one `DELETE`
            // that matches nothing.
            if let Err(err) = retract_document(env, &asset_id, persona_id).await {
                tracing::warn!(
                    event = "diag.retrieval.retract_failed",
                    asset_id = %asset_id,
                    error = %err,
                    "a row with nothing left to say kept its search surface"
                );
            }
            continue;
        };
        if let Err(err) = env.deps.asset_bodies.upsert(&asset_id, &body).await {
            tracing::warn!(
                event = "diag.asset_body.upsert_failed",
                asset_id = %asset_id,
                error = %err,
                "asset_body upsert failed"
            );
            continue;
        }
        if let Err(err) = env
            .deps
            .search_index
            .upsert(&IndexDoc {
                asset_id,
                persona_id,
                text: Some(body),
            })
            .await
        {
            tracing::warn!(
                event = "diag.retrieval.upsert_failed",
                asset_id = %asset_id,
                error = %err,
                "retrieval index upsert failed"
            );
            continue;
        }
        touched_personas.insert(persona_id);
        indexed += 1;
    }
    env.deps.search_index.flush().await?;
    for persona_id in touched_personas {
        notify_query_groups(env, persona_id);
    }
    // Chain-enqueue the next page. Empty result on the next run
    // terminates the walk.
    env.queue
        .enqueue(
            asterism_core::domain::job::JobKind::IndexRebuild,
            serde_json::json!({ "batch": true, "cursor": last_id_str }),
        )
        .await?;
    Ok(format!(
        "index backfill page: indexed={indexed} skipped_no_text={skipped_no_text} \
         skipped_gone={skipped_gone} \
         next_cursor={last_id_str}"
    ))
}

/// Page size for one retention sweep. Deliberately far below the
/// repository's 5 000 clamp: this is the only scheduled job that
/// destroys data, so a page is sized to finish quickly and hand the
/// writer back rather than to drain the backlog in one pass. A full
/// page chain-enqueues the next one.
const TRASH_PURGE_PAGE: u32 = 200;

/// Retention sweep — purges assets and Groups whose trash stamp has
/// aged past the configured retention period. Payload is empty (`{}`).
///
/// The cutoff is **not** in the payload: it is derived inside
/// `RetentionService::purge_expired` from the injected retention
/// value, so a job that sat in the queue across a policy change purges
/// on the current policy rather than the one in force when it was
/// enqueued.
///
/// Chain-enqueues itself while a page comes back full **and made
/// progress**. Both halves matter:
///
/// - "full" is measured on rows *scanned*, so a page is not mistaken for
///   the end of the backlog just because some rows were skipped.
/// - "made progress" is what stops a spin. A page that purged nothing
///   would otherwise re-enqueue instantly and forever, and since this
///   queue has no retry policy the failure would surface only as an
///   ever-growing `Jobs` table. The one *expected* skip — a restore
///   landing mid-sweep — needs no chaining anyway: the restored row
///   fails `scan_purgeable`'s predicate on the next pass, so it is gone
///   from the scan regardless. That leaves genuine errors, and retrying
///   those in a tight loop helps nobody; the next startup sweep picks
///   the page back up.
pub async fn trash_purge(
    env: &JobEnv,
    _payload: &serde_json::Value,
) -> Result<String, DomainError> {
    let Some(service) = env.deps.retention_service.get() else {
        return Ok("no retention service bound, skipped".into());
    };
    let sweep = service
        .purge_expired(chrono::Utc::now(), TRASH_PURGE_PAGE)
        .await?;
    if sweep.is_empty() {
        return Ok("retention sweep: nothing past retention".into());
    }
    if sweep.should_chain(TRASH_PURGE_PAGE) {
        env.queue
            .enqueue(
                asterism_core::domain::job::JobKind::TrashPurge,
                serde_json::json!({}),
            )
            .await?;
    }
    Ok(format!(
        "retention sweep: assets {}/{} groups {}/{} personas {}/{} skipped={}",
        sweep.assets_purged,
        sweep.assets_scanned,
        sweep.groups_purged,
        sweep.groups_scanned,
        sweep.personas_purged,
        sweep.personas_scanned,
        sweep.skipped
    ))
}

/// Page size for one observation retention pass.
///
/// Larger than the trash sweep's 200: a perf backlog is counted in
/// hundreds of thousands of rows and every page costs a chain hop,
/// while the work per row is one indexed delete plus its cascade
/// rather than a whole entity's teardown. 500 rows with their tags
/// take ~6 ms [measured: 300k-row `perf_log` on a reproduction of the V36
/// schema, SQLite 3.43.2], which is short enough to hand the shared
/// connection back between pages.
const OBSERVATION_SWEEP_PAGE: u32 = 500;

/// Expires observations past their stream's declared retention.
/// Payload is empty (`{}`) — the windows come from `STREAM_REGISTRY`,
/// never from the payload, so a queued job cannot sweep on a policy
/// that has since changed.
///
/// Chain-enqueues while a page comes back full. Unlike `trash_purge`
/// this needs no "made progress" guard: a full page here means rows
/// were deleted, so the backlog strictly shrinks and the chain
/// terminates.
pub async fn observation_sweep(
    env: &JobEnv,
    _payload: &serde_json::Value,
) -> Result<String, DomainError> {
    let sweep = env
        .deps
        .observations
        .sweep_retention(
            chrono::Utc::now().timestamp_millis(),
            OBSERVATION_SWEEP_PAGE,
        )
        .await?;
    if sweep.total() == 0 {
        return Ok("observation sweep: nothing past retention".into());
    }
    if sweep.should_chain(OBSERVATION_SWEEP_PAGE) {
        env.queue
            .enqueue(
                asterism_core::domain::job::JobKind::ObservationSweep,
                serde_json::json!({}),
            )
            .await?;
    }
    let detail: Vec<String> = sweep
        .removed
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(stream, n)| format!("{stream} {n}"))
        .collect();
    Ok(format!("observation sweep: {}", detail.join(", ")))
}

/// Re-evaluates every Query Group under one persona. Payload:
/// `{ "persona_id": "<uuid>" }`. Failures per group are collected
/// and surfaced in the completion message but do not fail the job
/// (one corrupt rule must not stop the other groups' refresh).
pub async fn query_group_refresh(
    env: &JobEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    let raw: asterism_core::application::query_group_invalidation::QueryGroupRefreshPayload =
        serde_json::from_value(payload.clone()).map_err(|e| {
            DomainError::Validation(format!("query_group_refresh payload invalid: {e}"))
        })?;
    let persona = asterism_core::application::mapping::parse_persona_id(&raw.persona_id)?;
    let outcome = env
        .deps
        .query_group_refresh
        .refresh_for_persona(&persona)
        .await;
    if !outcome.failures.is_empty() {
        // Loud but non-fatal — the eval Job never retries by design,
        // so surface every failure for the progress banner and the
        // diagnostic record.
        for (bucket, err) in &outcome.failures {
            tracing::warn!(
                event = "diag.query_group.refresh_failed",
                bucket = ?bucket.as_ref().map(|b| b.to_string()),
                error = %err,
                "query_group_refresh bucket failed"
            );
        }
    }
    Ok(format!(
        "query_group_refresh · persona {persona} · refreshed={} failed={}",
        outcome.refreshed,
        outcome.failures.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use crate::sqlite::repo::SqliteModalityRepository;
    use asterism_core::domain::content_region;
    use asterism_core::domain::modality::ModalityDef;
    use asterism_core::domain::repository::ModalityRepository;
    use asterism_core::domain::value::Modality;

    /// Parses the spelling a caller sends into the locator, the way the
    /// ingest boundary does. (The *storage* spelling is the tagged form
    /// and is exercised where a column is involved.)
    fn loc(raw: impl AsRef<str>) -> SourceLocator {
        SourceLocator::from_wire(raw.as_ref()).expect("locator")
    }

    const DIALOGUE_BODY: &str = "First reply line\nSecond reply line\nThird reply line";
    const WORK_BODY: &str = "# Design Note\nThe body opens here";
    const TAPE_BODY: &str = "some banner\n❯ cargo test\nrunning 3 tests";
    const PLAIN_BODY: &str = "# Just A Heading\nmore text below";

    #[test]
    fn derive_cover_reproduces_each_template() {
        let any = loc("/tmp/x.md");
        // Dialogue — first two non-empty lines joined by " / ".
        assert_eq!(
            derive_cover(CoverTemplate::Dialogue, Some(DIALOGUE_BODY), &any),
            "First reply line / Second reply line"
        );
        // Work product — heading title + first body line.
        assert_eq!(
            derive_cover(CoverTemplate::WorkProduct, Some(WORK_BODY), &any),
            "Design Note — The body opens here"
        );
        // Tape — the first prompt line.
        assert_eq!(
            derive_cover(CoverTemplate::Tape, Some(TAPE_BODY), &any),
            "❯ cargo test"
        );
        // Generic first-line — heading markers stripped off line 1.
        assert_eq!(
            derive_cover(CoverTemplate::FirstLine, Some(PLAIN_BODY), &any),
            "Just A Heading"
        );
    }

    /// **An artefact this library did not make is never stamped.**
    ///
    /// The property the whole stamping path rests on: it rewrites
    /// files, and everything except an artefact a dispatch produced is
    /// somebody's own file that this application was asked to index and
    /// not to edit. Every shape an imported asset's `extra` can take
    /// has to answer `false`, including the ones that look adjacent.
    #[test]
    fn only_what_a_dispatch_produced_is_stamped() {
        let reified = serde_json::json!({
            "_dispatch": {
                "selection_id": "01930000-0000-7000-8000-000000000001",
                "dispatch_id": "01930000-0000-7000-8000-000000000002",
                "exporter_slug": "file",
            }
        });
        assert!(produced_by_dispatch(&reified));
        assert_eq!(
            dispatch_id_of(&reified).as_deref(),
            Some("01930000-0000-7000-8000-000000000002")
        );

        // Everything an import can leave behind.
        for imported in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({ "camera": "X-T5" }),
            // The exporter's own payload, kept beside the trace when it
            // was not an object — an artefact that carries this and no
            // trace did not come from a dispatch.
            serde_json::json!({ "_derived": "whatever the exporter said" }),
            // A key of the right name that is not a trace.
            serde_json::json!({ "_dispatch": "01930000-0000-7000-8000-000000000002" }),
            serde_json::json!({ "_dispatch": null }),
        ] {
            assert!(
                !produced_by_dispatch(&imported),
                "would have rewritten a file for {imported}"
            );
            assert_eq!(dispatch_id_of(&imported), None, "for {imported}");
        }
    }

    /// A trace with no `dispatch_id` still marks the artefact as ours,
    /// and the manifest simply omits the field.
    ///
    /// The two questions are separate: whether to stamp at all, and
    /// what to say about the run. Folding them would mean a trace
    /// missing one key silently turned stamping off.
    #[test]
    fn a_trace_without_a_run_id_still_belongs_to_this_library() {
        let extra = serde_json::json!({ "_dispatch": { "exporter_slug": "file" } });
        assert!(produced_by_dispatch(&extra));
        assert_eq!(dispatch_id_of(&extra), None);
    }

    /// A detection that failed is reported as "no conflict", never as
    /// the job's outcome.
    ///
    /// The hash is already committed by the time this runs, and the
    /// backfill walk finds work by asking whether the fingerprint
    /// columns hold an answer — so a
    /// failure that propagated would fail a job whose durable half
    /// succeeded, and nothing would ever come back to redo the lookup
    /// for that row. The unraised conflict, by contrast, is raised again
    /// the next time either side of the pair is fingerprinted.
    #[test]
    fn a_failed_detection_is_reported_as_no_conflict() {
        let id = AssetId::new();
        assert!(!conflict_reported(
            &id,
            Err(DomainError::Infra(anyhow::anyhow!("the queue is down")))
        ));
        // The other two non-events, so "false" is not just what this
        // function always answers.
        assert!(!conflict_reported(&id, Ok(Detection::Unique)));
        assert!(!conflict_reported(&id, Ok(Detection::NotApplicable)));
        // And the events that are news.
        let other = AssetId::new();
        assert!(conflict_reported(&id, Ok(Detection::Queued(other))));
        assert!(conflict_reported(&id, Ok(Detection::AlreadyQueued(other))));
        assert!(conflict_reported(&id, Ok(Detection::Folding(other))));
        assert!(conflict_reported(&id, Ok(Detection::Recorded(other))));
    }

    /// Builds a PNG the walker accepts. `PngBuilder::new()` supplies the
    /// signature and a 1×1 grayscale IHDR; `raw_chunk` writes IDAT and
    /// the optional `tEXt` with zero CRCs (the walker reads past them
    /// without checking, and says why). The `bare` / `noted` pair share
    /// this IHDR so their content-region digests differ only in the
    /// presence of the `tEXt` chunk — the whole point of the axis.
    fn png(pixels: &[u8], text: Option<&[u8]>) -> Vec<u8> {
        let mut b =
            pngmeta::test_util::PngBuilder::new().raw_chunk(*b"IDAT", pixels.len() as u32, pixels);
        if let Some(text) = text {
            b = b.raw_chunk(*b"tEXt", text.len() as u32, text);
        }
        b.build()
    }

    fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).expect("fixture written");
        path.to_string_lossy().into_owned()
    }

    /// Parses a literal for the fingerprint cases: the declared format
    /// crosses these functions parsed, not as a string.
    fn mime(raw: &str) -> MimeType {
        MimeType::parse(raw)
    }

    /// Teeth: a PNG comes back with a content-axis digest, a file that
    /// is not one comes back with a marker, and the marker names what
    /// the row claimed to be.
    ///
    /// The pair of PNGs differing only in a `tEXt` chunk is the whole
    /// point of the axis: two different files, one picture.
    #[test]
    fn a_png_gets_a_region_digest_and_anything_else_gets_a_marker() {
        use asterism_core::domain::content_hash::{CONTENT_DIGEST_PREFIX, DIGEST_PREFIX};

        let dir = tempfile::tempdir().unwrap();
        let pixels = b"a compressed stream, near enough";
        let bare = write(&dir, "bare.png", &png(pixels, None));
        let noted = write(&dir, "noted.png", &png(pixels, Some(b"workflow\0{...}")));

        let a = hash_artefact(&bare, Some(&mime("image/png")), MAX_CONTENT_WALK_BYTES).unwrap();
        let b = hash_artefact(&noted, Some(&mime("image/png")), MAX_CONTENT_WALK_BYTES).unwrap();

        assert!(a.file.starts_with(DIGEST_PREFIX));
        assert!(a.content.starts_with(CONTENT_DIGEST_PREFIX), "{a:?}");
        assert_ne!(a.file, b.file, "the two files really do differ");
        assert_eq!(a.content, b.content, "…and are the same picture");

        // A format with no walker: the file axis still answers, the
        // content axis says which format it declined and does not
        // pretend the bytes were read.
        let clip = write(&dir, "clip.mp4", b"\0\0\0\x18ftypmp42not really a movie");
        let video = hash_artefact(&clip, Some(&mime("video/mp4")), MAX_CONTENT_WALK_BYTES).unwrap();
        assert!(video.file.starts_with(DIGEST_PREFIX));
        assert_eq!(video.content, "unsupported:video/mp4");

        // A `.png` that is not one: the signature check refuses, and
        // the marker says only what is known.
        let liar = write(&dir, "liar.png", b"\xff\xd8\xff\xe0 a jpeg wearing a hat");
        let refused =
            hash_artefact(&liar, Some(&mime("image/png")), MAX_CONTENT_WALK_BYTES).unwrap();
        assert!(refused.file.starts_with(DIGEST_PREFIX));
        assert_eq!(refused.content, "unsupported:unknown");

        // A truncated PNG walks to no region, and the value stored is
        // the marker rather than the perfectly real digest of nothing.
        let whole = png(pixels, None);
        let cut = write(&dir, "cut.png", &whole[..whole.len() / 2]);
        let broken = hash_artefact(&cut, Some(&mime("image/png")), MAX_CONTENT_WALK_BYTES).unwrap();
        assert_eq!(broken.content, content_region::EMPTY_SPAN);
        assert!(broken.file.starts_with(DIGEST_PREFIX));
    }

    /// Teeth: past the size gate the file axis is still computed and
    /// the content axis is a marker — the failure being guarded is a
    /// job that answers neither, or that reads a gigabyte into memory
    /// because the ceiling was only in a doc comment.
    #[test]
    fn a_file_past_the_size_gate_keeps_its_file_digest_and_marks_the_other() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = png(b"pixels enough to be over a tiny ceiling", None);
        let path = write(&dir, "big.png", &bytes);

        let gate = (bytes.len() - 1) as u64;
        let gated = hash_artefact(&path, Some(&mime("image/png")), gate).unwrap();
        assert_eq!(gated.content, content_region::TOO_LARGE);

        // The same file under a ceiling it fits: the file axis agrees
        // with the gated run (so the gate changed one answer, not two)
        // and the content axis is now a digest.
        let walked = hash_artefact(&path, Some(&mime("image/png")), gate + 1).unwrap();
        assert_eq!(walked.file, gated.file);
        assert!(
            walked
                .content
                .starts_with(asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX),
            "{walked:?}"
        );
    }

    /// Teeth: a claim is checked against the recomputed value **on the
    /// axis it named**, and both verdicts are reachable.
    ///
    /// The failure this catches is the cheap one: comparing every claim
    /// with the file digest. A well-formed `cr1-sha256:` declaration
    /// would then mismatch every time, and the alarm would send someone
    /// to look at a file that is fine.
    #[test]
    fn a_declaration_is_checked_against_the_axis_it_named() {
        let fingerprint = MaterialFingerprint {
            file: content_hash::of_bytes(b"the whole file"),
            content: format!(
                "{}{}",
                asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX,
                "a".repeat(64)
            ),
            meta: format!(
                "{}{}",
                asterism_core::domain::content_hash::META_DIGEST_PREFIX,
                "a".repeat(64)
            ),
            meta_kv: Some(r#"{"prompt":"a cat"}"#.to_string()),
            meta_raw: None,
            meta_text: None,
        };

        for declared in [&fingerprint.file, &fingerprint.content, &fingerprint.meta] {
            let (_, recomputed) =
                declared_axis_value(declared, &fingerprint).expect("a tagged claim picks an axis");
            assert_eq!(recomputed, declared, "the claim agrees with its own axis");
            let note = content_hash::declaration_verdict(declared, recomputed, 0);
            assert_eq!(note["verified"], serde_json::json!(true));
        }

        // A content-axis claim that disagrees: the axis is still the
        // content one, and `got` carries the content value rather than
        // the file digest the naive comparison would have used.
        let wrong = format!(
            "{}{}",
            asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX,
            "b".repeat(64)
        );
        let (axis, recomputed) = declared_axis_value(&wrong, &fingerprint).unwrap();
        assert_eq!(axis, DuplicateAxis::Content);
        assert_eq!(recomputed, fingerprint.content);
        let note = content_hash::declaration_verdict(&wrong, recomputed, 0);
        assert_eq!(note["verified"], serde_json::json!(false));
        assert_eq!(note["got"], serde_json::json!(fingerprint.content));
        assert_eq!(note["axis"], serde_json::json!("content"));

        // An untagged claim names no axis, so nothing is asserted about
        // it — including that it disagreed.
        assert!(declared_axis_value("deadbeef", &fingerprint).is_none());
        assert!(declared_axis_value(UNHASHABLE, &fingerprint).is_none());
    }

    /// Teeth: a claim on an axis this pass did **not** measure is not
    /// checked at all, and therefore reports no mismatch.
    ///
    /// The failure this catches is a false alarm, which is worse than a
    /// missing one: a caller that declared a correct `cr1-sha256:` for a
    /// PNG over the size gate would be told "the registered digest is
    /// not what the bytes hash to" — about bytes this build never
    /// hashed — and whoever read that would go and look at a file that
    /// is fine. Comparing a digest against a marker manufactures a
    /// disagreement between the caller and nothing.
    #[test]
    fn a_claim_on_an_axis_that_was_not_measured_is_left_unchecked() {
        let declared = format!(
            "{}{}",
            asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX,
            "a".repeat(64)
        );

        // Every way the content axis ends up without a digest: past the
        // size gate, no walker for the format, walked to nothing, and
        // the row that predates the column.
        for marker in [
            content_region::TOO_LARGE,
            content_region::EMPTY_SPAN,
            content_region::NOT_WALKED,
            "unsupported:image/jpeg",
            "unsupported:unknown",
            UNHASHABLE,
        ] {
            let fingerprint = MaterialFingerprint {
                file: content_hash::of_bytes(b"the whole file"),
                content: marker.to_string(),
                meta: marker.to_string(),
                meta_kv: None,
                meta_raw: None,
                meta_text: None,
            };
            assert!(
                declared_axis_value(&declared, &fingerprint).is_none(),
                "{marker} is not a digest to check a claim against"
            );
        }

        // The same claim against a real region digest *is* checked —
        // otherwise the assertions above would hold for a function that
        // had simply stopped checking anything.
        let measured = MaterialFingerprint {
            file: content_hash::of_bytes(b"the whole file"),
            content: format!(
                "{}{}",
                asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX,
                "b".repeat(64)
            ),
            meta: content_region::NOT_WALKED.to_string(),
            meta_kv: None,
            meta_raw: None,
            meta_text: None,
        };
        let (axis, recomputed) =
            declared_axis_value(&declared, &measured).expect("a digest is checkable");
        assert_eq!(axis, DuplicateAxis::Content);
        assert_eq!(recomputed, measured.content);
        assert_ne!(recomputed, declared, "and this pair is a real mismatch");

        // The file axis takes the same rule, and the empty-file digest
        // stays checkable through it: it is excluded from duplicate
        // *grouping*, which is a different question from whether a
        // caller may declare it.
        let empty = MaterialFingerprint {
            file: asterism_core::domain::content_hash::EMPTY.to_string(),
            content: content_region::NOT_WALKED.to_string(),
            meta: content_region::NOT_WALKED.to_string(),
            meta_kv: None,
            meta_raw: None,
            meta_text: None,
        };
        let (axis, recomputed) =
            declared_axis_value(asterism_core::domain::content_hash::EMPTY, &empty)
                .expect("a true statement about a 0-byte file is still checkable");
        assert_eq!(axis, DuplicateAxis::Artefact);
        assert_eq!(recomputed, asterism_core::domain::content_hash::EMPTY);
    }

    /// Teeth: the two passes fingerprint one artefact identically.
    ///
    /// They read the format from different places — the per-asset pass
    /// from the hydrated entity, the backfill from its scan row — and
    /// if those two ever disagree the same file gets a digest through
    /// one door and a marker through the other, which reads downstream
    /// as two pictures.
    #[tokio::test]
    async fn both_passes_read_the_same_format_and_reach_the_same_value() {
        use crate::sqlite::repo::SqliteAssetRepository;
        use asterism_core::domain::material::Material;
        use asterism_core::domain::repository::AssetRepository;
        use asterism_core::domain::value::{SourceKind, SourceRef};

        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "shot.png", &png(b"the same pixels either way", None));

        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = {
            let pid = uuid::Uuid::now_v7();
            isle.call(move |conn| {
                conn.execute(
                    "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
                     VALUES (?1, 'pack', 'P', 0, 0)",
                    rusqlite::params![pid],
                )?;
                Ok(())
            })
            .await
            .unwrap();
            asterism_core::domain::value::PersonaId::from_uuid(pid)
        };

        let mut asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), &path).unwrap(),
            None,
            chrono::Utc::now(),
            // This fixture is about the format the walker reads, not
            // about who ingested the row: a caller that states nothing
            // records nothing.
            &asterism_core::domain::attribution::AttributionContext::asserted(None, None).unwrap(),
        );
        asset.materials = vec![Material::primary(loc(&path), None, chrono::Utc::now())];
        repo.save(&asset).await.unwrap();

        // The per-asset pass's source of the format.
        let hydrated = repo.find(&asset.id).await.unwrap().unwrap();
        let from_entity = hydrated.materials[0].mime.clone();
        // The backfill's.
        let page = repo.scan_unhashed_materials(None, 10).await.unwrap();
        let row = page
            .iter()
            .find(|m| m.asset_id == asset.id)
            .expect("a freshly saved material is work");
        assert_eq!(
            row.mime, from_entity,
            "the scan row and the entity have to agree before the digests can"
        );
        assert_eq!(from_entity, Some(mime("image/png")));

        let per_asset = hash_artefact(&path, from_entity.as_ref(), MAX_CONTENT_WALK_BYTES).unwrap();
        let backfill = hash_artefact(&path, row.mime.as_ref(), MAX_CONTENT_WALK_BYTES).unwrap();
        assert_eq!(per_asset, backfill);
        assert!(
            per_asset
                .content
                .starts_with(asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX),
            "a vacuous pass: both doors returned a marker ({per_asset:?})"
        );

        driver.shutdown().await.unwrap();
    }

    /// The `file://` defect, closed, and the record case beside it so
    /// the assertion is a discrimination rather than a blanket "hash
    /// everything".
    ///
    /// What `hash_material` used to do with a `file://` locator: the
    /// predicate said hashable, `File::open` was handed the *spelling*,
    /// the open failed, no marker was written, and the row came back on
    /// the next backfill pass forever. Here the same fixture travels the
    /// real path — saved, read back through the row boundary, taken off
    /// the backfill scan — and arrives as a path a decoder can open.
    ///
    /// The record beside it is the fixture the string tests answered
    /// wrongly in the other direction: its container's name carries a
    /// `#` of its own, so `is_hashable_locator` refused it (correctly,
    /// by luck) while `guess_mime` and the thumb gate refused the
    /// *container* too. The container is a file; the record is not.
    #[tokio::test]
    async fn a_file_scheme_locator_reaches_the_hasher_as_a_path() {
        use crate::sqlite::repo::SqliteAssetRepository;
        use asterism_core::domain::material::Material;
        use asterism_core::domain::repository::AssetRepository;
        use asterism_core::domain::value::{SourceKind, SourceRef};

        let dir = tempfile::tempdir().unwrap();
        // A container whose own name carries a `#`, holding one record.
        let container = write(&dir, "a#b.png", &png(b"container bytes", None));
        let path = write(&dir, "schemed.png", &png(b"schemed bytes", None));

        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = {
            let pid = uuid::Uuid::now_v7();
            isle.call(move |conn| {
                conn.execute(
                    "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
                     VALUES (?1, 'pack', 'P', 0, 0)",
                    rusqlite::params![pid],
                )?;
                Ok(())
            })
            .await
            .unwrap();
            asterism_core::domain::value::PersonaId::from_uuid(pid)
        };

        let kind = SourceKind::new(SourceKind::FS).unwrap();
        let mut hashable = Asset::new(
            persona,
            // Registered with the scheme, the way an importer that
            // spelled it as a URL would.
            SourceRef::new(kind.clone(), format!("file://{path}")).unwrap(),
            None,
            chrono::Utc::now(),
            &asterism_core::domain::attribution::AttributionContext::asserted(None, None).unwrap(),
        );
        hashable.materials = vec![Material::primary(
            hashable.source.locator.clone(),
            None,
            chrono::Utc::now(),
        )];
        let mut record = Asset::new(
            persona,
            SourceRef::new(kind.clone(), format!("{container}#note-1")).unwrap(),
            None,
            chrono::Utc::now(),
            &asterism_core::domain::attribution::AttributionContext::asserted(None, None).unwrap(),
        );
        record.materials = vec![Material::primary(
            record.source.locator.clone(),
            None,
            chrono::Utc::now(),
        )];
        repo.save(&hashable).await.unwrap();
        repo.save(&record).await.unwrap();

        // Off the backfill scan — the projection with no entity behind
        // it, which is where the two passes could have disagreed.
        let page = repo.scan_unhashed_materials(None, 10).await.unwrap();
        let scanned = |id| {
            page.iter()
                .find(|m| m.asset_id == id)
                .expect("a freshly saved material is work")
                .locator
                .clone()
        };

        let schemed = scanned(hashable.id);
        let opened = schemed
            .local_path()
            .expect("a file:// spelling names a file, and this is that file");
        assert_eq!(
            opened,
            std::path::Path::new(&path),
            "the scheme is gone by the time a path is asked for"
        );
        // And the bytes are actually readable through it — the step
        // that used to fail.
        let fingerprint = hash_artefact(
            &opened.to_string_lossy(),
            Some(&mime("image/png")),
            MAX_CONTENT_WALK_BYTES,
        )
        .expect("the file opens");
        assert!(
            fingerprint
                .file
                .starts_with(asterism_core::domain::content_hash::DIGEST_PREFIX),
            "a real digest, not a marker: {fingerprint:?}"
        );

        // The record has no file of its own, so the marker branch is
        // the right answer — and the container it names *is* a file,
        // which is what stops this from being "everything is refused".
        let addressed = scanned(record.id);
        assert_eq!(addressed.local_path(), None);
        let asterism_core::domain::source_locator::SourceLocator::Record(inner) = &addressed else {
            panic!("a container plus an address is a record: {addressed:?}");
        };
        assert_eq!(
            inner.container().as_path(),
            std::path::Path::new(&container)
        );
        assert_eq!(inner.record().as_str(), "note-1");

        driver.shutdown().await.unwrap();
    }

    #[test]
    fn derive_cover_falls_back_to_file_stem_without_content() {
        assert_eq!(
            derive_cover(CoverTemplate::Dialogue, None, &loc("/tmp/notes/agenda.md")),
            "agenda"
        );
        // A record takes its container's stem — the artefact on disk is
        // the container, and the address is a key, not a name.
        assert_eq!(
            derive_cover(
                CoverTemplate::Dialogue,
                None,
                &loc("/tmp/notes/agenda.md#msg-1")
            ),
            "agenda"
        );
        // A locator with no file behind it has no stem to take, so its
        // own rendering is the name. `file_stem` over this would have
        // cut it at the last dot.
        assert_eq!(
            derive_cover(
                CoverTemplate::Dialogue,
                None,
                &loc("https://host/a/b.png?v=2")
            ),
            "https://host/a/b.png?v=2"
        );
    }

    /// The invariant after asset-model v4 (V38): for the surviving
    /// semantic seed rows, the resolved cover template (override, else
    /// kind default) is work_product / tape special, first-line for
    /// the rest. The old `dialogue` wording is no longer a master
    /// concern — it comes from the structural fallback in `cover_gen`
    /// (textual material inside a container).
    #[tokio::test]
    async fn seed_state_resolves_current_cover_templates() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteModalityRepository::new(isle.clone());

        async fn resolved(repo: &SqliteModalityRepository, slug: &str) -> CoverTemplate {
            let def: ModalityDef = repo
                .find(&Modality::new(slug).unwrap())
                .await
                .unwrap()
                .unwrap();
            def.cover_template.unwrap_or(CoverTemplate::FirstLine)
        }

        assert_eq!(
            resolved(&repo, "work_product").await,
            CoverTemplate::WorkProduct
        );
        assert_eq!(resolved(&repo, "tape").await, CoverTemplate::Tape);
        for slug in ["memory", "state", "emo", "non_rem", "tick_log"] {
            assert_eq!(
                resolved(&repo, slug).await,
                CoverTemplate::FirstLine,
                "seed slug {slug:?} must resolve to the generic template"
            );
        }
        // The conversation slug has left the master (V38).
        assert!(
            repo.find(&Modality::new("dialogue").unwrap())
                .await
                .unwrap()
                .is_none()
        );

        driver.shutdown().await.unwrap();
    }

    /// The master carries no format rows and no format-shaped kinds:
    /// "is this an image?" is the material's mime, answered by
    /// `render_policy`, and the semantic axis is out of that decision
    /// entirely. Filing a PNG under `memory` used to make it
    /// unthumbnailable precisely because the master had a say here.
    #[tokio::test]
    async fn seed_master_carries_no_format_rows_or_kinds() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteModalityRepository::new(isle.clone());

        for gone in ["image", "video", "audio", "dialogue"] {
            assert!(
                repo.find(&Modality::new(gone).unwrap())
                    .await
                    .unwrap()
                    .is_none(),
                "the {gone:?} row left the master in V38"
            );
        }

        // `session` is the exception: V38 deleted it as a *structural*
        // row (`kind = 'composition'`) and V42 brought the slug back as
        // a semantic one. Same word, different axis — what a container
        // holds, not the fact that it is a container.
        let session = repo
            .find(&Modality::new("session").unwrap())
            .await
            .unwrap()
            .expect("V42 re-seeded session as a semantic row");
        assert!(!session.terminal);

        // The master decides one display question, and only the tape
        // answers yes to it. Everything else about how a row draws —
        // thumbnail, player, "is this text" — comes from the material's
        // mime through `render_policy`.
        for slug in ["memory", "state", "emo", "non_rem", "session", "message"] {
            let def = repo
                .find(&Modality::new(slug).unwrap())
                .await
                .unwrap()
                .unwrap();
            assert!(!def.terminal, "{slug:?} is not a terminal transcript");
        }
        let tape = repo
            .find(&Modality::new("tape").unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(tape.terminal);

        driver.shutdown().await.unwrap();
    }
}
