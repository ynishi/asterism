//! Command DTOs — inputs for state-changing operations.
//!
//! Callers (Tauri command handlers, MCP tool handlers, HTTP endpoints)
//! build these as JSON; the application services in `asterism-core`
//! convert them into domain types and surface any validation failures as
//! `DomainError::Validation`. This crate only defines the shapes.

use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

/// Auto-organises existing assets under a Dir tree derived from
/// each asset's `source_locator` parent path. When `base_dir` is
/// set, only locators that start with it are affected and the tree
/// is built from the components *after* the prefix — this matches
/// the drag-drop story where the folder the user dropped becomes
/// the tree root, or the HTTP call where the caller passes an
/// explicit root. When `base_dir` is `None`, the tree is derived
/// from every component of the locator's parent.
///
/// A single Group named `<joined path>` is auto-created inside the
/// leaf Dir (Group names are `(persona, name)` unique globally, so
/// the joined path is used verbatim to avoid collisions between
/// two different folder trees whose leaves happen to share a name),
/// and every matched asset is idempotently added to that Group.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct OrganizeByLocationCommand {
    /// When set, only assets owned by this persona are organised.
    /// `None` = every persona.
    pub persona_id: Option<String>,
    /// Path prefix to strip from every `source_locator` before the
    /// Dir tree is built. Locators that don't start with this
    /// prefix are ignored.
    pub base_dir: Option<String>,
}

/// Result summary of an `organize-by-location` run.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct OrganizeByLocationResult {
    /// Dirs that had to be created (existing dirs get reused).
    pub dirs_created: u64,
    /// Groups that had to be created.
    pub groups_created: u64,
    /// Assets successfully attached to a Group.
    pub assets_organized: u64,
    /// Assets that were skipped — usually because the locator did
    /// not match `base_dir` or had no parent path.
    pub skipped: u64,
}

/// Registers a new persona in Asterism.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RegisterPersonaCommand {
    /// Display name.
    pub name: String,
    /// Optional natural key from an external persona pack (unique when
    /// present).
    pub pack_id: Option<String>,
}

/// Toggles a persona's archive flag (a soft-delete alternative).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ArchivePersonaCommand {
    /// Target persona id (UUID hyphenated).
    pub persona_id: String,
    /// `true` = archive, `false` = restore.
    pub archived: bool,
}

/// Moves a persona — and every asset it holds — to the trash.
///
/// Reversible: the assets are stamped with the persona's own trash
/// timestamp, so [`RestorePersonaCommand`] brings back exactly this set
/// while leaving anything the user trashed separately where it is.
/// Nothing is destroyed until the retention sweep or an explicit
/// [`PurgePersonaCommand`].
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TrashPersonaCommand {
    /// Target persona id.
    pub persona_id: String,
}

/// Returns a trashed persona and the assets that went to the trash with
/// it. Assets trashed individually stay trashed.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RestorePersonaCommand {
    /// Target persona id.
    pub persona_id: String,
}

/// Permanently deletes an **already-trashed** persona. Irreversible, and
/// the widest destructive verb in the system: the DB cascade takes every
/// asset of that persona (with their tags, comments, group filings, body
/// text and thumbnails), plus its Groups, snapshots and dispatch
/// history.
///
/// Rejected with a conflict when the persona is still live.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PurgePersonaCommand {
    /// Target persona id.
    pub persona_id: String,
}

/// What the caller wants done if the asset being registered turns out
/// to hold bytes an existing asset already holds.
///
/// The wire half of `asterism_core::domain::value::OnDuplicate`, which
/// is the type the row is written from. Two declarations of the same
/// closed set rather than one, because this crate is a leaf — it has
/// no dependency on `asterism-core` (that arrow points the other way),
/// and it is the crate the TypeScript bindings and the MCP tool schemas
/// are generated from. The `snake_case` tokens are `ask` / `fold` /
/// `separate` on both sides, and the single conversion between them
/// (`AssetService::add`) is an exhaustive match, so a variant added to
/// either set stops compiling until the other is answered for.
///
/// Spelled as an enum rather than a free string — unlike `author_kind`,
/// the neighbouring caller-asserted field — because the set really is
/// closed and small: a caller reading the generated schema sees the
/// three answers instead of guessing, and an unknown token is refused
/// by deserialisation, above every side effect the ingest could have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum OnDuplicate {
    /// Register normally and put the match in front of a person to
    /// confirm.
    Ask,
    /// Fold into the existing asset without asking.
    Fold,
    /// Keep both rows; only record that the bytes matched.
    Separate,
}

