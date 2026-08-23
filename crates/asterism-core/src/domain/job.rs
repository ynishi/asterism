//! `Job` — lifecycle model for asynchronous work.
//!
//! The actual engine lives in `asterism-infra` (apalis + apalis-sql). The
//! domain layer only owns the kind / state / progress vocabulary;
//! scheduling, retry policy, and worker orchestration belong to the engine.
//! Fire-and-forget `tokio::spawn` is intentionally avoided.

use chrono::{DateTime, Utc};

use crate::domain::value::{JobId, Progress};
use crate::error::DomainError;

/// Kinds of background job Asterism runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// Entry point for the asset ingest pipeline (fans out to thumbnail,
    /// auto-tag, index, and edge jobs).
    AssetAdd,
    /// Extracts keywords and materialises channel tags.
    AutoTag,
    /// Generates the card cover using a modality-specific template.
    CoverGen,
    /// Generates a resized thumbnail for an image asset and writes it
    /// into `thumb_cache`. Payload: `{ asset_id, size_px }`. Enqueued
    /// per size at asset-add time (smallest first) so the grid can
    /// paint 128 px quickly while 256 / 512 fill in behind it.
    ThumbGen,
    /// Rebuilds constellation edges for a specific asset.
    EdgeRebuild,
    /// Imports a persona from an external pack or character card.
    PersonaImport,
    /// Rebuilds search / lookup indexes.
    IndexRebuild,
    /// Session reconciliation pass. Enqueued after Import batches and
    /// by explicit user request; payload is empty (`{}`).
    ///
    /// The precomputed rkyv session snapshot this job originally
    /// rebuilt was retired when Session became a 1st-class entity —
    /// the aggregates (`message_count` / `started_at_ms` /
    /// `ended_at_ms`) are derived at query time, so the handler
    /// (`asterism_infra::jobs::handlers::session_rebuild`) currently
    /// only emits the `sessions:progress` broadcast the UI listens
    /// for. It stays wired so every caller keeps succeeding
    /// idempotently and a future write-back pass has a slot.
    SessionRebuild,
    /// Drives one dispatch through its exporter lifecycle
    /// (`dispatch → poll → harvest → reify`). Enqueued by
    /// `DispatchService::create` and re-enqueued by the runner
    /// itself between poll cycles until the state is terminal.
    /// Payload: `{ "dispatch_id": "<uuid>" }`.
    DispatchRun,
    /// Re-evaluates every Query Group under one persona. Fired by the
    /// invalidation hook off every user-facing AssetService write
    /// (asset add / delete, tag attach / detach, manual group
    /// mutation), coarse-grained per persona because the design's
    /// v1 invalidation is persona-scoped. Payload:
    /// `{ "persona_id": "<uuid>" }`. Handler-side per-persona
    /// serialisation prevents overlapping runs; a lightweight
    /// application-layer debounce collapses bursts before enqueue.
    QueryGroupRefresh,
    /// Retention sweep: purges assets and Groups whose trash stamp is
    /// older than the configured retention period. Payload is empty
    /// (`{}`); the cutoff comes from the retention value injected into
    /// `AssetService`, never from the payload, so a stale queued job
    /// cannot purge on yesterday's policy.
    ///
    /// Chain-enqueues itself while a sweep fills its page, so a large
    /// backlog drains without a driver. This is the only scheduled job
    /// that destroys data, which is why it is deliberately the *second*
    /// step of a two-step verb: it can only ever act on rows a user
    /// already put in the trash.
    TrashPurge,
    /// Fingerprints an original's bytes into `material.content_hash`
    /// — the axis duplicate detection groups on.
    ///
    /// Two payload shapes, the same split `IndexRebuild` uses:
    /// `{ "asset_id": "<uuid>" }` hashes one asset's materials (the
    /// ingest fan-out), and `{ "batch": true, "cursor": "<uuid>" }`
    /// walks everything imported before the column existed,
    /// chain-enqueueing the next page while pages come back full.
    ///
    /// Kept off the ingest critical path deliberately: hashing reads
    /// every byte of the original, and an import wave would otherwise
    /// pay for a full re-read of the library before the first card
    /// appears.
    MaterialHash,
    /// Recovers `material.meta_text` — the words a container wrote into
    /// an artefact, read for search rather than for identity
    /// ([`embedded_text`](crate::domain::embedded_text)).
    ///
    /// One payload shape, `{ "batch": true, "cursor": ... }`, and no
    /// per-asset form: nothing asks for one. The ingest path already
    /// fills this column, because the pass that hashes an artefact reads
    /// the bytes once and answers every axis off that one buffer. What
    /// this walk exists for is the library that was imported before the
    /// column existed — a set that is fixed at upgrade time and shrinks
    /// to nothing.
    ///
    /// Its predicate is `meta_text IS NULL` over the formats the
    /// recovery reads, so a row is offered **once**, whatever came back:
    /// `{}` ("read, and these bytes carry no words") retires a row as
    /// firmly as a page of recovered prose does. A startup on a
    /// recovered library therefore costs one query.
    ///
    /// Off the critical path and out of the migration chain for the
    /// reason the two walks above are: it opens files. A migration would
    /// pay that before the application serves anything, and it grows
    /// with the corpus rather than with the change.
    MaterialText,
    /// Measures `asset.width_px` / `height_px`.
    ///
    /// Two payload shapes, the same split
    /// [`MaterialHash`](Self::MaterialHash) uses:
    ///
    /// - `{ "asset_id": "<uuid>" }` — one asset, **because somebody
    ///   asked**. A person who replaced the file behind a card, an agent
    ///   naming rows over HTTP. Overwrites: the request is newer
    ///   information than the stored value.
    /// - `{ "batch": true, "scope": "<scope>", "cursor": "<uuid>" }` — a
    ///   pass over the table, chain-enqueued while pages come back full.
    ///
    /// # The scopes, and which of them ends
    ///
    /// - `unlooked` (default) — `dims_probed_at IS NULL`. **The only one
    ///   that terminates for good**, and the startup seed's reading: a
    ///   row is offered once, whatever came back, so a text note and an
    ///   AVI leave after one look. Selecting on `width_px IS NULL`
    ///   instead would re-offer them on every pass.
    /// - `unmeasured` — `width_px IS NULL`. For when the *situation*
    ///   changed: a volume is mounted now, a parser learned a container.
    ///   Re-offers rows `unlooked` retired, which is the point — the
    ///   stamp says nothing has to look again on its own, not that
    ///   nobody may.
    /// - `all` — every row, and the only scope that overwrites. For when
    ///   the *measurement* changed.
    ///
    /// A read that fails writes nothing at all, so the row stays in
    /// every scope and a later pass reaches it. Recording it would
    /// answer a temporary question permanently — a library on an
    /// external disk, swept once while the disk was out, would never be
    /// measurable again.
    ///
    /// Off the ingest path for the reason the fingerprint walk is: it
    /// opens files. Measures through `asterism-media-probe`, which is
    /// also what the importers measure through — a second implementation
    /// here would put values measured two ways into one column.
    AssetDims,
    /// Re-derives duplicate conflicts from fingerprints that are
    /// **already written**.
    ///
    /// Payload: `{ "batch": true, "cursor": {"asset_id": …, "ord": …} }`,
    /// chain-enqueued while pages come back full. Reads no files — every
    /// digest it compares is already a column — which is why it is its
    /// own job rather than a mode of [`MaterialHash`](Self::MaterialHash).
    ///
    /// # What it is for
    ///
    /// A conflict is derived from a fingerprint at the moment the digest
    /// lands, once. The hashing walk finds work by asking whether the
    /// fingerprint columns are *empty*, so a pair whose moment passed —
    /// the detection landed after the rows did, the lookup errored and
    /// was swallowed, the second side arrived mid-write — is never
    /// looked at again. Measured on a Dogfood profile: 289 fingerprinted
    /// materials, two byte-identical groups, **zero** conflicts.
    ///
    /// Safe to run at any time and as often as wanted:
    /// `duplicate_conflict` is `UNIQUE (pair_lo, pair_hi, axis)` with an
    /// `ON CONFLICT DO NOTHING` insert, so a pair already on the queue is
    /// not duplicated and one a person already answered keeps its
    /// resolution instead of being asked again.
    ///
    /// Runs as [`DetectionOrigin::Backfill`], which is what stops it
    /// folding anything: both rows have been in the library, so even a
    /// lane that asked for `Fold` gets its pair queued for a person.
    ///
    /// [`DetectionOrigin::Backfill`]:
    ///     crate::application_support::duplicate_detection::DetectionOrigin::Backfill
    DuplicateScan,
    /// Folds one asset into another: the losing row becomes a headstone
    /// (`asset.folded_into`) and the structure that hung off it — edges,
    /// Group membership, container children, tag links — moves to the
    /// keeper. Payload:
    /// `{ "asset_id": "<uuid>", "keeper_id": "<uuid>" }`, the row being
    /// folded and the row that stays.
    ///
    /// **Separate from [`MaterialHash`](Self::MaterialHash) on purpose**,
    /// though the fingerprint is what raises the conflict that leads
    /// here. Two reasons, and the first is the
    /// one that would hurt:
    ///
    /// - **There is no retry.** A handler that both wrote the digest and
    ///   folded could fail after the digest landed and before the fold
    ///   finished, and nothing would come back for it: the backfill walk
    ///   selects on `content_hash IS NULL`, so a row that got its hash
    ///   is a row the walk will never look at again. The half-done state
    ///   would be permanent. Split, the hash write is its own completed
    ///   fact and the fold is a job that simply never ran — recoverable
    ///   from the durable rows by whatever raises the conflict next.
    /// - Fingerprinting is a fact about bytes; folding is a decision
    ///   about identity, taken by a person or by the lane's declared
    ///   strategy. A job that reads a file has no business carrying that.
    ///
    /// **What this does not do**: merge the two rows' metadata. Ratings,
    /// descriptions, flags, visibility — none of it moves, and the
    /// keeper's own columns are left exactly as they were. Only the
    /// structure that pointed at the losing row is re-pointed. The
    /// field-by-field merge rules belong to a later wave.
    AssetFold,
    /// Removes observations past their stream's declared retention.
    ///
    /// The windows live in `STREAM_REGISTRY` beside the streams they
    /// govern, so this job carries no policy of its own — it is the
    /// clock, not the rule. Like [`Self::TrashPurge`] it chain-enqueues
    /// while a pass fills its page, and for the same reason: these
    /// tables share the one SQLite connection everything else uses, so
    /// a year of perf rows must not leave in a single statement.
    ObservationSweep,
    /// Derives `material_series` keys: applies every registered
    /// [`Strategy`] to every material's `meta_kv` and files what each
    /// rule concluded.
    ///
    /// Two payload shapes, the same split
    /// [`MaterialHash`](Self::MaterialHash) uses:
    ///
    /// - `{ "asset_id": "<uuid>" }` — one asset's materials, enqueued
    ///   when the fingerprint pass writes their `meta_kv`. Without it a
    ///   newly imported file would have no key until the next start.
    /// - `{ "batch": true, "cursor": {"asset_id": …, "ord": …,
    ///   "strategy_id": …} }` — a pass over the `(material, rule)` pairs
    ///   nothing has answered yet, chain-enqueued while pages come back
    ///   full.
    ///
    /// # One predicate covers all three reasons to recompute
    ///
    /// A new material, a newly registered rule and an edited rule look
    /// like three triggers and are one: *a `(material, rule)` pair with
    /// no row*. A new material has no row under any rule, a new rule has
    /// no row for any material, and an edit is expressed by **deleting**
    /// that rule's rows, which turns it into the second case. Nothing
    /// here detects anything; the walk finds the work.
    ///
    /// # Why the walk shrinks, and what breaks it
    ///
    /// **All three outcomes are written, including
    /// [`NotApplicable`](crate::domain::series::SeriesKey::NotApplicable).**
    /// That is what makes the pair answered and keeps it out of the next
    /// page — the rule
    /// [`AssetDims`](Self::AssetDims)'s `unlooked` scope states as "a row
    /// is offered once, whatever came back". An implementation that filed
    /// only derived keys would re-offer every JPEG and every material a
    /// rule declines forever: the walk would never empty, and the chain
    /// would re-enqueue itself for as long as the process lived.
    ///
    /// # Why this is not a mode of [`MaterialHash`](Self::MaterialHash)
    ///
    /// The same judgement recorded for [`AssetDims`](Self::AssetDims) and
    /// [`DuplicateScan`](Self::DuplicateScan), and here it is the
    /// sharpest of the three: **this job reads no files at all.** Every
    /// input is a column — `meta_kv`, `material.mime`, and the rules —
    /// which is the property the [`series`](crate::domain::series)
    /// module doc sells the whole axis on: a Strategy can be rewritten
    /// and a library re-derived without touching a disk. Folded into the
    /// hashing job that budget would be spent by construction, because
    /// that job's unit of work is a file read.
    ///
    /// The two also stop for different reasons and would deadlock each
    /// other's stop condition if they shared one. The hashing walk selects
    /// rows whose fingerprint columns hold no answer, so it is *done* with
    /// a material the moment `meta_kv` lands — while this walk's work
    /// **begins** there, and comes back for the same material whenever a
    /// rule is added. One predicate cannot say both.
    ///
    /// [`Strategy`]: crate::domain::series::Strategy
    SeriesDerive,
    /// Transcodes a video the webview cannot display into a playable
    /// preview rendition (H.264 MP4, capped resolution) cached beside
    /// the profile database.
    ///
    /// The original is never touched — the ledger's asset stays the
    /// asset, the rendition is a disposable derived file, exactly the
    /// thumbnail relationship at video scale (measured 2026-07-31:
    /// WKWebView cannot decode VP9 in the DOM at all, and Matroska is
    /// rejected at the container level, so for those formats a
    /// rendition is the only way the preview ever plays).
    ///
    /// Payload: `{ "asset_id": "<uuid>" }`. On-demand only — enqueued
    /// the first time the detail pane asks for a preview, never as an
    /// import fan-out (a transcode reads and re-encodes every frame,
    /// which is far too heavy to pay speculatively for a whole wave).
    PreviewGen,
    /// Reads a container's own chapter list into the imported structure
    /// layer of each material that can carry one.
    ///
    /// Two payload shapes, the same split
    /// [`MaterialHash`](Self::MaterialHash) uses:
    ///
    /// - `{ "asset_id": "<uuid>" }` — one asset's materials, enqueued by
    ///   the ingest fan-out for the video and audio it imports.
    /// - `{ "batch": true, "cursor": {"asset_id": …, "ord": …} }` — a
    ///   walk over the materials no reading has reached yet,
    ///   chain-enqueued while pages come back full.
    ///
    /// # The band is the stamp
    ///
    /// The walk selects materials with **no imported structure layer**,
    /// and a completed run always leaves one — including for a file that
    /// declares no chapters at all, which is recorded as an *empty* band
    /// rather than as no band (see
    /// [`replace_imported_chapters`](crate::application_support::replace_imported_chapters)).
    /// So the predicate empties the way
    /// [`AssetDims`](Self::AssetDims)'s `unlooked` scope does, and for
    /// the same reason: a material is offered once, whatever came back.
    /// Selecting on "has no chapters" instead would re-offer every
    /// chapterless clip on every pass, and each offer spawns an ffmpeg.
    ///
    /// No separate stamp column, unlike `dims_probed_at`, because there
    /// is a row here to carry the fact: the band exists or it does not.
    /// A column would be a second answer to a question the table
    /// already answers.
    ///
    /// A file that could not be read is the deliberate exception:
    /// nothing is written, so the material stays in the walk. Leaving a
    /// band behind for a file nobody managed to open would record
    /// "scanned, declares nothing" about bytes that were never seen —
    /// answering a temporary question (an unmounted volume, a file mid-
    /// move) permanently, which is the judgement
    /// [`AssetDims`](Self::AssetDims) also records.
    ///
    /// # Why this is not a mode of [`AssetDims`](Self::AssetDims)
    ///
    /// Both open files, so the split is not about I/O. It is that they
    /// write different aggregates under different rules: a dimension is
    /// a column on the asset, overwritten in place, while a chapter list
    /// is a band of rows whose contents are replaced wholesale and whose
    /// neighbours — the bands a person wrote — must not move. One
    /// handler over both would be one retry policy over two kinds of
    /// loss.
    ChapterScan,
    /// Writes AI-disclosure provenance into a file this library
    /// produced. Payload: `{ "asset_id": "<uuid>" }`.
    ///
    /// # Why this is its own job and not part of the export
    ///
    /// The disclosure is derived from stored container metadata, and
    /// that metadata is written by
    /// [`MaterialHash`](Self::MaterialHash). A dispatch mints its
    /// outputs with no fingerprint at all — `reify` builds the material
    /// from the exporter's string and enqueues the hashing — so a stamp
    /// taken at export time reads an empty evidence set, establishes
    /// nothing, and writes nothing. It would run, succeed, and leave
    /// every file unmarked.
    ///
    /// So the order is a chain rather than a hope: the hashing job
    /// enqueues this one after the fingerprint lands, which is the
    /// first moment there is anything to disclose.
    ///
    /// # Why not a mode of `MaterialHash`
    ///
    /// They fail differently and are retried differently. Hashing reads
    /// bytes and writes a column; stamping rewrites the user's file. A
    /// hashing failure should be retried freely, and a stamping failure
    /// leaves an artefact that exists and is unmarked — a state to
    /// record, not an error to retry into. One handler over both would
    /// be one retry policy over two kinds of loss.
    ///
    /// # The slug was `provenance_stamp` for one commit
    ///
    /// Renamed with the rest of the feature, once `provenance` stopped
    /// meaning two things. A slug is a stored value and renaming one is
    /// normally a migration; this one is safe to rename outright
    /// because the kind has never been in a release, so the only rows
    /// that can carry the old spelling are jobs queued on a development
    /// machine in the hours since it was added. An unknown slug is
    /// skipped rather than fatal (`parse` refuses it, the dispatcher
    /// reports "unknown job kind skipped"), and the cost of one skipped
    /// row is one artefact that stays unmarked until something
    /// re-fingerprints it.
    DisclosureStamp,
    /// Encodes an image's pixels into the stored feature vector the
    /// visual layer (#112) retrieves by.
    ///
    /// Two payload shapes, the same split
    /// [`MaterialHash`](Self::MaterialHash) uses:
    /// `{ "asset_id": "<uuid>" }` encodes one asset (the ingest
    /// fan-out), and `{ "batch": true }` walks image materials with no
    /// stored answer under the configured model, chain-enqueueing
    /// itself while pages come back full. The batch form is seeded by
    /// model installation, not by startup: with no model configured
    /// there is nothing to walk, and the handler skips rather than
    /// failing so an ingest enqueued before a model exists costs one
    /// settled row.
    ///
    /// Off the ingest critical path for the reason the fingerprint
    /// walk is: it opens and decodes the original. A completed
    /// encode chain-enqueues [`VisualEdgeRebuild`](Self::VisualEdgeRebuild)
    /// for the same asset.
    VisualFeature,
    /// Recomputes the visual-similarity edges of one asset from stored
    /// feature vectors — the visual counterpart of
    /// [`EdgeRebuild`](Self::EdgeRebuild), owning
    /// `visual_synth_kinds` and nothing else.
    ///
    /// Payload: `{ "asset_id": "<uuid>" }`. The scan is the whole
    /// persona's vectors under the current model, deliberately not the
    /// ±48 h candidate window; only the bounded top set above the
    /// score floor is materialised, and a scan that clears nothing
    /// writes an empty set rather than padding.
    VisualEdgeRebuild,
    /// Proposes channel tags for one encoded image: the asset's stored
    /// vector against every Tag name's cached text embedding, writing
    /// scored `suggested` evidence — never a tag link; a person's
    /// acceptance is what writes `asset_tag`.
    ///
    /// Two payload shapes: `{ "asset_id": "<uuid>" }` (chained from a
    /// completed encode) and `{ "batch": true }` walking encoded
    /// vectors the pass has not stamped. The pass stamps whether or
    /// not anything cleared the floor, so a scene with no matching
    /// vocabulary is offered exactly once, and it inserts only where
    /// no evidence row exists — a person's ruling is out of its reach
    /// by construction.
    VisualTagSuggest,
    /// Trains the tag head from the person's own rulings (#132 phase
    /// 2): every accepted / rejected suggestion is a labeled example,
    /// the asset's **cached** vector is the input, and the output is a
    /// per-tag logistic row — CPU seconds, never a re-encode.
    ///
    /// Payload: `{}`. The corpus is whatever rulings exist under the
    /// bound encoder's identity; there is nothing to scope.
    ///
    /// Nothing is promoted on faith: each trainable tag holds out part
    /// of its rulings, the candidate and the zero-shot baseline are
    /// scored on the same held-out set, and the run promotes only on a
    /// strict win — a losing run still writes its artifact and report,
    /// because "zero-shot is still better" is a result, not a failure.
    /// Promotion is a pointer move. The scoring side — the follow-up
    /// branch — will read it once at startup, the encoder's bind-once
    /// rule; until that lands the pointer records the verdict and the
    /// zero-shot pass keeps scoring.
    HeadTrain,
}