/// Ingests a single asset — entry point for the asset-add pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct AddAssetCommand {
    /// Persona bucket (required).
    pub persona_id: String,
    /// Ingest source slug (`fs`, `persona-pack`, and so on).
    pub source_kind: String,
    /// Location of the original artefact.
    pub locator: String,
    /// Semantic classification slug (`state`, `tape`, and so on).
    /// `None` = unclassified — the normal value for conversation
    /// messages (their structure is carried by `external_session_key`
    /// / `session_id`, their format by the material layer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modality: Option<String>,
    /// Occurrence time in unix epoch milliseconds.
    pub occurred_at_ms: i64,
    /// **Session.id UUID direct** — the surrogate id of an existing
    /// `session` row. Callers that already know the Session (server-
    /// side scripts, re-runs that carry the id verbatim) use this
    /// field; importers should leave it `None` and hand the raw key
    /// through `external_session_key` instead. session-model v2: the
    /// value is the composite Asset id and lands on `asset.container_id`
    /// (the wire name stays `session_id` for back-compat). Membership
    /// is modality-agnostic (asset-model v4) — any asset may bind.
    /// `session_id` and `external_session_key` are mutually
    /// exclusive — supplying both is a `Validation` error (matrix
    /// ambiguity guard).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// **Importer-supplied external key** (Claude Code session UUID,
    /// JSONL file stem, …). The server resolves this to a composite
    /// Asset id via `SessionService::find_or_create_by_external_key`,
    /// so the resulting member asset lands with
    /// `asset.container_id = <composite Asset id>`. Membership is
    /// modality-agnostic. Mutually exclusive with `session_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_session_key: Option<String>,
    /// **What the source calls this row** — an issue key, a row's
    /// primary key, an upstream API's id. Lands verbatim on
    /// `asset.external_key`.
    ///
    /// External linkage, and nothing else: it is there so something
    /// outside Album can find its way back to a row. Nothing about
    /// matching or minting reads it. It carries no uniqueness and cannot
    /// — an external record legitimately arrives more than once, and two
    /// platforms both numbering a record `12345` is ordinary (V62 took
    /// the last UNIQUE off the column).
    ///
    /// Not to be confused with `external_session_key` above, which names
    /// a *container* the row joins and is resolved to a composite id.
    /// This one names the row itself and is stored as stated.
    ///
    /// `None` = the source states no id of its own, which is most rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_key: Option<String>,
    /// Constellation-edge grouping key for assets that are not members
    /// of a Session container (tape / journal / image / future slot).
    /// Members carry `container_id` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    /// Free-form labels (status hints, secondary modality tags, and so on).
    pub labels: Vec<String>,
    /// Register / tone annotation.
    pub register_note: Option<String>,
    /// Originating platform (human-readable name).
    pub platform: Option<String>,
    /// Original artefact size.
    pub file_size_bytes: Option<u64>,
    /// Duration for time-bounded assets.
    pub duration_ms: Option<u64>,
    /// Pixel width of the **stored bytes** — the coded dimension, before
    /// any orientation is applied. Not the width a viewer displays.
    ///
    /// This is what the parsers actually measure: the image importer
    /// takes EXIF dims or the decoded header and reads `orientation`
    /// separately without applying it, so a photo tagged Orientation 5-8
    /// (90° transpose) arrives here as the landscape pair even though it
    /// displays portrait. A reader that wants display dimensions has to
    /// combine this with `extra.orientation`.
    ///
    /// `None` = nobody measured it, and it is never `0` — a zero would
    /// sort ahead of every measured value on an ascending axis, the same
    /// reason `duration_ms` stays absent rather than reading as zero.
    ///
    /// Paired with `height_px`: **one without the other is a half-written
    /// pair and a `Validation` error**, on the same terms as
    /// `author_kind` / `author_subject`. Both fields default on the wire,
    /// so `{"width_px": 1920}` deserialises to `(Some, None)` and is
    /// refused by `AssetService::add` rather than landing as half a
    /// resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_px: Option<u32>,
    /// Pixel height of the stored bytes, on exactly the terms
    /// [`AddAssetCommand::width_px`] states — coded, orientation not
    /// applied, absent rather than zero, and refused when it arrives
    /// without its partner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_px: Option<u32>,
    /// Source-specific extension bag, serialised as a JSON string.
    pub extra_json: Option<String>,
    /// Optional cover text supplied by the ingest side. When present it
    /// is stored verbatim on the asset and the server-side `cover_gen`
    /// job skips over it. Lets importers that already know the display
    /// text (for example the first user prompt of a Claude Code
    /// session) avoid the generic filesystem-heuristic fallback.
    pub cover_hint: Option<String>,
    /// If set, run `organize_by_location` (idempotent) for this
    /// asset's persona with this `base_dir` immediately after the
    /// save. The importer can therefore "add + file" in one call
    /// instead of a follow-up backfill sweep. `None` (default)
    /// skips the sweep — legacy callers see no change.
    #[serde(default)]
    pub auto_organize_base_dir: Option<String>,
    /// Declared origin of this artefact — `asset:<uuid>`,
    /// `dispatch:<uuid>` or `sidecar`
    /// ([`ProvenanceRef`][provenance-note]).
    ///
    /// This is how a file that left Asterism, went through an outside
    /// generator, and came back gets reconnected to what it was made
    /// from: the output is a new file and carries nothing of its
    /// parent, so the caller that ran the chain declares the link here
    /// and the server writes a `derived_from` edge.
    ///
    /// A claim that cannot be resolved does **not** fail the ingest —
    /// it is recorded on the asset's `extra._trace` so the artefact
    /// still lands and the broken link stays visible.
    ///
    /// The arrival channel is bookkept on the note as `_trace.source`,
    /// derived from the claim's form rather than asserted here: a
    /// `sidecar` claim is the importer reporting what it found next to
    /// the file (`embedded`), anything else came with this payload
    /// (`pushed`). After-the-fact declarations through
    /// `DeclareProvenanceCommand` record `manual`.
    ///
    /// [provenance-note]: `asterism_core::domain::provenance::ProvenanceRef`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<String>,
    /// Attribution kind for the subject this asset is by — `"owner"`
    /// (no `author_subject`) or `"subject"` (one required). Any other
    /// pair is rejected as a `Validation` error rather than degrading
    /// into a plausible-looking value.
    ///
    /// **Caller-asserted**, the write-side counterpart of the
    /// `viewer_subject` contract the read surface already carries: the
    /// loopback port authenticates nobody, so the server records what
    /// the caller stated. Authentication is a hosted-time transport
    /// concern; when it arrives the handler starts overriding or
    /// rejecting the assertion and this shape does not move.
    ///
    /// `None` = **unrecorded**, never "the owner" — see
    /// [`Author`][author-note].
    ///
    /// Value domain on this face: `"owner" | "subject"`. `"persona"` is
    /// rejected — a persona is a voice and a filing membership, not a
    /// subject a write is attributed to. Two other commands carry a
    /// field of this same name over a **different** domain
    /// (`PostAssetCommentCommand`: `"user" | "persona"`;
    /// `AppendMessageCommand`: `"human" | "claude_code" | "agent" |
    /// "persona"`); the values are not interchangeable between them.
    ///
    /// [author-note]: `asterism_core::domain::attribution::Author`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_kind: Option<String>,
    /// Subject token when `author_kind = "subject"` — the same token
    /// `viewer_subject` and the restricted-sharing list carry. Must be
    /// absent for `"owner"`, and supplying one without a kind is a
    /// half-written pair and a `Validation` error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_subject: Option<String>,
    /// Agent that performed this ingest (`claude-code`, `codex`,
    /// `asterism-ui`, an importer's own name) — an open slug, since the
    /// set of things that can drive Asterism is not closed. Blank is
    /// rejected: an empty assertion must not be storable as one that
    /// says something.
    ///
    /// Separate from `author_*` because one subject drives Asterism
    /// through several agents, and "through which" is the question an
    /// audit of an agent-run library actually asks. Caller-asserted on
    /// the same terms. `None` = unrecorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
    /// What to do if this asset's fingerprint later matches an existing
    /// one — see [`OnDuplicate`].
    ///
    /// Declared here and stored on the row, because the match cannot be
    /// known yet: `add` returns without reading the file, and the
    /// fingerprint is computed afterwards by a background job. By the
    /// time there is a conflict to resolve, this command is gone.
    ///
    /// `None` = **unrecorded**, and deliberately not `ask`. The design
    /// resolves an undeclared registration
    /// against an importer / lane setting and then a persona default;
    /// **neither of those layers is implemented** — this field is the
    /// whole declaration surface today, so an undeclared row currently
    /// falls to the single built-in default. Storing `ask` here would
    /// make an unanswered registration indistinguishable from one that
    /// asked for confirmation, and only the first is free to pick up a
    /// lane default later.
    ///
    /// Nothing acts on the value yet: detection, the conflict queue and
    /// the fold verb are later subtasks. What this field buys now is
    /// that a declaration made at registration is still there when they
    /// arrive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_duplicate: Option<OnDuplicate>,
    /// What the caller says this artefact's bytes hash to —
    /// `sha256:<64 lowercase hex>`.
    ///
    /// **A claim, not a fingerprint.** The server recomputes the digest
    /// from the file regardless, stores *that*, and compares. A match
    /// is recorded; a mismatch is recorded with both values side by
    /// side and logged. Neither outcome deletes, rejects or quarantines
    /// anything: the bytes on disk are the fact, and a caller that
    /// disagrees with them has said something worth keeping rather than
    /// something worth acting on.
    ///
    /// The name is `declared_` for that reason. Calling it
    /// `content_hash` would put a caller-supplied string one assignment
    /// away from the column that means "we read these bytes", and the
    /// single mistake this whole path exists to prevent is that
    /// assignment — a declaration accepted as the digest also satisfies
    /// the per-asset job's "already fingerprinted" test, so the file
    /// would never be read at all.
    ///
    /// **It does not make anything faster.** A declaration cannot
    /// settle a duplicate (the value alone
    /// never decides a fold), and the one step a hint could skip —
    /// reading the file — is the step that checks it. So there is no
    /// fast path here to take: the field buys integrity, and integrity
    /// only.
    ///
    /// Only the file axis is accepted. `cr1-sha256:` (content axis) is
    /// refused for now — nothing computes it, so the claim would sit
    /// unchecked and indistinguishable from a confirmed one — as is any
    /// other tag, a bare hex string and a blank. The rule and its
    /// reasons are
    /// [`content_hash::parse_declaration`][declaration-note]; the
    /// refusal is a `Validation` error, and since the field is optional
    /// the retry is the same request without it.
    ///
    /// `None` = **unrecorded**: the caller asserted nothing, which is
    /// the normal case and not the same as asserting agreement.
    ///
    /// [declaration-note]: `asterism_core::domain::content_hash::parse_declaration`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_content_hash: Option<String>,
    /// AlbumMeta statements to file on the row at registration — what
    /// the caller *says about* this artefact, keyed by names the caller
    /// chose.
    ///
    /// The after-the-fact counterpart is [`DeclareAssetMetaCommand`],
    /// and the domain module (`asterism_core::domain::album_meta`)
    /// carries the argument for the whole shape: why a statement is
    /// neither a tag nor a column, and why an identifier that arrives on
    /// an artefact is recorded as something somebody said rather than
    /// becoming a key.
    ///
    /// This is the field an importer that read a value **out of the
    /// source** uses — a generator's own reference, a workflow id, a
    /// catalogue number. Recording it here keeps it findable without
    /// letting it decide which rows are the same row.
    ///
    /// Keys and values are checked on the same terms as the declaration
    /// verb (lowercase / digits / `_` / `-`; no blank value), and a
    /// refusal fails the whole ingest rather than dropping the entry:
    /// silently landing an asset without the statement that made it
    /// findable is the outcome this shape exists to prevent.
    ///
    /// There is no removal spelling here and no channel to choose. A
    /// registration has nothing to retract, and everything on this
    /// command arrived with the payload — so each entry is stamped
    /// `pushed`. A statement dug back out of the artefact by a reader
    /// is a different path and will stamp `embedded` when it lands.
    ///
    /// Empty = nobody said anything, which is the normal case.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub album_meta: std::collections::BTreeMap<String, String>,
}

/// Rewrites `display_order` across a persona slice. `ordered_ids`
/// is the front-to-back list the sidebar is about to render; the
/// server sets each persona's `display_order` to its index so the
/// order survives across sessions.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ReorderPersonasCommand {
    /// Persona ids in the desired sidebar order (front → back).
    pub ordered_ids: Vec<String>,
}

/// Writes a clipboard-pasted image blob to disk and dispatches
/// `add_asset` in one shot. `bytes` is the raw payload (PNG /
/// JPEG); `mime_type` is the clipboard hint (`image/png`, …) and
/// picks the file extension so downstream `image::open` on the
/// stored file can rely on the container. The saved locator is
/// echoed back through `AssetDto.locator`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PasteImageImportCommand {
    /// Persona bucket the pasted image lands in.
    pub persona_id: String,
    /// Raw image bytes from the clipboard.
    pub bytes: Vec<u8>,
    /// Clipboard MIME hint (`image/png`, `image/jpeg`, …). Used
    /// only to pick the on-disk file extension; the importer
    /// pipeline still sniffs the container itself.
    pub mime_type: String,
}

/// How a person answered one raised duplicate question.
///
/// The wire half of
/// `asterism_core::domain::duplicate_conflict::ConflictResolution`, two
/// declarations of one closed set for the reason [`OnDuplicate`]
/// records about its own pair.
///
/// Two values because a person looking at a pair has two things to say.
/// There is deliberately no "skip" / "later": leaving the question
/// unanswered is what the queue already represents, and spelling it as
/// an answer would close a row that nobody decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    /// One thing. The pair is folded: the row named by `keeper_id`
    /// stays and absorbs the other, which becomes a headstone.
    Folded,
    /// Two things. Both rows stay exactly as they are; only the
    /// question is closed.
    Kept,
}

/// Answers one duplicate question — the panel's confirm.
///
/// Writes the answer onto the queue row and, for `folded`, enqueues the
/// fold. Nothing is deleted either way: a closed row is the record that
/// the question was raised and ruled on, and it is also what keeps the
/// same pair from being asked again.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct ResolveDuplicateConflictCommand {
    /// The queue row to answer (`DuplicateConflictDto::id`).
    pub conflict_id: String,
    /// The answer.
    pub resolution: ConflictResolution,
    /// Which of the two rows stays. **Required for `folded`, and
    /// rejected for `kept`.**
    ///
    /// Named by the caller rather than derived, because deriving it is
    /// exactly what did not happen: age picks the keeper for an
    /// automatic fold, and this row is on the queue because that choice
    /// was handed to a person. A default here would quietly hand it
    /// back at the one moment somebody is making it — and an agent that
    /// omitted the field would never learn which row it had just turned
    /// into a headstone.
    ///
    /// Must be one of the pair; any other id is refused rather than
    /// folding two assets the queue row does not name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keeper_id: Option<String>,
}

/// A person's ruling that a set of rows is one thing — the manual
/// merge verb's input.
///
/// The other entry point to a fold is the queue answer above: a pair
/// the fingerprint match raised, kept in
/// [`ResolveDuplicateConflictCommand`] because every field of that
/// command is keyed to the pair the queue row was raised on. This is
/// what reaches the fold from somewhere the queue does not: a person
/// looking at several rows in a panel and declaring them one thing.
///
/// The shape mirrors the domain type that checks it
/// (`asterism_core::domain::merge_plan::MergePlan`): the keeper and the
/// rows folded into it *and* the whole set the ruling was made over.
/// The last field is not decoration; without it the caller has said
/// two different things — "the rows I did not tick are separate" and
/// "I did not notice them" — with the same call, and the check that
/// splits them cannot be added after the fact (the rows a person saw
/// stop being knowable the moment the call is made). The `MergePlan`
/// port doc is the authoritative account of what it refuses and why;
/// this command's job is to carry the same three lists across the wire.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct MergeAssetsCommand {
    /// The row that stays — `MergePlan::keeper`. It is not derived
    /// (from age, from listing order, from anything) because the person
    /// looking at the panel picked it, and picking it is what the queue
    /// answer above records for a two-row conflict and this command
    /// records for a set of any size.
    pub keeper_id: String,
    /// Rows folded into the keeper, in the order they will be folded —
    /// `MergePlan::discard`. The order is the caller's and reads out in
    /// `register_note` paragraphs and the keeper's `_trace.absorbed`
    /// entries afterwards; the port doc records why sorting here would
    /// replace an order somebody chose with one nobody did.
    pub discard_ids: Vec<String>,
    /// **Every row the person is ruling over**, keeper and discard
    /// together, exactly the set they saw on screen.
    /// [`MergePlan::declare`](../../asterism_core/domain/merge_plan/struct.MergePlan.html#method.declare)
    /// refuses the call unless this is `keeper_id ∪ discard_ids` (each
    /// id once). Named to split "leave the others alone" from "I did
    /// not notice them", which without this field are the same call.
    pub member_ids: Vec<String>,
    /// `true` = preview only, no writes. `false` = commit.
    ///
    /// A single field rather than two verbs because the preview is a
    /// prediction of *this* call — the counts come from the statements
    /// that would have been kept, not from a second implementation —
    /// and a run following a preview reads the answer back on the same
    /// shape. See [`MergeAssetsDto::committed`](crate::dto::MergeAssetsDto::committed)
    /// for how the two are told apart on the way back.
    #[serde(default)]
    pub dry_run: bool,
}

/// Ingests many assets in one call — the batched form of
/// [`AddAssetCommand`], meant for importers walking large source dirs.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AddAssetBatchCommand {
    /// Assets to ingest. The server processes them independently: any
    /// individual failure is reported through [`AddAssetBatchResult`]
    /// without aborting the rest.
    ///
    /// Everything an item can declare travels on the item, including
    /// [`AddAssetCommand::on_duplicate`] — there is no batch-level
    /// duplicate strategy on purpose. "One setting for this whole
    /// import run" is the lane layer, which
    /// is unimplemented; adding it here would be that layer, built in
    /// the one place that cannot express a lane that spans more than a
    /// single call.
    pub items: Vec<AddAssetCommand>,
    /// If set, run one `organize_by_location` sweep after all items
    /// have been ingested (persona filter defaults to `None` so
    /// every persona touched by the batch is covered). Batch-level
    /// sweep is far cheaper than per-item ones — the Dir / Group
    /// caches inside `organize_by_location` amortise across the
    /// whole run instead of re-listing on every asset. Per-item
    /// `auto_organize_base_dir` values on the individual
    /// `AddAssetCommand`s are ignored while this batch-level field
    /// is set so we don't sweep 89 k times back-to-back.
    #[serde(default)]
    pub auto_organize_base_dir: Option<String>,
}

/// Result of a batched ingest. `succeeded` and `failed` are sized to
/// match the request; each entry corresponds to the same index in
/// [`AddAssetBatchCommand::items`].
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AddAssetBatchResult {
    /// Ingested asset ids (empty string when the corresponding item
    /// failed — see `failed[i]` for the reason).
    pub succeeded: Vec<String>,
    /// Per-item failure messages (empty string when the item succeeded).
    pub failed: Vec<String>,
    /// Total number of items that landed successfully.
    pub success_count: u64,
    /// Total number of items that failed.
    pub failure_count: u64,
}

/// Partially updates asset metadata (`None` leaves the field unchanged).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct UpdateAssetMetaCommand {
    /// Target asset id.
    pub asset_id: String,
    /// Replacement for the label list (`None` = leave unchanged).
    pub labels: Option<Vec<String>>,
    /// Replacement register / tone annotation.
    pub register_note: Option<String>,
    /// Manual override for the card cover text (useful when the auto-gen
    /// result needs a human tweak).
    pub cover: Option<String>,
    /// Hand-given name (`None` = leave unchanged, `Some("")` clears it
    /// back to unnamed). Distinct from `cover`: a cover is derived text
    /// a job can regenerate, a title is what a person decided to call
    /// this. It matters most for containers, which own no body to
    /// derive a cover from and would otherwise borrow their first
    /// member's — a session called "msg-1".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Star rating 0-5 (`None` on the wire = leave unchanged). To
    /// clear a rating pass `0` (or wire a separate `clear_rating`
    /// bool if the caller needs the tri-state).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    /// Replacement primary modality slug (`None` = leave unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modality: Option<String>,
    /// Replacement constellation-edge grouping key (`None` = leave
    /// unchanged). Passed through to the persistence layer verbatim;
    /// the service does not validate the modality invariant here
    /// (P3 importers own the routing between `session_id` and
    /// `bundle_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
}

/// Partially updates metadata for multiple assets in one call.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct UpdateAssetMetaBatchCommand {
    /// Items to update. The server processes them independently.
    pub items: Vec<UpdateAssetMetaCommand>,
}

/// Result of a batched metadata update.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct UpdateAssetMetaBatchResult {
    /// Updated asset DTOs (null when the corresponding item failed).
    pub succeeded: Vec<Option<crate::dto::AssetDto>>,
    /// Per-item failure messages (empty string when succeeded).
    pub failed: Vec<String>,
    /// Total number of items updated successfully.
    pub success_count: u64,
    /// Total number of items that failed.
    pub failure_count: u64,
}

/// Moves an asset to the trash — reversible, and the only route to
/// [`PurgeAssetCommand`].
///
/// Nothing is destroyed: the asset keeps its tags, group filing and
/// hand-arranged order, comments, body text, thumbnails, and snapshot
/// membership while it sits in the trash, so
/// [`RestoreAssetCommand`] brings all of it back. The asset simply
/// stops appearing in listings, counts, and search.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TrashAssetCommand {
    /// Target asset id.
    pub asset_id: String,
}

/// Returns a trashed asset to the live set. Idempotent.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RestoreAssetCommand {
    /// Target asset id.
    pub asset_id: String,
}

/// Permanently deletes an **already-trashed** asset. Irreversible:
/// `asset_tag`, edges, thumbnails, group filing, body text, snapshot
/// membership, and comments all go with it via the DB foreign-key
/// cascade. Everything Asterism knew about the asset is gone — the
/// original file on disk is untouched, but re-importing it produces a
/// fresh asset with none of that value.
///
/// Rejected with a conflict when the asset is still live: purge is
/// reachable only through [`TrashAssetCommand`], so a bulk caller
/// always leaves a recoverable intermediate state.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PurgeAssetCommand {
    /// Target asset id.
    pub asset_id: String,
}

/// Permanently deletes **every** asset currently in the trash — the
/// bulk form of [`PurgeAssetCommand`], and irreversible for the same
/// reasons.
///
/// Deliberately takes no filter. The trash is one place, not a view of
/// the library: a user emptying it means "everything I threw away is
/// gone", and honouring the grid's active filter here would leave
/// rows behind that the confirmation prompt just said would go. Live
/// assets are never in scope — the command reaches only rows that
/// already carry a trash stamp, so it inherits the same
/// "trash first, purge second" guarantee.
///
/// No `SchemaBridge` derive, unlike every sibling here: the exported
/// schema is built from the field list, and a struct with no fields
/// gives the macro nothing to infer from. The emptiness *is* the
/// contract, so the fix is not to invent a field — the frontend sends
/// `{}` and reads [`EmptyTrashResult`], which is exported.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmptyTrashCommand {}

/// Outcome of [`EmptyTrashCommand`].
///
/// `skipped` is not an error channel the caller has to act on: the
/// realistic cause is a restore landing between the scan and the
/// purge, which comes back as a conflict. Reported rather than
/// propagated so one recovered asset cannot cancel the rest of the
/// sweep — the same trade the retention sweep makes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, SchemaBridge)]
pub struct EmptyTrashResult {
    /// Assets permanently deleted by this call.
    pub purged: u64,
    /// Assets that were in the trash at scan time but could not be
    /// purged (logged with the reason on the server).
    pub skipped: u64,
}

/// Enqueues an incremental constellation-edge rebuild for the asset.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RebuildEdgesCommand {
    /// Target asset id.
    pub asset_id: String,
}

/// Declares (or repairs) the origin of an asset that is already in
/// the library — the after-the-fact twin of
/// [`AddAssetCommand::derived_from`].
///
/// Exists because a claim can outlive its first chance to resolve:
/// the artefact landed before its dispatch finished, the sidecar
/// arrived later, or nobody knew the parent at ingest time. Same
/// vocabulary (`asset:<uuid>`, `dispatch:<uuid>`, `sidecar`), same
/// behaviour on failure — the claim is recorded on `extra._trace`
/// rather than rejected, so it stays repairable.
///
/// Claims through this verb are bookkept as `_trace.source:
/// "manual"` — this *is* the after-the-fact declaration channel, as
/// opposed to `embedded` / `pushed` which only ingest-time claims
/// ([`AddAssetCommand::derived_from`]) can carry.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeclareProvenanceCommand {
    /// Target asset id.
    pub asset_id: String,
    /// The origin claim, in `derived_from` syntax.
    pub derived_from: String,
    /// What the claim asserts: `"derived_from"` (this came out of
    /// that) or `"reference"` (this was made with that in view).
    ///
    /// `None` means `derived_from`, which is what every claim written
    /// before this field existed meant — so an old caller keeps its
    /// exact behaviour and an old `_trace` note keeps its exact
    /// reading.
    ///
    /// The distinction is not decoration. `derived_from` is the
    /// assertion that one artefact came out of another, and nothing in
    /// the corpus can contradict it later; a person who worked from two
    /// references has said something weaker and true, and without this
    /// field the only ways to record it were to overstate it or to lose
    /// it. Unknown values are refused rather than defaulted — the
    /// default is the stronger of the two, and a typo must not promote
    /// a claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
    /// Agent that made this after-the-fact declaration
    /// (`claude-code`, `codex`, `asterism-ui`, …). Recorded on the
    /// `_trace` note as `operator`, alongside the `source: "manual"`
    /// this channel stamps — the two answer different questions
    /// (*through what* the repair came, and *which channel* the claim
    /// arrived on).
    ///
    /// Caller-asserted, like `AddAssetCommand::operator_ai`. `None`
    /// leaves the field off the note entirely rather than guessing —
    /// an unrecorded operator is not "the person at the keyboard".
    ///
    /// No author counterpart: the subject a repair is *by* is not the
    /// subject the asset is by, and overwriting the latter from a
    /// repair verb would rewrite authorship as a side effect of
    /// fixing a link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// Records — or removes — one AlbumMeta statement on an asset.