impl JobKind {
    /// Slug representation shared by the DB schema and DTOs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AssetAdd => "asset_add",
            Self::AutoTag => "auto_tag",
            Self::CoverGen => "cover_gen",
            Self::ThumbGen => "thumb_gen",
            Self::EdgeRebuild => "edge_rebuild",
            Self::PersonaImport => "persona_import",
            Self::IndexRebuild => "index_rebuild",
            Self::SessionRebuild => "session_rebuild",
            Self::DispatchRun => "dispatch_run",
            Self::QueryGroupRefresh => "query_group_refresh",
            Self::TrashPurge => "trash_purge",
            Self::MaterialHash => "material_hash",
            Self::MaterialText => "material_text",
            Self::AssetDims => "asset_dims",
            Self::DuplicateScan => "duplicate_scan",
            Self::AssetFold => "asset_fold",
            Self::ObservationSweep => "observation_sweep",
            Self::SeriesDerive => "series_derive",
            Self::PreviewGen => "preview_gen",
            Self::ChapterScan => "chapter_scan",
            Self::DisclosureStamp => "disclosure_stamp",
            Self::VisualFeature => "visual_feature",
            Self::VisualEdgeRebuild => "visual_edge_rebuild",
            Self::VisualTagSuggest => "visual_tag_suggest",
            Self::HeadTrain => "head_train",
        }
    }

    /// Parses a slug (unknown values yield a validation error).
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "asset_add" => Ok(Self::AssetAdd),
            "auto_tag" => Ok(Self::AutoTag),
            "cover_gen" => Ok(Self::CoverGen),
            "thumb_gen" => Ok(Self::ThumbGen),
            "edge_rebuild" => Ok(Self::EdgeRebuild),
            "persona_import" => Ok(Self::PersonaImport),
            "index_rebuild" => Ok(Self::IndexRebuild),
            "session_rebuild" => Ok(Self::SessionRebuild),
            "dispatch_run" => Ok(Self::DispatchRun),
            "query_group_refresh" => Ok(Self::QueryGroupRefresh),
            "trash_purge" => Ok(Self::TrashPurge),
            "material_hash" => Ok(Self::MaterialHash),
            "material_text" => Ok(Self::MaterialText),
            "asset_dims" => Ok(Self::AssetDims),
            "duplicate_scan" => Ok(Self::DuplicateScan),
            "asset_fold" => Ok(Self::AssetFold),
            "observation_sweep" => Ok(Self::ObservationSweep),
            "series_derive" => Ok(Self::SeriesDerive),
            "preview_gen" => Ok(Self::PreviewGen),
            "chapter_scan" => Ok(Self::ChapterScan),
            "disclosure_stamp" => Ok(Self::DisclosureStamp),
            "visual_feature" => Ok(Self::VisualFeature),
            "visual_edge_rebuild" => Ok(Self::VisualEdgeRebuild),
            "visual_tag_suggest" => Ok(Self::VisualTagSuggest),
            "head_train" => Ok(Self::HeadTrain),
            other => Err(DomainError::Validation(format!(
                "unknown job kind: {other:?}"
            ))),
        }
    }
}