///
/// AlbumMeta is what a person or an agent *says about* an asset under a
/// name they chose, as opposed to what an importer read out of the
/// source. The domain module
/// (`asterism_core::domain::album_meta`) carries the argument for why
/// it is neither a tag nor a column, and why an external identifier
/// arriving on an artefact is recorded here rather than becoming a key.
///
/// Statements are single-slot: declaring the same `key` twice leaves
/// the later one. That is the difference from a provenance claim, which
/// is append-only because each one draws an edge — a second statement
/// under one name is a correction, and keeping both would leave a
/// reader to guess which is current.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct DeclareAssetMetaCommand {
    /// Target asset id.
    pub asset_id: String,
    /// The name the statement is filed under. Lowercase letters,
    /// digits, `_` and `-`; refused otherwise (see the domain module on
    /// why `.` in particular cannot be allowed).
    pub key: String,
    /// What is being said. `None` **removes** the key.
    ///
    /// Removal is a separate meaning from an empty string, which is
    /// refused: a caller that sends `""` has almost always failed to
    /// build the value it meant to send, and recording that as a
    /// retraction would delete a statement on the strength of a bug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Agent this statement came through (`claude-code`, `codex`,
    /// `asterism-ui`, …), recorded on the entry as `operator`.
    ///
    /// Caller-asserted and optional, on the same terms as
    /// [`DeclareProvenanceCommand::operator_ai`]: absent means nobody
    /// stated one, not that a person was at the keyboard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// Requests cancellation of a running or pending job.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CancelJobCommand {
    /// Target job id.
    pub job_id: String,
}

/// Creates a new user-curated Group (bucket) under a persona.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateGroupCommand {
    /// Owner persona id.
    pub persona_id: String,
    /// Group name — unique per persona.
    pub name: String,
    /// Optional freeform description.
    pub description: Option<String>,
}

/// Moves a Group to the trash — reversible, and the only route to
/// [`PurgeGroupCommand`].
///
/// The `asset_bucket` rows stay, so the membership **and its
/// hand-arranged order** survive; the Group just stops appearing in the
/// sidebar and stops contributing to filters. Member assets are never
/// touched — trashing a Group discards a filing, not the things filed.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TrashGroupCommand {
    /// Target group id.
    pub group_id: String,
}

/// Returns a trashed Group to the sidebar, membership and drag order
/// intact. Idempotent.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RestoreGroupCommand {
    /// Target group id.
    pub group_id: String,
}

/// Permanently deletes an **already-trashed** Group and every
/// `asset_bucket` row that referenced it. Irreversible: the name, the
/// Dir filing, and the drag-arranged member order are gone. Member
/// assets survive — they are simply no longer filed here.
///
/// Rejected with a conflict when the Group is still live.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PurgeGroupCommand {
    /// Target group id.
    pub group_id: String,
}

/// Adds an asset to a Group. Idempotent — a duplicate insert is a
/// no-op.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AddAssetToGroupCommand {
    /// Asset to add.
    pub asset_id: String,
    /// Target group.
    pub group_id: String,
}

/// Removes an asset from a Group. Idempotent — missing link is a
/// no-op.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RemoveAssetFromGroupCommand {
    /// Asset to remove.
    pub asset_id: String,
    /// Source group.
    pub group_id: String,
}

/// One asset↔group pair inside a `BatchGroupMembershipCommand`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct GroupMembershipEntry {
    /// Asset side of the link.
    pub asset_id: String,
    /// Group side of the link.
    pub group_id: String,
}

/// Applies a batch of group-membership changes in one call: every
/// `attach` pair is linked (idempotent) and every `detach` pair
/// unlinked (missing link = no-op). The bulk primitive behind AI /
/// script membership cleanup — one round-trip
/// instead of N single-pair calls. Manual groups only; query-group
/// membership is defined by its stored query.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct BatchGroupMembershipCommand {
    /// Pairs to link.
    pub attach: Vec<GroupMembershipEntry>,
    /// Pairs to unlink.
    pub detach: Vec<GroupMembershipEntry>,
}

/// Merges one Group into another (duplicate-group consolidation):
/// members of `from_group_id` that the target
/// lacks are appended after its tail, then the source group is
/// deleted. Both groups must be manual and belong to the same
/// persona.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MergeGroupsCommand {
    /// Group to dissolve.
    pub from_group_id: String,
    /// Group that absorbs the members.
    pub into_group_id: String,
}

/// Rewrites the front-to-back order of a Group. `ordered_asset_ids` is
/// the full sequence in the new order; any id not currently in the
/// group is silently skipped (the client sends the snapshot it drew,
/// and drift against server state should not fail the write).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ReorderGroupAssetsCommand {
    /// Target group.
    pub group_id: String,
    /// Full ordered sequence of asset ids (front-to-back).
    pub ordered_asset_ids: Vec<String>,
}

/// Renames a Group. The name stays unique per persona.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RenameGroupCommand {
    /// Target group.
    pub group_id: String,
    /// New name.
    pub name: String,
}

/// Files a Group under a Dir (`None` = back to the root level).
/// Organisation axis only — the group's members and nesting links
/// are untouched.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MoveGroupToDirCommand {
    /// Target group.
    pub group_id: String,
    /// Destination dir; `None` = root.
    pub dir_id: Option<String>,
}

/// Connects a Group into another Group (the Are.na channel-in-channel
/// gesture). Idempotent; rejected when it would close a cycle or
/// cross personas.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct LinkGroupCommand {
    /// The containing group.
    pub parent_group_id: String,
    /// The group being connected in.
    pub child_group_id: String,
}

/// Removes a Group-in-Group connection. Idempotent — a missing link
/// is a no-op.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct UnlinkGroupCommand {
    /// The containing group.
    pub parent_group_id: String,
    /// The group being disconnected.
    pub child_group_id: String,
}

/// Rewrites the order of a Group's child groups. Same drift-tolerant
/// contract as [`ReorderGroupAssetsCommand`].
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ReorderGroupChildrenCommand {
    /// The containing group.
    pub parent_group_id: String,
    /// Full ordered sequence of child group ids (front-to-back).
    pub ordered_child_ids: Vec<String>,
}

/// Creates a sidebar Dir under a persona (`parent_id = None` = root).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateDirCommand {
    /// Owner persona id.
    pub persona_id: String,
    /// Parent dir; `None` = root level.
    pub parent_id: Option<String>,
    /// Dir name — unique among siblings.
    pub name: String,
}

/// Renames a Dir. The name stays unique among siblings.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RenameDirCommand {
    /// Target dir.
    pub dir_id: String,
    /// New name.
    pub name: String,
}

/// Re-parents a Dir (`None` = to the root). Rejected when the target
/// parent sits inside the moved dir's own subtree.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MoveDirCommand {
    /// Target dir.
    pub dir_id: String,
    /// New parent; `None` = root.
    pub new_parent_id: Option<String>,
}

/// Deletes an **empty** Dir. Rejected while the dir still contains
/// child dirs or groups.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteDirCommand {
    /// Target dir.
    pub dir_id: String,
}

/// Upserts the identity signal for a persona (avatar / bio /
/// role). Every optional field is a full-replace: passing `None`
/// clears the stored value; omitting the field on the wire is not
/// supported (the client always sends the full desired state). To
/// remove the whole row use `DeletePersonaProfileCommand`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SetPersonaProfileCommand {
    /// Target persona.
    pub persona_id: String,
    /// Portrait / thumbnail asset id. `None` clears the avatar.
    pub avatar_asset_id: Option<String>,
    /// One-line asterism-internal bio. `None` clears.
    pub bio_short: Option<String>,
    /// Free-form role tag chip. `None` clears.
    pub role_tag: Option<String>,
}

/// Removes the persona profile row entirely (reverts the sidebar
/// card to the plain name + accent color).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeletePersonaProfileCommand {
    /// Target persona.
    pub persona_id: String,
}

/// Sets (or clears) the wallpaper for a persona.
///
/// `wallpaper_asset_id = None` clears the wallpaper while leaving the
/// theme row intact so subsequent decoration fields inherit it. To
/// remove the theme row entirely (revert to "no custom theme, use
/// built-in defaults") use `DeletePersonaThemeCommand`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SetPersonaThemeCommand {
    /// Target persona.
    pub persona_id: String,
    /// Image asset used as the wallpaper. `None` clears the wallpaper
    /// but keeps the theme row alive.
    pub wallpaper_asset_id: Option<String>,
}

/// Removes the persona theme row entirely, reverting the UI to
/// built-in defaults for that persona.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeletePersonaThemeCommand {
    /// Target persona.
    pub persona_id: String,
}

/// Creates a Modality master row (`POST /asterism/modalities`).
///
/// `slug` is validated by the same `[a-z0-9_-]{1,64}` grammar as
/// `Modality`; a duplicate slug surfaces as `409 Conflict`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateModalityCommand {
    /// New modality slug (primary key).
    pub slug: String,
    /// Display name.
    pub label: String,
    /// Whether assets of this classification read as terminal
    /// transcripts. The one display question the semantic axis still
    /// decides — everything else comes from the material's mime.
    #[serde(default)]
    pub terminal: bool,
    /// Sidebar sort rank.
    pub sort_order: i64,
    /// Whether the modality starts hidden (defaults to `false` when
    /// omitted).
    #[serde(default)]
    pub hidden: bool,
    /// Optional cover-template slug (`None` = the generic first-line
    /// template).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_template: Option<String>,
}

/// Partially updates a Modality master row
/// (`PATCH /asterism/modalities/{slug}`). Every field is `None =
/// leave unchanged`, mirroring [`UpdateAssetMetaCommand`]. The target
/// `slug` is supplied by the path; the body `slug` field is ignored
/// (defaulted for convenience).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct UpdateModalityCommand {
    /// Target modality slug (populated from the request path).
    #[serde(default)]
    pub slug: String,
    /// Replacement display name (`None` = unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Replacement terminal-reading flag (`None` = unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<bool>,
    /// Replacement sort rank (`None` = unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<i64>,
    /// Replacement hidden flag (`None` = unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// Replacement cover-template override slug (`None` = unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_template: Option<String>,
}

/// Deletes a Modality master row
/// (`DELETE /asterism/modalities/{slug}`). Rejected with `409
/// Conflict` when any asset still carries the slug (no orphaning
/// delete); the operational retirement path is the `hidden` flag.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteModalityCommand {
    /// Target modality slug.
    pub slug: String,
}

/// Registers a series Strategy (`POST /asterism/series-strategies`).
///
/// The rule crosses a process boundary as data — an importer, or the
/// agent driving one, runs in its own process and can register a rule
/// but cannot ship a decoder — so `decode` names one of the tokens this
/// build already carries and an unknown one is refused rather than read
/// as "do not decode".
///
/// No `id` field: the identity is a surrogate the server mints, and the
/// response carries it. What derived keys are filed under is therefore a
/// value the caller learns rather than one it asserts.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateSeriesStrategyCommand {
    /// Display name. Not read by the derivation.
    pub name: String,
    /// The one media type this rule claims (`image/png`). One type and
    /// no wildcard: a rule reads one generator's habits inside one
    /// container.
    pub applies_to: String,
    /// Decoder token: `none` / `raw_json` / `base64_json` / `exif`.
    pub decode: String,
    /// Sub-trees to keep, each a path whose first segment is a keyword
    /// of the container's metadata. Omitted or empty selects the whole
    /// of it, which is what makes an exclude-only rule expressible.
    #[serde(default)]
    pub include: Vec<Vec<String>>,
    /// Sub-trees to drop, applied after `include` and rooted the same
    /// way.
    #[serde(default)]
    pub exclude: Vec<Vec<String>>,
}

/// Partially updates a series Strategy
/// (`PATCH /asterism/series-strategies/{id}`). Every field is `None =
/// leave unchanged`, mirroring [`UpdateModalityCommand`]. The target id
/// is supplied by the path; the body `id` field is ignored.
///
/// **Four of the five fields invalidate.** `applies_to`, `decode`,
/// `include` and `exclude` are what the derivation reads, so changing
/// any of them makes every key already derived under this rule a key
/// nothing would derive again; `name` is a label and moves nothing. The
/// service compares the four and only invalidates when one actually
/// differs, so renaming a rule does not cost a re-derivation of the
/// library.
///
/// The id is not among them. It is what derived rows are filed under —
/// a rule that could change it would strand its own keys.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct UpdateSeriesStrategyCommand {
    /// Target Strategy id (populated from the request path).
    #[serde(default)]
    pub id: String,
    /// Replacement display name (`None` = unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Replacement media type (`None` = unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<String>,
    /// Replacement decoder token (`None` = unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decode: Option<String>,
    /// Replacement include list (`None` = unchanged; `Some([])` = select
    /// the whole of the container's metadata).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<Vec<String>>>,
    /// Replacement exclude list (`None` = unchanged; `Some([])` = drop
    /// nothing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<Vec<String>>>,
}

/// Deletes a series Strategy
/// (`DELETE /asterism/series-strategies/{id}`).
///
/// No guard, unlike [`DeleteModalityCommand`]: the keys derived under
/// the rule go with it (the schema cascades), and they cost a scan of
/// rows already in hand to rebuild rather than a pass over anybody's
/// disk. A modality slug cannot be recomputed, which is why that one is
/// refused while assets still carry it.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteSeriesStrategyCommand {
    /// Target Strategy id.
    pub id: String,
}

/// Attaches a tag to an asset by name (creates the tag if it does
/// not exist yet, idempotent on both the tag row and the m:n link).
///
/// Reads through `TagRepository::find_or_create` + `link`, so the
/// same call is safe to fire from the manual chip editor and the
/// auto-tag pipeline without extra guards.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AttachTagCommand {
    /// Target asset.
    pub asset_id: String,
    /// Tag name — trimmed and shared across personas. A new tag row
    /// is materialised on the first attach and reused thereafter.
    pub name: String,
}

/// Detaches a tag from an asset. Idempotent — a missing m:n link is
/// a no-op. The tag row is left in place so other assets keep it;
/// tags with zero attached assets drop out of the sidebar via the
/// `tag_counts` query (dead channels never surface).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DetachTagCommand {
    /// Target asset.
    pub asset_id: String,
    /// Tag to remove from the asset.
    pub tag_id: String,
}

/// Attaches one tag to many assets in one call — the batched form of
/// [`AttachTagCommand`], meant for the grid multi-select action bar.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AttachTagBatchCommand {
    /// Items to attach. The server processes them independently: any
    /// individual failure is reported through [`AttachTagBatchResult`]
    /// without aborting the rest.
    pub items: Vec<AttachTagCommand>,
}

/// Result of a batched tag attach. Shape mirrors
/// [`UpdateAssetMetaBatchResult`] — each index lines up with the
/// same index in [`AttachTagBatchCommand::items`].
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AttachTagBatchResult {
    /// Attached tag DTOs (null when the corresponding item failed).
    pub succeeded: Vec<Option<crate::dto::TagDto>>,
    /// Per-item failure messages (empty string when succeeded).
    pub failed: Vec<String>,
    /// Total number of items attached successfully.
    pub success_count: u64,
    /// Total number of items that failed.
    pub failure_count: u64,
}

/// Detaches one tag from many assets in one call — the batched form of
/// [`DetachTagCommand`].
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DetachTagBatchCommand {
    /// Items to detach. The server processes them independently.
    pub items: Vec<DetachTagCommand>,
}

/// Result of a batched tag detach. `detach` carries no payload, so the
/// per-item `succeeded` slot is a plain flag; the counts / `failed`
/// vector keep the same shape as the other batch results.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DetachTagBatchResult {
    /// Per-item success flag (`false` when the corresponding item
    /// failed — see `failed[i]` for the reason).
    pub succeeded: Vec<bool>,
    /// Per-item failure messages (empty string when succeeded).
    pub failed: Vec<String>,
    /// Total number of items detached successfully.
    pub success_count: u64,
    /// Total number of items that failed.
    pub failure_count: u64,
}

/// Renames a tag in place (`POST /asterism/tags/rename`).
///
/// The new name goes through the same normalisation as
/// [`AttachTagCommand`] (trim; empty is rejected), so a name minted
/// here and a name minted by an attach cannot drift apart.
///
/// **Renaming never merges.** When the normalised name already
/// belongs to a *different* tag the call is rejected with `409
/// Conflict` and the caller is pointed at [`MergeTagsCommand`] —
/// collapsing two channels destroys one of them, which is an
/// explicit gesture, not a side effect of a typo fix. Renaming a tag
/// to the name it already carries succeeds as a no-op.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RenameTagCommand {
    /// Target tag.
    pub tag_id: String,
    /// New channel name (trimmed; must be non-empty and unused by
    /// any other tag).
    pub name: String,
}

/// Deletes a tag row and every `asset_tag` link that referenced it
/// (`POST /asterism/tags/delete`).
///
/// Unlike [`DetachTagCommand`] — which unlinks one asset and leaves
/// the channel standing — this removes the channel itself. There is
/// no trash for tags: the row and its links go in one transaction.
/// The assets are untouched beyond losing the link.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteTagCommand {
    /// Tag to delete. Unknown ids are rejected with `404`.
    pub tag_id: String,
}

/// Result of a [`DeleteTagCommand`].
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteTagResult {
    /// Always `true` — the failure paths return an error status
    /// instead. Present so a caller can assert on the body rather
    /// than on an empty 200.
    pub deleted: bool,
    /// Number of `asset_tag` links removed with the tag. Counts link
    /// rows, so trashed assets are included (their links survive
    /// trashing by design); this is deliberately *not* the live
    /// count `tags/counts` reports.
    pub detached_assets: u64,
}

/// Merges one tag into another and deletes the source
/// (`POST /asterism/tags/merge`) — the repair verb for the synonym /
/// spelling-variant sprawl an automatic tagger produces at scale.
///
/// Every asset carrying `source_tag_id` ends up carrying
/// `target_tag_id`; assets that already carried the target keep a
/// single link (the duplicate is dropped, not doubled). The source
/// row is then removed. All of it happens in one transaction.
///
/// The target's `axis` survives untouched — merge folds the source
/// *into* the target, so the target's classification is the one that
/// was chosen to keep. A source axis that disagreed is discarded with
/// the source row.
///
/// A Query Group rule that names the source tag by id is **not**
/// rewritten (same treatment as group merge): its refresh runs, finds
/// no assets under the now-absent id, and the group's membership goes
/// empty. Re-save the rule against the target to follow the merge.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MergeTagsCommand {
    /// Tag to dissolve. Must differ from `target_tag_id`.
    pub source_tag_id: String,
    /// Tag that absorbs the source's assets and survives.
    pub target_tag_id: String,
    /// When `true`, compute the outcome and roll back: nothing is
    /// written, the numbers in [`MergeTagsResult`] are the ones the
    /// real call would produce, and `source_removed` comes back
    /// `false`. Merge is not undoable, so an agent about to fold two
    /// channels of unknown size can look before it leaps.
    #[serde(default)]
    pub dry_run: bool,
}

/// Result of a [`MergeTagsCommand`].
///
/// `affected_assets + already_tagged` is the number of assets the
/// source carried. Both count link rows, so trashed assets are
/// included — same rule as [`DeleteTagResult::detached_assets`].
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MergeTagsResult {
    /// Assets moved onto the target (they did not carry it before).
    pub affected_assets: u64,
    /// Assets that already carried the target, so the source's link
    /// was dropped rather than moved.
    pub already_tagged: u64,
    /// Whether the source tag row was actually deleted (`false` for
    /// a `dry_run`).
    pub source_removed: bool,
}

/// Promotes a Tag into a hand-curated Group: creates a new Group
/// under the given persona (and optionally files it under a Dir),
/// then attaches every asset currently carrying the tag to the new
/// Group in one call.
///
/// This is the primary curation gesture behind an Are.na-style "turn
/// this channel into a bucket I own": the underlying tag is left
/// untouched (auto-tag can keep growing it), while the Group starts
/// off as a hand-arrangeable snapshot the user takes ownership of.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PromoteTagToGroupCommand {
    /// Source tag whose currently-linked assets seed the new group.
    pub tag_id: String,
    /// Owner persona of the new group.
    pub persona_id: String,
    /// Human-facing group name. Must be unique per persona.
    pub name: String,
    /// Optional freeform description passed through to the group.
    pub description: Option<String>,
    /// Optional sidebar dir the new group is filed under.
    pub dir_id: Option<String>,
}

/// Result of a `PromoteTagToGroupCommand` — echoes the group that was
/// materialised and the number of assets attached, so the UI can
/// flash a `▤ N assets → <group>` toast without a follow-up query.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PromoteTagToGroupResult {
    /// The newly-created group.
    pub group_id: String,
    /// The persona the group belongs to.
    pub persona_id: String,
    /// Group name (echo of the command for convenience).
    pub name: String,
    /// Number of assets attached to the new group.
    pub asset_count: u64,
}