/// Lifecycle state for a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobState {
    /// Enqueued, not yet picked up.
    Pending,
    /// Currently executing on a worker.
    Running,
    /// Completed successfully.
    Completed,
    /// Failed; the engine decides whether to retry.
    Failed,
    /// Cancelled — paired with the SQLite interrupt semantics from the
    /// adapter layer.
    Cancelled,
}

impl JobState {
    /// Slug representation shared by the DB schema and DTOs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parses a slug (unknown values yield a validation error).
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(DomainError::Validation(format!(
                "unknown job state: {other:?}"
            ))),
        }
    }
}

/// Domain model for a single background job.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    /// Surrogate id (UUID v7).
    pub id: JobId,
    /// Job kind.
    pub kind: JobKind,
    /// Lifecycle state.
    pub state: JobState,
    /// Latest progress; the `ProgressEmitter` forwards it to the UI.
    pub progress: Progress,
    /// Kind-specific parameters (opaque to the engine).
    pub payload: serde_json::Value,
    /// When the job was enqueued.
    pub created_at: DateTime<Utc>,
    /// When its state was last updated.
    pub updated_at: DateTime<Utc>,
}

impl Job {
    /// Creates a new job in `Pending` state.
    pub fn new(kind: JobKind, payload: serde_json::Value) -> Self {
        let now = Utc::now();
        Self {
            id: JobId::new(),
            kind,
            state: JobState::Pending,
            progress: Progress::default(),
            payload,
            created_at: now,
            updated_at: now,
        }
    }
}