/// Freezes a picked asset list as an immutable `Snapshot` — internal
/// materialise input only (no public command surface mints a
/// Snapshot directly; the dispatch / promote paths are the callers).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateSnapshotCommand {
    /// Persona bucket. Every id in `asset_ids` must belong to this
    /// persona (enforced server-side).
    pub persona_id: String,
    /// Asset ids in pick order; frozen as the Snapshot membership.
    /// Non-empty; the server rejects an empty vector.
    pub asset_ids: Vec<String>,
}

/// Promotes a frozen `Snapshot` into a hand-owned `Group`
/// ("Promote snapshot to Group").
///
/// Creates a new Group under the Snapshot's persona, bulk-attaches the
/// frozen membership in order, and stamps the Group's
/// `origin_snapshot_id` birth record. Name uniqueness follows the
/// (persona, name) unique constraint on `bucket`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PromoteSnapshotToGroupCommand {
    /// Source Snapshot.
    pub snapshot_id: String,
    /// Human-facing group name (unique per persona).
    pub name: String,
    /// Optional freeform description passed through to the Group.
    pub description: Option<String>,
    /// Optional sidebar dir the new Group is filed under.
    pub dir_id: Option<String>,
}

/// Result of a `PromoteSnapshotToGroupCommand` (also returned by the
/// fused `PromoteVolatileSelectionCommand`).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PromoteSnapshotToGroupResult {
    /// The newly-created Group id.
    pub group_id: String,
    /// The Snapshot that seeded it (echo for convenience).
    pub snapshot_id: String,
    /// Group name (echo).
    pub name: String,
    /// Number of assets attached to the new Group.
    pub asset_count: u64,
}

/// Fuses "freeze the volatile grid pick" + "promote to Group" into a
/// single command (right-click "Group-ify selection", W5-d): mints a Snapshot
/// from `asset_ids` (content-hash deduped — an identical pick reuses
/// the existing freeze) and materialises a hand-owned Group from
/// it, stamping `origin_snapshot_id` as the birth record.
///
/// If the promote half fails (e.g. group-name conflict) the minted
/// Snapshot stays behind — harmless by design: a Snapshot is a nameless
/// content object and the orphan is the later GC job's concern.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PromoteVolatileSelectionCommand {
    /// Persona bucket. Every id in `asset_ids` must belong to this
    /// persona (enforced server-side — a Snapshot cannot span personas).
    pub persona_id: String,
    /// Asset ids in pick order; frozen as the Snapshot membership.
    /// Non-empty; the server rejects an empty vector.
    pub asset_ids: Vec<String>,
    /// Human-facing group name (unique per persona).
    pub name: String,
    /// Optional freeform description passed through to the Group.
    pub description: Option<String>,
    /// Optional sidebar dir the new Group is filed under.
    pub dir_id: Option<String>,
}

/// Posts a new comment on an Asset. `author_kind` is `"user"` for
/// the human running Asterism or `"persona"` for one of the vault's
/// Personas (in which case `author_persona_id` is required and
/// must reference an existing persona). `body` is free-form text;
/// blank / whitespace-only bodies are rejected.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct PostAssetCommentCommand {
    /// Target Asset.
    pub asset_id: String,
    /// Value domain on this face: `"user" | "persona"` — a comment
    /// names the voice it is written in, not an attribution. The same
    /// field name carries a **different** domain on the other two faces
    /// (`AddAssetCommand`: `"owner" | "subject"`;
    /// `AppendMessageCommand`: `"human" | "claude_code" | "agent" |
    /// "persona"`); the values are not interchangeable between them.
    pub author_kind: String,
    /// Persona id when `author_kind = "persona"`; `None` for user
    /// posts.
    pub author_persona_id: Option<String>,
    /// Comment body.
    pub body: String,
}

/// Rewrites the body of an existing `AssetComment`. Stamps
/// `edited_at`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct EditAssetCommentCommand {
    /// Owning Asset id (redundant with the lookup but keeps the
    /// wire shape explicit).
    pub asset_id: String,
    /// Target comment.
    pub comment_id: String,
    /// New body.
    pub body: String,
}

/// Deletes an `AssetComment` by id. Idempotent.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteAssetCommentCommand {
    /// Target comment.
    pub comment_id: String,
}

/// Places a new mark into an Asset's material.
///
/// `anchor_kind` names the coordinate space; `"temporal"` is the only
/// one this build places, and it requires `start_ms` (milliseconds from
/// the playback origin, the same zero the player reports). `end_ms` is
/// the exclusive end of an interval — omit it for an instant. The
/// target Asset must be time-bearing: a mark on the timeline of
/// something that has no timeline is refused.
///
/// `author_kind` is `"user"` for the human running Asterism or
/// `"persona"` for one of the vault's Personas (in which case
/// `author_persona_id` is required and must reference an existing
/// persona) — the same value domain [`PostAssetCommentCommand`] uses,
/// and not the one `AddAssetCommand` uses. `body` is free-form text;
/// blank / whitespace-only bodies are rejected.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct PostMaterialMarkCommand {
    /// Target Asset.
    pub asset_id: String,
    /// Coordinate space slug (`"temporal"`).
    pub anchor_kind: String,
    /// Position on the timeline. Required for `"temporal"`; negative
    /// values are rejected.
    pub start_ms: Option<i64>,
    /// Exclusive end of the interval. `None` marks an instant; a value
    /// no greater than `start_ms` is rejected.
    pub end_ms: Option<i64>,
    /// Value domain on this face: `"user" | "persona"` — a mark names
    /// the voice it is written in, not an attribution.
    pub author_kind: String,
    /// Persona id when `author_kind = "persona"`; `None` for user
    /// marks.
    pub author_persona_id: Option<String>,
    /// Mark body.
    pub body: String,
}

/// Rewrites the body of an existing `MaterialMark`. Stamps `edited_at`.
///
/// The anchor is not editable through this command: moving a mark is a
/// different act from rewording one, and no surface asks for it yet.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct EditMaterialMarkCommand {
    /// Owning Asset id (redundant with the lookup but keeps the wire
    /// shape explicit).
    pub asset_id: String,
    /// Target mark.
    pub mark_id: String,
    /// New body.
    pub body: String,
}

/// Deletes a `MaterialMark` by id. Idempotent.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteMaterialMarkCommand {
    /// Target mark.
    pub mark_id: String,
}

/// Opens a band over an Asset's material that the person owns.
///
/// The band is always `origin = "user"`, and the command has no field
/// for that: the other two origins name producers — the material itself
/// and a job — and neither of them arrives through a command. A caller
/// asking for an imported band is asking for the file to be read again,
/// which is a different verb on a different surface.
///
/// The new band is never the default. Moving the flag as a side effect
/// of creation would leave a caller that wanted a second band with the
/// first one no longer shown; [`SetDefaultMaterialLayerCommand`] is how
/// a caller says it meant that.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateMaterialLayerCommand {
    /// Asset whose material the band is over.
    pub asset_id: String,
    /// Which of the asset's originals. `None` is the primary one — the
    /// axis `duration_ms` measures — which is what every surface marks
    /// today. It is a field rather than a constant so that the day a
    /// surface addresses a second original, the caller says which
    /// rather than the server assuming.
    pub material_ord: Option<u32>,
    /// `"structure"` (holds chapters) or `"annotation"` (holds marks).
    pub role: String,
    /// Display order within `(asset_id, material_ord, role)`. The
    /// caller's to choose, as a chapter's `ord` is: the order bands are
    /// offered in is a property of the surface offering them, not
    /// something the server can derive.
    pub ord: u32,
}

/// Chooses the band a surface shows, and the one a new mark lands in.
///
/// Open to every origin, unlike the write verbs: choosing to read the
/// file's own chapter list rather than one's own is not an edit to
/// either. The refusal that does apply is the model's — an annotation
/// band that is not the user's cannot be the default, because a new note
/// would land in a band nobody may write to.
///
/// The call changes **two** rows (the flag moves off whichever band held
/// it), so a caller that displays bands re-reads the asset's list rather
/// than patching one entry.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SetDefaultMaterialLayerCommand {
    /// Band to show.
    pub layer_id: String,
}

/// Deletes a band the person owns, with everything in it.
///
/// Refuses an imported or machine band rather than deleting one: those
/// are reproduced by running their producer again, so the removal would
/// last until the next re-read and then silently undo itself. A verb
/// whose effect ends when something unrelated happens is worse than no
/// verb.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteMaterialLayerCommand {
    /// Target band.
    pub layer_id: String,
}

/// Adds a section to a structure band the person owns.
///
/// `start_ms` is the position on the playback timeline; `end_ms` is the
/// exclusive end, and omitting it declares a section with no stated end
/// (the shape MP4's `chpl` produces) rather than one running to the end
/// of the media. `label` may be empty — plenty of containers declare
/// untitled sections, and this verb accepts what an import accepts.
///
/// Refused against an imported or machine band (not the caller's to
/// write into) and against an annotation band (which holds notes, not
/// sections — post a material mark instead).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PostChapterMarkCommand {
    /// Band to write into.
    pub layer_id: String,
    /// Start of the section, in milliseconds from the presentation
    /// origin. Negative values are rejected.
    pub start_ms: i64,
    /// Exclusive end. `None` states no end; a value no greater than
    /// `start_ms` is rejected.
    pub end_ms: Option<i64>,
    /// Section title. May be empty.
    pub label: String,
    /// Reading order within the band.
    pub ord: u32,
}

/// Rewrites one section of a structure band the person owns.
///
/// Unlike [`EditMaterialMarkCommand`], which rewords without moving,
/// this verb **can** move a section: the reason a person opens a band of
/// their own is usually that the file's divisions are in the wrong
/// places, so correcting a position is the ordinary case rather than a
/// second act.
///
/// The band is named as well as the chapter, matching the
/// `(asset_id, mark_id)` pair on the mark face: the ownership guard is a
/// fact about the parent, and naming it is what lets a caller's own band
/// id fail to reach into another band's row.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct EditChapterMarkCommand {
    /// Band the chapter is in.
    pub layer_id: String,
    /// Target chapter.
    pub chapter_id: String,
    /// New title. May be empty.
    pub label: String,
    /// New start position. **`None` leaves the section where it is**,
    /// and is the only way to say so — `end_ms` below is read only when
    /// this is present, which keeps "do not move it" and "make it a
    /// section with no stated end" two different requests rather than
    /// one absent field.
    pub start_ms: Option<i64>,
    /// New exclusive end, read only when `start_ms` is present. `None`
    /// there states a section with no end.
    pub end_ms: Option<i64>,
    /// New reading order. `None` leaves it alone.
    pub ord: Option<u32>,
}

/// Removes one section from a structure band the person owns.
///
/// Not idempotent, deliberately, where [`DeleteMaterialMarkCommand`] is:
/// a chapter is named by `(layer_id, chapter_id)`, so an id that is not
/// in that band is refused rather than treated as already-gone — without
/// that a caller could not tell "removed" from "was never yours".
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteChapterMarkCommand {
    /// Band the chapter is in.
    pub layer_id: String,
    /// Target chapter.
    pub chapter_id: String,
}

/// Creates a Thread anchored to the given axis.
///
/// `anchor_kind`:
/// - `"app_global"` — the Home-tab Inbox / free-form axis.
///   `anchor_id` must be `None`.
/// - `"snapshot"` — attaches to a Snapshot. `anchor_id` is required
///   and must reference an existing Snapshot.
/// - `"query_group"` — attaches to a query Group. `anchor_id` is
///   required.
/// - `"card"` — attaches to an Asset. `anchor_id` is required.
///
/// `title` is trimmed; a blank / whitespace-only value is rejected.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateThreadCommand {
    /// Anchor kind slug.
    pub anchor_kind: String,
    /// Anchor entity id (required for every kind except
    /// `"app_global"`).
    pub anchor_id: Option<String>,
    /// Display label.
    pub title: String,
}

/// Toggles a Thread's `archived` flag. Idempotent — setting it to
/// its current value returns the row unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ArchiveThreadCommand {
    /// Target Thread.
    pub thread_id: String,
    /// New archived state.
    pub archived: bool,
}

/// Deletes a Thread and every Message attached to it. Idempotent —
/// a missing id is a no-op.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteThreadCommand {
    /// Target Thread.
    pub thread_id: String,
}

/// Appends a Message to a Thread.
///
/// `author_kind` accepts `"human"` / `"claude_code"` / `"agent"` /
/// `"persona"`. `author_name` is required when `author_kind =
/// "agent"` (the agent slug). `author_persona_id` is required when
/// `author_kind = "persona"` (must reference an existing Persona).
///
/// `role`:
/// - `"note"` — plain thought / observation.
/// - `"action"` — record of an action the author performed.
/// - `"system"` — pipeline-emitted event (usually written by the
///   server side rather than caller-side).
///
/// `body` is trimmed; blank bodies are rejected.
///
/// `idempotency_key` is optional. When set, the pair
/// `(thread_id, idempotency_key)` is unique — retrying an append
/// with the same key returns the previously stored Message instead
/// of inserting a duplicate.
///
/// `refs` are reference chips embedded in the body (Phase 4 UI
/// consumers). Each entry carries a kind + entity uuid.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AppendMessageCommand {
    /// Target Thread.
    pub thread_id: String,
    /// Value domain on this face: `"human" | "claude_code" | "agent" |
    /// "persona"` — one enum folding "a person", "which agent" and
    /// "which voice" together, which is a different factoring of the
    /// question from the asset face. The same field name carries a
    /// **different** domain there (`AddAssetCommand`: `"owner" |
    /// "subject"`) and on `PostAssetCommentCommand` (`"user" |
    /// "persona"`); the values are not interchangeable between them.
    pub author_kind: String,
    /// Agent slug when `author_kind = "agent"`; `None` otherwise.
    pub author_name: Option<String>,
    /// Persona id when `author_kind = "persona"`; `None` otherwise.
    pub author_persona_id: Option<String>,
    /// `"note"` / `"action"` / `"system"`.
    pub role: String,
    /// Message body (markdown).
    pub body: String,
    /// Reference chips (may be empty).
    pub refs: Vec<crate::dto::MessageRefDto>,
    /// Optional deduplication key.
    pub idempotency_key: Option<String>,
}

/// Deletes one Message. Idempotent — a missing id is a no-op.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteMessageCommand {
    /// Target Message.
    pub message_id: String,
}

/// Kicks off one exporter invocation against a frozen `Snapshot`.
///
/// The server records a `DispatchJob` row in `pending` state and
/// enqueues an apalis `DispatchRun` task that drives the exporter's
/// `dispatch → poll → harvest` state machine. Callers monitor
/// progress through `GET /asterism/dispatch/{id}` or the
/// `dispatch:progress:{id}` event channel.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateDispatchCommand {
    /// Snapshot whose frozen asset_ids seed this dispatch.
    pub snapshot_id: String,
    /// Registered exporter slug (`comfy` / `gemini` / `vdsl` /
    /// `alc-sd-bake`). Failing to match a registered exporter
    /// returns `Validation`.
    pub exporter_slug: String,
    /// Action string (`img2img` / `txt2img` / `lora_bake` / …). The
    /// server pre-flights against `Exporter::accepts` before
    /// enqueueing.
    pub action: String,
    /// Exporter-specific parameters, JSON-encoded as a string.
    /// `schema-bridge` does not know how to codegen
    /// `serde_json::Value`, so the wire representation is a string
    /// the server parses (empty string = `{}`).
    pub params_json: String,
    /// Agent that started this dispatch (`claude-code`, `codex`,
    /// `asterism-ui`, …). Persisted on the dispatch row, so the
    /// runner — which finishes long after the caller is gone — can
    /// stamp it on the `_dispatch` note and on the `operator_ai`
    /// column of every asset the run reifies.
    ///
    /// Caller-asserted, like `AddAssetCommand::operator_ai`. `None` =
    /// unrecorded. No author counterpart: the subject a reified output
    /// is *by* is a question for the authentication layer this
    /// codebase does not have yet, and the exporter cannot answer it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
    /// Pursuit to file this round under. `None` mints a fresh pursuit
    /// server-side (always-mint: work cannot happen outside a pursuit,
    /// there is no detached state). Continuation is explicit — the
    /// server never infers it from snapshot overlap — so a surface
    /// that wants rounds to correlate has to thread this id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pursuit_id: Option<String>,
}

/// Runs a dispatch from a live source: the input is **either** a Group
/// (manual or query — a query group is refreshed synchronously first
/// so the freeze is always fresh) **or** a volatile grid selection
/// (`asset_ids`). The server freezes the members into a Snapshot
/// (content-hash deduped), stamps the provenance
/// (`source_group_id` / `source_query_json`) on the dispatch row, and
/// enqueues the run.
///
/// Exactly one of `group_id` / `asset_ids` must be provided.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DispatchRunCommand {
    /// Owner persona id.
    pub persona_id: String,
    /// Group to freeze (`None` = volatile-selection dispatch).
    pub group_id: Option<String>,
    /// Volatile selection to freeze, in pick order (empty when
    /// `group_id` is given).
    #[serde(default)]
    pub asset_ids: Vec<String>,
    /// Registered exporter slug.
    pub exporter_slug: String,
    /// Action string pre-flighted by the runner.
    pub action: String,
    /// Exporter-specific parameters, JSON-encoded (empty = `{}`).
    pub params_json: String,
    /// Agent that started this dispatch — same contract and same
    /// downstream stamping as [`CreateDispatchCommand::operator_ai`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
    /// Pursuit to file this round under — same contract as
    /// [`CreateDispatchCommand::pursuit_id`] (`None` mints).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pursuit_id: Option<String>,
}

/// Re-runs a finished dispatch with the same frozen input, exporter,
/// action, and params (P2). The snapshot row is shared — the
/// content hash already deduped it — so history grows by one job row
/// only.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RedispatchCommand {
    /// The dispatch to re-run.
    pub dispatch_id: String,
    /// Pursuit to file the re-run under. `None` continues the prior
    /// dispatch's pursuit — not an inference: the caller named the
    /// prior round literally, and a re-run is a new round of the same
    /// line of work (the new-patchset-on-the-same-change shape).
    /// Supply an id to file it elsewhere instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pursuit_id: Option<String>,
}

/// Opens a pursuit explicitly — the "start new pursuit" affordance
/// (#29). Optional: a dispatch arriving unstamped mints one anyway
/// (always-mint); pre-creating simply lets the caller name the intent
/// up front. An empty pursuit that never receives work is an honest
/// record, closable as abandoned.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct OpenPursuitCommand {
    /// Owner persona id.
    pub persona_id: String,
    /// Id to create the pursuit at, instead of a minted one. The
    /// repair case the server cannot solve alone: an artefact came
    /// back naming a pursuit that does not exist here (a sidecar
    /// written on another machine, a restore that outran its
    /// pursuit), the claim was recorded unresolved, and creating the
    /// pursuit under that exact id is what lets the re-resolve sweep
    /// join the two. `None` mints one, which is the ordinary path.
    /// Taken as given, never checked for meaning: an id already in use
    /// is refused as a conflict rather than merged into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pursuit_id: Option<String>,
    /// Pursuit this one is spawned from — set at creation, immutable,
    /// same persona. `None` for a root pursuit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_pursuit_id: Option<String>,
    /// Short human label; provenance of intent, not state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// One short free-text slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Caller-asserted operator slug, like
    /// [`CreateDispatchCommand::operator_ai`]. Recorded on the pursuit
    /// row so a line of work an agent opened says so; omitting it
    /// records nothing rather than the owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// Closes a pursuit (#29, verdicts on #22): records a one-way
/// lifecycle fact, never a status write. A repeat close is a new fact
/// — standing re-derives from the latest event.
///
/// A `satisfied` close is where the cull is recorded: the candidate
/// set is derived from the pursuit's own ledger (never supplied
/// here), frozen into a snapshot, and the verdicts below are resolved
/// against it — a member removed mid-work and not spoken for culls as
/// `reject`, an untouched member without a verdict gets no row, and
/// the kept set the event freezes is exactly the `keep` verdicts.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct ClosePursuitCommand {
    /// The pursuit to close.
    pub pursuit_id: String,
    /// `satisfied` or `abandoned`.
    pub outcome: String,
    /// `satisfied` only: verdicts over the ledger's candidates. Empty
    /// is a defined state — a close that recorded no decisions (and,
    /// with no mid-work removals, freezes nothing). Must be empty for
    /// `abandoned`, which applies nothing.
    #[serde(default)]
    pub verdicts: Vec<CullVerdictEntry>,
    /// One short free-text slot on the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// One short free-text slot on the cull record itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cull_note: Option<String>,
    /// Caller-asserted operator slug, recorded on the event — who
    /// concluded the line of work, in the same sense as
    /// [`OpenPursuitCommand::operator_ai`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// One requested verdict within a `satisfied` close (#22).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct CullVerdictEntry {
    /// The asset spoken for — must be a candidate of the pursuit's
    /// ledger.
    pub asset_id: String,
    /// `keep` or `reject`. An `existing`-origin member takes `reject`
    /// only (keeping what the library already holds is the untouched
    /// default), except as salvage: a `keep` on a removed member
    /// cancels the removal's default reject.
    pub verdict: String,
    /// The grounds, when stated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Appends one gesture to a pursuit's membership ledger (#22): an
/// asset entering (`in`, with its origin), a mid-work removal, or its
/// reversal. Append-only — membership derives on read, latest gesture
/// per asset wins. `update` is the model's reserved round-trip verb
/// and is refused here until that slice lands.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct RecordPursuitTxCommand {
    /// The pursuit whose ledger this writes.
    pub pursuit_id: String,
    /// `in` / `remove` / `unremove`.
    pub kind: String,
    /// The asset the gesture is about. Must exist in the pursuit's
    /// persona for `in`; must currently be a member for `remove`, and
    /// a removed one for `unremove`.
    pub asset_id: String,
    /// `in` only, required there: `generated` / `imported` /
    /// `existing` — where the asset came from. (`generated` entries
    /// are ordinarily written by the dispatch reify itself; the verb
    /// accepts the origin for completeness.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// One short free-text slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Caller-asserted operator slug, recorded on the row — see
    /// [`OpenPursuitCommand::operator_ai`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// Reopens a pursuit (#29). Legal on an already-open pursuit: the
/// fact is recorded and standing does not change.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct ReopenPursuitCommand {
    /// The pursuit to reopen.
    pub pursuit_id: String,
    /// One short free-text slot on the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Caller-asserted operator slug, recorded on the event — see
    /// [`OpenPursuitCommand::operator_ai`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// Moves a dispatch's pursuit filing — the restamp repair verb (#29),
/// for when the carrying failed (a context-losing surface minted
/// fragments, a pre-created pursuit was stranded). The move is
/// recorded with the prior filing it replaced — read on the server
/// inside the same transaction that moves the stamp, so the record is
/// never stale — and it never touches what happened, never crosses
/// personas. The command carries no caller-observed `from`: a caller
/// acting on an old view moves whatever filing is current
/// (compare-and-swap against the caller's own read is a possible
/// later addition, not a promise this verb makes).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct RestampDispatchCommand {
    /// The dispatch round to re-file.
    pub dispatch_id: String,
    /// The pursuit to file it under.
    pub to_pursuit_id: String,
    /// Caller-asserted operator slug, recorded on the restamp row —
    /// who filed the correction, which is a different question from
    /// who ran the round being moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// Creates a Query Group — a Group whose membership is the
/// materialised result of a stored rule ("Save as Group").
///
/// `query_json` is the v1 blob
/// (`asterism_contract::query_group::QueryGroupQuery`) as a JSON
/// string — a text blob for the same `schema-bridge` reason as
/// [`CreateDispatchCommand::params_json`]. The server validates it
/// loudly, mints the `kind='query'` Group, and evaluates the rule
/// synchronously once so the group is never visible with empty
/// members.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateQueryGroupCommand {
    /// Owner persona id.
    pub persona_id: String,
    /// Group name — unique per persona (Conflict on duplicates).
    pub name: String,
    /// `query_json` v1 blob (validated server-side).
    pub query_json: String,
}

/// Rewrites a Query Group's rule ("Update query"): validate,
/// reject a rule that would close a dependency cycle, persist,
/// and synchronously re-evaluate the membership.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct UpdateQueryGroupQueryCommand {
    /// Target group id (`kind='query'` required).
    pub group_id: String,
    /// Replacement `query_json` v1 blob.
    pub query_json: String,
}

/// Creates a `SavedQuery` — DEAD WIRE TYPE. The SavedQuery concept was
/// absorbed into Query Groups; the
/// V19 migration transcribed every row and dropped the table, and no
/// backend surface accepts this command any more. The type survives
/// only to keep `bindings.ts` stable until the W5 UI wave removes the
/// SavedQuery catalog; do not add new callers.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateSavedQueryCommand {
    /// Persona bucket the query is pinned under.
    pub persona_id: String,
    /// Human-facing name; unique per persona (server rejects
    /// duplicates with `Conflict`).
    pub name: String,
    /// `ListAssetsQuery` serialised as JSON.
    pub filter_json: String,
    /// `SortSpec` (target / order / reverse) serialised as JSON.
    pub sort_json: String,
    /// Optional sidebar position; the server assigns "end of list"
    /// (max + 1) when `None`.
    pub position: Option<i64>,
}

/// Renames a `SavedQuery`. Uniqueness `(persona_id, name)` is
/// re-checked server-side.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RenameSavedQueryCommand {
    /// Target saved-query id.
    pub id: String,
    /// New name.
    pub name: String,
}

/// Deletes a `SavedQuery`. Idempotent — missing id is a no-op.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteSavedQueryCommand {
    /// Target saved-query id.
    pub id: String,
}

/// Rewrites a Session's title
/// (`POST /asterism/sessions/{id}/rename`). The Session model
/// treats `title` as a
/// user-editable label with `None` meaning "untitled"; passing
/// `title = None` is the canonical way to clear a Session back to
/// the untitled state (the fatter `PatchSessionMetadataCommand`
/// cannot express "clear" because its per-field `None` means
/// "leave unchanged"). `note` / `cover_hint` are untouched.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RenameSessionCommand {
    /// Target Session id (populated from the request path on the
    /// HTTP surface; the body field defaults for wire flexibility).
    #[serde(default)]
    pub id: String,
    /// New title (`Some`) or "clear back to untitled" (`None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Partially updates a Session's metadata
/// (`PATCH /asterism/sessions/{id}`). Every field is `None = leave
/// unchanged`, `Some(v) = set to v`, mirroring
/// [`UpdateModalityCommand`]. To clear `title` back to `NULL` use
/// [`RenameSessionCommand`] instead (the patch shape has no
/// "explicit null" leg).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PatchSessionMetadataCommand {
    /// Target Session id (populated from the request path on the
    /// HTTP surface).
    #[serde(default)]
    pub id: String,
    /// Replacement title (`None` = leave unchanged; use
    /// `RenameSessionCommand` to clear).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Replacement note (`None` = leave unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Replacement cover hint (`None` = leave unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_hint: Option<String>,
}

/// Deletes a Session (`DELETE /asterism/sessions/{id}`). Rejected
/// with `409 Conflict` when any `asset` row still references the
/// Session — orphaning delete is forbidden, mirroring the Modality
/// delete guard. Detach the participating assets (or delete them)
/// first, then retry.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeleteSessionCommand {
    /// Target Session id.
    pub id: String,
}

/// Appends one telemetry event to the local `event_log` (dogfooding
/// metrics — `app_open` / `persona_switch` / `search` / `burst_open` /
/// `asset_open`, kinds are open slugs). `occurred_at` is stamped
/// server-side; the recording path is fire-and-forget on the UI side
/// so it never blocks an interaction. Local-only: rows never leave
/// the machine.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RecordEventCommand {
    /// Event kind (open slug, non-empty).
    pub kind: String,
    /// Persona in scope when the event fired (`None` = no persona
    /// context, e.g. the all-personas view).
    pub persona_id: Option<String>,
    /// User-perceived duration of the interaction, when the event
    /// measures one (e.g. `persona_switch` reload time).
    pub duration_ms: Option<i64>,
    /// Extension bag serialised as JSON (`schema-bridge` cannot
    /// codegen `serde_json::Value`, same rule as
    /// [`CreateDispatchCommand::params_json`]).
    pub payload_json: Option<String>,
}

/// Appends one webview-origin diagnostic to `diag_log`
/// (`POST /asterism/diag`, Tauri `record_diag`).
///
/// The write half of the diagnostics story the webview lost when
/// `tauri-plugin-log` was removed: the backend subscriber persists
/// everything `tracing` emits, but a `console.error` or an uncaught
/// exception inside the webview died in the devtools console — which a
/// bundled app does not have open. The frontend capture
/// (`lib/diag.ts`) forwards those moments through this command; the
/// server re-emits them as `tracing` events under the `asterism_webview`
/// target, so they ride the same subscriber → `diag_log` pipe as every
/// native diagnostic and come back out of `GET /asterism/diag`.
///
/// Fire-and-forget on the UI side, like [`RecordEventCommand`]: a lost
/// diagnostic is strictly better than a blocked or recursing UI.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RecordDiagCommand {
    /// Severity name (`error` / `warn` / `info` / `debug` / `trace` —
    /// the set `DiagLevel::parse` accepts). A `String` rather than the
    /// enum so the bindings stay flat; an unknown value is a validation
    /// error, not a guessed level.
    pub level: String,
    /// Event slug. Must start with `webview.` — the namespace is
    /// enforced server-side so a client payload can never steer a
    /// record into the `perf` / `job` streams.
    pub event: String,
    /// Human-readable text (the console arguments / error message).
    pub message: String,
    /// Structured context (stack, source position) serialised as JSON
    /// (`schema-bridge` cannot codegen `serde_json::Value`, same rule
    /// as [`RecordEventCommand::payload_json`]).
    pub attrs_json: Option<String>,
}

/// Overrides one application setting (`PUT /asterism/settings/{key}`).
///
/// `key` must name an entry of the closed setting registry; an unknown
/// key is a `404`, not a silent insert. `value_json` is the value as
/// JSON text (`true`, `7`, `"http://…"`) and must match the key's
/// declared kind, otherwise the write is rejected as a validation
/// error. The stored value is canonicalised, so whitespace differences
/// never produce two spellings of the same setting.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SetSettingCommand {
    /// Registry key (`ui.clean_mode`, `jobs.concurrency`, …).
    pub key: String,
    /// New value as JSON text, matching the key's declared kind.
    pub value_json: String,
}

/// Clears one setting override (`DELETE /asterism/settings/{key}`).
///
/// Idempotent: resetting a key that was never overridden succeeds. This
/// is deliberately a delete rather than a write of the default value, so
/// a later change to the default in code reaches every profile that had
/// not pinned the key.
///
/// The response is the *resolved* value, not necessarily the default:
/// clearing the stored row does not clear an environment variable that
/// outranks it, so a pinned key still reports `source: "env"`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ResetSettingCommand {
    /// Registry key to reset.
    pub key: String,
}
