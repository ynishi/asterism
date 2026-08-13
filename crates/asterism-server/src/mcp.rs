//! MCP transport — the third adapter over the same application services.
//!
//! Tools are a curated use-case vocabulary, not a mirror of the 100+
//! HTTP routes: an agent walking `tools/list` should see the ledger's
//! actual entry points (search / list / get / add / lineage / comments /
//! catalog / dispatch) and nothing that only exists to serve the grid's
//! rendering loop. Anything not covered here is reachable over the HTTP
//! API on the same port — `get_info` says so in `instructions`.
//!
//! Input schemas come from the same `asterism-contract` types that back
//! HTTP bodies and Tauri IPC (contract feature `json-schema`), so the
//! three transports cannot drift on shape. Thin parameter structs exist
//! only where HTTP used path/query extractors instead of a body type
//! (mirroring `http.rs`'s `LineageParams` and friends).
//!
//! This handler is served over **streamable-http**, nested at `/mcp` on
//! the loopback axum router ([`streamable_service`]) — it exists
//! wherever the HTTP API does (Tauri-embedded serve and the standalone
//! binary alike). MCP clients that spawn a child process use
//! `asterism-server mcp` instead, which is a lifecycle-aware stdio
//! *proxy* onto this same endpoint (see `crate::mcp_proxy`), not a
//! second instance of this handler.
//!
//! Domain failures are reported as tool results (`is_error: true`) with
//! the same `{kind, message}` shape the HTTP `ApiError` writes, rather
//! than JSON-RPC protocol errors: a NotFound is an answer the calling
//! agent should read, not a broken call.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, DeclareAssetMetaCommand, MergeAssetsCommand, PostAssetCommentCommand,
    PostMaterialMarkCommand, ResolveDuplicateConflictCommand,
};
use asterism_contract::query::{GetAssetDetailQuery, ListAssetsQuery, SearchAssetsQuery};
use asterism_core::DomainError;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ListResourcesResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::state::ServerCtx;

/// The MCP server handler — a thin tool facade over [`ServerCtx`].
///
/// Holds the same service bundle the HTTP handlers share; every tool
/// body is one service call plus serialisation, exactly like an axum
/// handler.
#[derive(Clone)]
pub struct AsterismMcp {
    ctx: Arc<ServerCtx>,
}

/// Wraps a serialisable answer as a successful tool result (one JSON
/// text content block).
fn ok_json<S: serde::Serialize>(value: &S) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![ContentBlock::json(value)?]))
}

/// Maps a [`DomainError`] onto a failed tool result carrying the same
/// `{kind, message}` JSON the HTTP boundary returns for the same error.
fn domain_error(err: DomainError) -> CallToolResult {
    let kind = match &err {
        DomainError::PersonaNotFound(_)
        | DomainError::AssetNotFound(_)
        | DomainError::NotFound { .. } => "NotFound",
        DomainError::Validation(_) => "Validation",
        DomainError::DuplicatePersona(_) | DomainError::Conflict(_) => "Conflict",
        DomainError::Infra(_) => "Internal",
    };
    let body = serde_json::json!({ "kind": kind, "message": err.to_string() });
    CallToolResult::error(vec![ContentBlock::text(body.to_string())])
}

/// `asset_lineage` input — mirrors the HTTP `LineageParams` query pair
/// plus the path segment.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AssetLineageParams {
    /// Asset to trace from.
    pub asset_id: String,
    /// Hops to walk in both directions (server clamps to 1..=8).
    pub depth: Option<u32>,
    /// Requesting subject (`null` = owner view).
    pub viewer_subject: Option<String>,
}

/// `asset_comments` input.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AssetCommentsParams {
    /// Asset whose comment thread to read.
    pub asset_id: String,
}

/// `material_marks` input.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MaterialMarksParams {
    /// Asset whose material to read the marks of.
    pub asset_id: String,
}

/// `material_layers` input.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MaterialLayersParams {
    /// Asset whose material to read the bands of.
    pub asset_id: String,
}

/// `catalog_overview` input.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct CatalogOverviewParams {
    /// Scope the modality / tag / group listings to one persona
    /// (`null` = across every persona).
    pub persona_id: Option<String>,
}

/// `dispatch_get` input.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DispatchGetParams {
    /// Dispatch id (returned by the export that created it, and
    /// stamped on derived assets' provenance).
    pub dispatch_id: String,
}

/// `duplicate_conflicts` input — the same persona / limit pair the
/// HTTP query string carries.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DuplicateConflictsParams {
    /// Restrict to one persona's questions (`null` = every persona).
    pub persona_id: Option<String>,
    /// Maximum number of questions to return. Omitted = the service's
    /// default work-list size.
    pub limit: Option<u32>,
}

#[tool_router]
impl AsterismMcp {
    /// Builds the handler over an assembled service bundle.
    pub fn new(ctx: Arc<ServerCtx>) -> Self {
        Self { ctx }
    }

    #[tool(
        description = "Ranked shortlist for \"find me something like this\" (Tantivy BM25, Japanese + English tokenization, typo-tolerant). Returns the closest N candidates, best match first, narrowed by `filter`. **This does not answer \"which assets match\".** It looks at a bounded number of candidates, so assets past that ceiling never appear however deep you page: `matched` counts candidates that survived the filter, `candidates_considered` says how many were looked at, and `truncated` says the shortlist filled up. There is no `total` because there is no total to give. For an exhaustive, countable answer use `asset_list` with `filter.text_match` — the same text as a SQL predicate (substring match, any length), which can be counted, sorted on any axis and paged to any depth. Order here is relevance and only relevance: `filter.sort` is rejected rather than ignored."
    )]
    async fn asset_search(
        &self,
        Parameters(query): Parameters<SearchAssetsQuery>,
    ) -> Result<CallToolResult, McpError> {
        match self.ctx.asset_service.search(query).await {
            Ok(page) => ok_json(&page),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "List assets with filters (persona / modality / tags / groups / label / format / color / rating band / trash side / `text_match`) and an optional server-side sort axis. `text_match` is the exhaustive text filter: it matches when the asset's body **contains** the string, with no word boundaries and no dictionary (`スト` finds `テスト`, `猫` finds `黒猫`), and it behaves like every other predicate here — countable, sortable, pageable to any depth. Use it when you want every asset carrying a term; use `asset_search` when you want the closest few to a fuzzy description. Three independent time windows narrow separately: `occurred_from_ms` / `occurred_until_ms` (when the thing happened, upper end exclusive), `created_*` (when it was ingested) and `updated_*` (when it last changed) — the latter two are inclusive at both ends so a sync cursor can be replayed verbatim. Every returned card carries `updated_at_ms`; hand the highest one back as `updated_from_ms` to poll for changes, and pair it with sort target `updated_at` so the page is ordered along the cursor. `sort` example: {\"target\":\"occurred_at\",\"order\":\"updated\",\"reverse\":false}. Omitting `sort` returns the repository's arrival order. Paged via offset / limit."
    )]
    async fn asset_list(
        &self,
        Parameters(query): Parameters<ListAssetsQuery>,
    ) -> Result<CallToolResult, McpError> {
        match self.ctx.asset_service.list(query).await {
            Ok(page) => ok_json(&page),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Fetch one asset's full detail: metadata, materials, tags, labels, session membership, and the `extra` bag (provenance trace included)."
    )]
    async fn asset_get(
        &self,
        Parameters(query): Parameters<GetAssetDetailQuery>,
    ) -> Result<CallToolResult, McpError> {
        match self.ctx.asset_service.detail(query).await {
            Ok(detail) => ok_json(&detail),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Ingest one asset into the ledger. Same command shape importers POST to /asterism/assets/add. `persona_id`, `source_kind`, `locator`, `occurred_at_ms` are required. Declare provenance via `derived_from` (`asset:<uuid>`, `dispatch:<uuid>`, or `sidecar`) to reconnect a file that went through an outside generator. Record anything you want to *say about* the artefact — a generator's own reference, a workflow id, a catalogue number — in `album_meta` as `{\"key\":\"value\"}`; keys are lowercase letters, digits, `_` and `-`, and one bad entry refuses the whole ingest rather than landing an asset that answers to none of its names. Note what `album_meta` is not: a recorded identifier stays a statement, never a key — Album's own asset id is the identity, and looking rows up by a recorded value is a filter over it. Attribution is self-declared: `operator_ai` is your own slug (`claude-code`, `codex`, …) and `author_kind` is \"owner\" or \"subject\" (the latter needs `author_subject`). Omitting them records nothing — they are never filled in for you, so an absent author reads as unrecorded rather than as the owner."
    )]
    async fn asset_add(
        &self,
        Parameters(command): Parameters<AddAssetCommand>,
    ) -> Result<CallToolResult, McpError> {
        // An MCP client is a remote caller like any other: its three
        // attribution fields are its own statement about itself, and
        // this is where they become the context the service records
        // from (the service never reads them).
        let attribution = match crate::attribution::asserted(
            command.author_kind.as_deref(),
            command.author_subject.as_deref(),
            command.operator_ai.as_deref(),
        ) {
            Ok(attribution) => attribution,
            Err(err) => return Ok(domain_error(err)),
        };
        match self.ctx.asset_service.add(command, &attribution).await {
            Ok(asset) => ok_json(&asset),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Record — or remove — one AlbumMeta statement on an asset that is already in the ledger: the after-the-fact half of `asset_add`'s `album_meta`. `key` is the name it is filed under (lowercase letters, digits, `_`, `-`), `value` is what is being said, and omitting `value` removes the key. Declaring the same key twice leaves the later statement, so this is also how a value guessed at ingest gets corrected. `operator_ai` is the agent the statement came through and is recorded on the entry; it is not who the asset is by. Nothing is inferred from what you record here — no edge is drawn and the asset's identity does not move."
    )]
    async fn asset_declare_meta(
        &self,
        Parameters(command): Parameters<DeclareAssetMetaCommand>,
    ) -> Result<CallToolResult, McpError> {
        // The command's `operator_ai` belongs to the statement
        // (`_trace.meta.<key>.operator`), not to the row's attribution
        // columns — saying something about an asset is not authoring it.
        // So the context here is the request's own channel, exactly as
        // the HTTP route decides it.
        let attribution = match crate::attribution::asserted(None, None, None) {
            Ok(attribution) => attribution,
            Err(err) => return Ok(domain_error(err)),
        };
        match self
            .ctx
            .asset_service
            .declare_asset_meta(command, &attribution)
            .await
        {
            Ok(asset) => ok_json(&asset),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Trace an asset's derivation chain (what it was made from and what was made from it) as a lineage graph: nodes with depth, roots of the chain, and the dispatch ids the chain passed through, nearest first."
    )]
    async fn asset_lineage(
        &self,
        Parameters(params): Parameters<AssetLineageParams>,
    ) -> Result<CallToolResult, McpError> {
        // Same default as the HTTP route: deep enough for
        // out-and-back-twice chains without a corpus walk.
        let depth = params.depth.unwrap_or(4);
        match self
            .ctx
            .asset_service
            .lineage_of(&params.asset_id, params.viewer_subject.as_deref(), depth)
            .await
        {
            Ok(view) => ok_json(&view),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Read an asset's comment thread in chronological order — the same rows the desktop detail panel shows."
    )]
    async fn asset_comments(
        &self,
        Parameters(params): Parameters<AssetCommentsParams>,
    ) -> Result<CallToolResult, McpError> {
        match self.ctx.asset_comment_service.list(&params.asset_id).await {
            Ok(comments) => ok_json(&comments),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Append one comment to an asset's thread. Author identity is self-declared: `author_kind` is \"user\" or \"persona\" (the closed set the comment model accepts; \"persona\" requires `author_persona_id`). An agent posts as the persona it acts for rather than impersonating the user."
    )]
    async fn asset_comment_add(
        &self,
        Parameters(command): Parameters<PostAssetCommentCommand>,
    ) -> Result<CallToolResult, McpError> {
        // `command.author_kind` ("user" / "persona") is the comment's own
        // author — a register, not an attribution assertion, and a
        // different value domain from the asset-side field of the same
        // name. It stays with the command; the context states only that
        // this write arrived over MCP, which is the asserted channel.
        let attribution = match crate::attribution::asserted(None, None, None) {
            Ok(attribution) => attribution,
            Err(err) => return Ok(domain_error(err)),
        };
        match self
            .ctx
            .asset_comment_service
            .post(command, &attribution)
            .await
        {
            Ok(comment) => ok_json(&comment),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Read the marks placed into an asset's material — notes at a position inside the content, as opposed to `asset_comments`, which are about the asset as a whole. Returned in the material's own order (earliest position first), not the order they were placed in. Each mark carries `anchor_kind` (`\"temporal\"` today) with `start_ms` and an optional exclusive `end_ms`: a point on the playback timeline, or an interval."
    )]
    async fn material_marks(
        &self,
        Parameters(params): Parameters<MaterialMarksParams>,
    ) -> Result<CallToolResult, McpError> {
        match self
            .ctx
            .material_mark_service
            .list_by_asset(&params.asset_id)
            .await
        {
            Ok(marks) => ok_json(&marks),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Place one mark into an asset's material. `anchor_kind` is \"temporal\": `start_ms` is required and gives the position on the playback timeline, `end_ms` is an exclusive end for an interval (omit it for an instant). The asset must be time-bearing — a mark on the timeline of something with no duration is refused. Author identity is self-declared exactly as on `asset_comment_add`: `author_kind` is \"user\" or \"persona\" (the latter requires `author_persona_id`)."
    )]
    async fn material_mark_add(
        &self,
        Parameters(command): Parameters<PostMaterialMarkCommand>,
    ) -> Result<CallToolResult, McpError> {
        // `command.author_kind` names the voice the mark is written in,
        // like the comment field of the same name; the context states
        // only that this write arrived over MCP, the asserted channel.
        let attribution = match crate::attribution::asserted(None, None, None) {
            Ok(attribution) => attribution,
            Err(err) => return Ok(domain_error(err)),
        };
        match self
            .ctx
            .material_mark_service
            .post(command, &attribution)
            .await
        {
            Ok(mark) => ok_json(&mark),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Read how an asset's material is divided, and by whom. Returns every band of marks over the material, each with the chapters in it. `origin` says who produced the band: \"imported\" is the container's own declaration (an MP4 `chpl`, a Matroska Chapters segment), \"user\" is the person's own reading of the same material, \"machine\" is a job's. `role` is \"structure\" (holds the chapters returned beside it) or \"annotation\" (holds notes — read those with `material_marks`; `chapters` is always empty here). `is_default` is the band a surface shows. Each chapter carries `start_ms`, an optional exclusive `end_ms` (absent = the file stated no end, so the section runs to the next one's start), a `label` that may legitimately be empty, and `ord`, the reading order the band states — which need not be the timeline's. **Chapter ids are not stable across a re-read of the material**: reading the file again replaces an imported band's rows wholesale, so key on (`layer_id`, `ord`) rather than on `id`. An asset with no bands has never been read for chapters; a structure band with an empty list was read and declares none."
    )]
    async fn material_layers(
        &self,
        Parameters(params): Parameters<MaterialLayersParams>,
    ) -> Result<CallToolResult, McpError> {
        match self
            .ctx
            .material_layer_service
            .list_views(&params.asset_id)
            .await
        {
            Ok(views) => ok_json(&views),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Discover the filter vocabulary in one call: personas (with display names), modality asset counts, tag counts, and user-curated groups. Use the returned ids/slugs as `asset_list` / `asset_search` filter values."
    )]
    async fn catalog_overview(
        &self,
        Parameters(params): Parameters<CatalogOverviewParams>,
    ) -> Result<CallToolResult, McpError> {
        let persona_id = params.persona_id.as_deref();
        let personas = match self.ctx.persona_service.list().await {
            Ok(v) => v,
            Err(err) => return Ok(domain_error(err)),
        };
        let modality_counts = match self
            .ctx
            .asset_service
            .list_modality_asset_counts(persona_id, None)
            .await
        {
            Ok(v) => v,
            Err(err) => return Ok(domain_error(err)),
        };
        let tag_counts = match self.ctx.asset_service.list_tag_counts(persona_id).await {
            Ok(v) => v,
            Err(err) => return Ok(domain_error(err)),
        };
        let groups = match self.ctx.asset_service.list_groups(persona_id).await {
            Ok(v) => v,
            Err(err) => return Ok(domain_error(err)),
        };
        ok_json(&serde_json::json!({
            "personas": personas,
            "modality_counts": modality_counts,
            "tag_counts": tag_counts,
            "groups": groups,
        }))
    }

    #[tool(
        description = "List unanswered duplicate questions: pairs of assets in one persona that were found to agree on **one** fingerprint axis, where nobody has yet said whether they are one thing or two. **Read `axis` before you answer** — it names what was actually compared, and the three claims are not equally strong. `\"artefact\"`: every byte of the original file, so the two are the same file. `\"content\"`: only the bytes that decide the decoded result, so the files differ and the picture does not (a re-encode, or a copy with its metadata rewritten). `\"meta\"`: only the metadata the container carries — and detection reports the strongest agreement it finds, so a `meta` row is a pair that agreed on *neither* of the others: **its pictures differ.** That is 'made the same way' rather than 'one thing twice', and folding it discards a distinct picture; the axis structure that lets such a pair be queued is a known defect under repair. Each row carries both sides as full cards (`newcomer` = the younger arrival, `incumbent` = the row that was already there), the digest they share on that axis, and `fold_exclusion` — `\"lineage\"` (the two are connected through `derived_from`, or the graph was too large to say otherwise) or `\"dispatch\"` (at least one is the output of an export run) — naming a rule that stopped an automatic fold and handed the pair to a person. A null `fold_exclusion` means no automatic fold was declined. Answer a row with `duplicate_conflict_resolve`."
    )]
    async fn duplicate_conflicts(
        &self,
        Parameters(params): Parameters<DuplicateConflictsParams>,
    ) -> Result<CallToolResult, McpError> {
        match self
            .ctx
            .asset_service
            .list_duplicate_conflicts(params.persona_id.as_deref(), params.limit)
            .await
        {
            Ok(conflicts) => ok_json(&conflicts),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Answer one duplicate question from `duplicate_conflicts`. `resolution: \"folded\"` rules the two one thing and queues a fold — `keeper_id` is required and must be one of the pair; the row named stays and absorbs the other's tags, groups, comments and edges, and the other becomes a headstone that still resolves to the keeper. `resolution: \"kept\"` rules them two separate things: both rows stay untouched, `keeper_id` must be omitted, and the pair is not raised again. Either way the fact that the bytes matched stays recorded as an `identical_to` edge. Refused if the question was already answered, or if either side has since been folded away or moved to the trash."
    )]
    async fn duplicate_conflict_resolve(
        &self,
        Parameters(command): Parameters<ResolveDuplicateConflictCommand>,
    ) -> Result<CallToolResult, McpError> {
        // An answer to a duplicate question is a write like any other,
        // and an MCP client is a remote caller: the context states the
        // channel it arrived through and nothing about who the caller
        // is, because this command carries no claim to record.
        let attribution = match crate::attribution::asserted(None, None, None) {
            Ok(attribution) => attribution,
            Err(err) => return Ok(domain_error(err)),
        };
        match self
            .ctx
            .asset_service
            .resolve_duplicate_conflict(command, &attribution)
            .await
        {
            Ok(outcome) => ok_json(&outcome),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Collapse N rows into one: the manual merge verb. Not scoped to the duplicate queue — the caller declares the whole set (keeper_id + discard_ids + member_ids = every row the caller is ruling over) and the discards are folded into the keeper, absorbing their tags, groups, comments, and edges. `dry_run: true` previews the outcome and writes nothing; `dry_run: false` commits. Both branches return the same shape — the `committed` bool is what tells them apart, so a run following a preview reads the answer back on the same fields. `member_ids` must equal `{keeper_id} ∪ discard_ids` exactly (each id once); a mismatch is refused as `Validation` before any write. `refusals` on the response names rows the fold could not touch (the keeper was in the trash, was itself folded, or an id names no live row) and rides with `committed: false` — a refusal is not a call error but a decision the caller has to re-make; one refusal abandons the whole merge (all-or-nothing). `warnings` on a preview names pairs a rule (lineage, dispatch) would have declined an automatic fold of — it is not binding here (a person's ruling overrides the automatic rules on purpose) and always empty on the commit branch."
    )]
    async fn assets_merge(
        &self,
        Parameters(command): Parameters<MergeAssetsCommand>,
    ) -> Result<CallToolResult, McpError> {
        // Same attribution shape as the queue verb above and every
        // other MCP write: the channel is asserted, the caller is not.
        let attribution = match crate::attribution::asserted(None, None, None) {
            Ok(attribution) => attribution,
            Err(err) => return Ok(domain_error(err)),
        };
        match self
            .ctx
            .asset_service
            .merge_assets(command, &attribution)
            .await
        {
            Ok(outcome) => ok_json(&outcome),
            Err(err) => Ok(domain_error(err)),
        }
    }

    #[tool(
        description = "Fetch one outbound dispatch's persisted state (status, exporter, frozen input snapshot, outputs). Dispatch ids appear in asset lineage and in exporter sidecars."
    )]
    async fn dispatch_get(
        &self,
        Parameters(params): Parameters<DispatchGetParams>,
    ) -> Result<CallToolResult, McpError> {
        match self.ctx.dispatch_service.get(&params.dispatch_id).await {
            Ok(dispatch) => ok_json(&dispatch),
            Err(err) => Ok(domain_error(err)),
        }
    }
}

/// Onboarding guide body. Kept in one place so `list_resources` and
/// `read_resource` cannot drift from each other, and so callers can
/// grep for the URI without hitting a duplicated blob.
///
/// The text is a **skeleton** — layout and content will shift as the
/// tool set moves; consumers should treat it as orientation, not a
/// spec.
const GUIDE_ONBOARDING_URI: &str = "asterism://guides/onboarding";
const GUIDE_ONBOARDING_TITLE: &str = "Asterism MCP Onboarding";
const GUIDE_ONBOARDING_BODY: &str = r#"# Asterism MCP — orientation

Asterism is a local-first provenance ledger for AI-generated and personal
media. This MCP surface is the third adapter over the same application
services that back the HTTP API (`/asterism/*`) and the Tauri desktop
app; anything the desktop app can do is available as a tool or over
loopback HTTP on the same port.

## Recommended flow (agent-facing)

1. `catalog_overview` — discover the persona / modality / tag / group
   vocabulary before filtering. Returned ids/slugs feed
   `asset_list` / `asset_search`.
2. `asset_search` (Tantivy BM25, JP + EN) or `asset_list`
   (server-side sort + facets) — find assets.
3. `asset_get` — one asset's full record (metadata, materials, tags,
   labels, session membership, `extra._trace`).
4. `asset_lineage` — trace a derivation chain (nodes with depth, roots,
   dispatch ids passed through). Clamped to depth 1..=8, 200 nodes.
5. `asset_add` — ingest one asset. `derived_from` (`asset:<uuid>` /
   `dispatch:<uuid>` / `sidecar`) reconnects a file that went through
   an outside generator. `operator_ai` is your own slug, self-declared
   and unauthenticated (like `viewer_subject` on the read side); leave
   it out and the row records no operator at all.
6. `asset_comment_add` / `asset_comments` — write to and read a comment
   thread as yourself (`author_kind = "user" | "persona"`; persona
   requires `author_persona_id`).
7. `material_mark_add` / `material_marks` — the same two verbs one level
   in: a note at a position *inside* the asset's content rather than
   about the asset. Today's anchor is `"temporal"` (`start_ms`, plus an
   exclusive `end_ms` for an interval) and needs a time-bearing asset;
   marks read back in the material's order, earliest position first.
8. `material_layers` — how the material is divided, and by whom. Every
   band of marks over the content, each with its chapters: `origin`
   separates the container's own declaration (`imported`) from a
   person's reading of it (`user`) and a job's (`machine`), so "the
   chapters this file ships with" and "the chapters someone corrected"
   are two answers rather than one. Read-only here on purpose — the
   write verbs correct a person's own reading of a file, which is an
   act with a person in the middle of it, and the desktop app and the
   HTTP surface (`/asterism/material-layers`,
   `/asterism/chapter-marks`) are where it happens.
9. `duplicate_conflicts` / `duplicate_conflict_resolve` — the pairs of
   assets that agreed on one fingerprint axis and that nobody has ruled
   on yet, and the verb that answers one. **Each row names its axis**,
   and they claim different things: `artefact` is every byte of the
   file, `content` is only the bytes that decide the decoded picture,
   `meta` is only the container's metadata. Detection reports the
   strongest agreement it found, so a `meta` row agreed on neither of
   the others — **its two pictures differ**, and it is a "made the same
   way" observation rather than one thing twice. `folded` names a
   `keeper_id` from the pair and queues the fold, which leaves a marker
   in place of the other row and does not come back; `kept` says they
   are two separate things and leaves both rows alone. A pair is asked
   about once: whichever answer is given, it is not raised again.
10. `dispatch_get` — one outbound dispatch's persisted state.

## Beyond the tool set

The tools above are a curated use-case vocabulary. When you need
something not covered here (settings, threads, snapshots, group
membership, media/thumb bytes), reach for the HTTP API under
`/asterism/*` on the same loopback port — same shapes, same services.

Tag *administration* is HTTP-only and worth knowing about before you
tag at scale: `POST /asterism/tags/rename` (409 if the name is taken —
it never merges), `POST /asterism/tags/merge`
(`{source_tag_id, target_tag_id, dry_run}` — folds one channel into
another and deletes the source; `dry_run: true` returns the same
counts without writing), and `POST /asterism/tags/delete` (drops the
channel and every link to it). These are the way back from synonym and
spelling-variant sprawl.

Series **Strategies** — the rules that group materials by how they were
made — are HTTP-only too (`/asterism/series-strategies`), and writing one
needs a vocabulary rather than a verb: read
`asterism://schemas/series-strategy` first. It carries the closed decoder
set, the path grammar, and the reason to reach for `include` over
`exclude`.

Binary is HTTP-only, deliberately — bytes do not belong in a tool
result. An asset's **original** file is
`GET /asterism/assets/{id}/file` (streamed, typed by the material's
mime); its cached thumbnail is
`GET /asterism/assets/{id}/thumbs/{size_px}` (202 = generation queued,
poll again). Both take `?viewer_subject=` and apply the same
visibility rule as `asset_get`.

## Locators and local bytes

Asterism is local-first: the originals it browses are files on this
disk, referenced in place (never copied, never written back), and
instant browsing is built from those local bytes — thumbnails,
previews, and `GET /asterism/assets/{id}/file` all read the original
directly.

A locator is an open string. Besides the absolute path (the common
case), two non-file shapes are ordinary internal record forms: a
fragment (`session.jsonl#<id>`) addressing one record inside a
container file on this disk, and a caller-minted logical name
(`chat/<id>/msg-1`) for something that never had a file. A fragment's
record text stays in its container — Asterism never writes back to a
locator — and is read out of it for search and the session Reader
view; the metadata loop (tags, groups, lineage, search, comments)
works on these assets like any other.

A remote URL (`https://…`, `s3://…`) is accepted at registration and
stored verbatim as a fact about origin, but Asterism never fetches
it: no thumbnail can be rendered, binary serving answers 409, and
content hashing records "no bytes to read", so duplicate grouping
never claims the asset. To manage remote output (object storage,
hosted generators) as browsable media, bring the file to this disk
first and import that path.

## Attribution (whose work, which agent, and how it arrived)

An asset records three things about *who*, separate from provenance
(`extra._trace` says where a claim entered; these say whose the write
is):

- **author** — `author_kind = "owner" | "subject"`, with
  `author_subject` naming the subject for the `"subject"` kind.
  `"owner"` is not a token but a reference to this instance's single
  owner record, whose subject is unbound today: it reads as "whoever
  this Asterism belongs to" and resolves to no name until
  authentication binds it once. Author subjects and `viewer_subject`
  are one namespace — "written by alice" and "shared with alice" mean
  the same alice.
- **operator_ai** — the agent that carried the operation out
  (`"claude-code"`, `"codex"`, an importer's own slug). One subject
  drives Asterism through several agents, so "through what" is a
  different question from "whose".
- **channel** — read back as `attributed_via`: `"owner-surface"` (the
  desktop app's own IPC), `"asserted"` (a remote caller stating its
  own), or `"authenticated"` (reserved for the auth wave). **You
  cannot state it.** It is derived from which entry point served the
  write and no command carries a field for it, because that
  derivation is the only thing that will let a hosted deployment tell
  an authenticated author from a caller that merely claimed one.

Everything written through this surface is `"asserted"`: whatever you
put in `author_kind` / `author_subject` / `operator_ai` is your own
statement about yourself, believed and labelled as such, exactly like
`viewer_subject` on the read side. `author_kind = "owner"` is
therefore **refused** here — a `Validation` error (HTTP `400` for the
same call on `/asterism/assets/add`) — since owner-ness follows from
the owner's own app or from authentication, never from the claim. Say
`"subject"` with the subject you go by, or leave the fields out.

Absence is meaningful: an unset field means "nobody recorded this",
never "the owner". Asterism does not default-fill attribution, so an
assertion made today stays distinguishable from anything a future
login system backfills, and a row that attributes nobody records no
channel either. Background jobs and sweeps (thumbnails, hashing,
re-resolution) leave these fields alone — that the app was running is
not somebody a write can be by. A dispatch carries its own
attribution through to the assets it reifies (its operator is also
echoed in `extra._dispatch.operator`), and the manual provenance verb
records its operator in `extra._trace.operator`.

A persona is not an author. Every asset belongs to exactly one
persona, and comments and thread messages can be posted in a
persona's voice — but membership says where an asset is filed and a
voice says how something is phrased; neither says who wrote it. That
is why the asset-side `author_kind` rejects `"persona"` while
`asset_comment_add` accepts `"user" | "persona"`: the comment field
is naming a voice, not making an attribution.

## Stability note

Tool names, argument shapes, and this guide are **subject to change**
while the MCP surface iterates. Treat every response shape as
observation-time truth; hard-coded parsers should be regenerated from
the `inputSchema` published by `tools/list`.
"#;

/// How to write a series Strategy, for the agent on the far side of the
/// process boundary that will write one.
///
/// A resource rather than a tool because a Strategy is registered over
/// HTTP (`POST /asterism/series-strategies`) and what is missing on that
/// side is not a verb but the vocabulary: which decoders this build
/// ships, how a path is spelled, and which of the two rules to reach for.
/// The design's whole argument for making a Strategy *data* is that the
/// author cannot ship code — so the closed set has to be readable, or
/// "you may register a rule" means nothing.
///
/// The measured example is the seeded rule, verbatim, because a made-up
/// one would be a shape nobody has run.
const SCHEMA_SERIES_STRATEGY_URI: &str = "asterism://schemas/series-strategy";
const SCHEMA_SERIES_STRATEGY_TITLE: &str = "Series Strategy — how to write one";
const SCHEMA_SERIES_STRATEGY_BODY: &str = r#"# Series Strategy — "made the same way"

A **Strategy** is a rule for reading one sentence out of a material's
metadata: *these two files were made the same way*. Applying it to a
material yields a **key**, and materials sharing a key are one series —
one generation run, one recipe, one character card.

It is data, not code, and deliberately so: an importer (or the agent
driving one) runs in its own process and talks to this server over HTTP,
so a rule it wants applied has to cross the wire as a value. That is why
`decode` names one of a small closed set rather than being a parser you
supply.

Nothing here touches the metadata digest (`m1-`). That digest states what
the container carried; a series key states what one rule made of it. Two
statements about one material, and neither weakens the other.

## The shape

```json
{
  "name": "VDSL recipe",
  "applies_to": "image/png",
  "decode": "raw_json",
  "include": [["vdsl", "script"]],
  "exclude": []
}
```

| field | meaning |
|---|---|
| `name` | a label. Never read by the derivation — renaming moves no key |
| `applies_to` | the one media type this rule is written against. One type, no wildcard: PNG `tEXt` keywords and EXIF tag numbers are not one namespace |
| `decode` | how the text the container carried becomes something a path can walk |
| `include` | the sub-trees to keep. Empty (or omitted) means the whole of the metadata |
| `exclude` | the sub-trees to drop, applied **after** `include` |

`applies_to` is compared against a material's own media type for
equality, so it has to be a `type/subtype` pair and is **refused with a
`400`** when it is not: `"png"`, `"image/"` and `"image/*"` all name
nothing that can ever match. A subtype this build has never heard of
(`image/jxl`) is fine — what a material carries is whatever its importer
declared. Parameters and case are normalised away, so
`IMAGE/PNG; charset=binary` registers as `image/png`.

## `decode` — the closed set this build ships

| token | reads |
|---|---|
| `none` | nothing. The value is the whole of what a path can address, so a one-segment path selects it and a longer one finds nothing. The right choice for prose, and the safe choice for a container you have not looked inside |
| `raw_json` | a JSON document sitting in the value. ComfyUI's `workflow` / `prompt` chunks and VDSL's `vdsl` chunk are this |
| `base64_json` | base64 of a JSON document — the character-card convention (`ccv3`). Standard alphabet, canonical padding; anything else is not decoded |
| `exif` | one EXIF field, written `type:rendering` (`rational:1/125`). It moves the type from the front of the string to a second path segment, so `["exif:0x829a","rational"]` selects the value only where that tag really is a rational |

A token this build does not ship is **refused** (`400`), not quietly
treated as `none`. The two look alike — neither descends into a value —
and they are not: `none` is you saying *the value is the whole of what I
am addressing*, while an unknown token is a rule this build cannot carry
out.

**A value the decoder cannot read is kept as the text the container
stated.** It is not treated as absent, because dropping it would remove a
distinction, and removing distinctions is how unrelated files end up
under one key. A path into such a value simply resolves nowhere.

## Paths

A path is a list of segments, outermost first. **The first segment is a
keyword of the container's metadata** — the `tEXt` keyword, the chunk
name — and the rest walk into whatever `decode` made of that keyword's
value.

```
["vdsl", "script"]        the `script` field of the decoded `vdsl` chunk
["prompt"]                the `prompt` chunk, whole
["ccv3", "data", "name"]  two levels inside the decoded card
["ifd0:0x010f"]           a JPEG's `Make` tag, whole
["ifd0:0x010f", "ascii"]  the same tag, only where it really is an ASCII string
```

**A JPEG's keywords are addresses rather than names**: `<ifd>:0x<tag>`,
where the IFD is `ifd0` / `ifd1` for the TIFF blocks and `exif` / `gps` /
`interop` for the sub-blocks. The IFD is part of the key because one tag
number means different things in different ones — `0x0112` in `ifd0` is
how the photograph is rotated and in `ifd1` is how its thumbnail is.

Segments name **object keys and only object keys**. There is no array
indexing: `["a", "0"]` addresses a key spelled `0`, never the first
element, and a path that lands on an array selects it whole. A path with
no segments is refused — it would select nothing and drop nothing, which
is not what leaving a field blank meant.

Send `include` as a **list of paths**: `[["vdsl","script"]]`, not
`["vdsl","script"]`. The second is one path naming a keyword called
`vdsl` and a keyword called `script`, and it is the mistake that spells
most like the right thing.

## Include is sharp and goes stale; exclude is blunt and safe

The two rules fail in opposite directions when a field nobody named turns
up:

| | a new field arrives | result |
|---|---|---|
| `include` | it is not selected | fewer distinctions — separate things share a key |
| `exclude` | it is not dropped | more distinctions — one run splits into several |

**Merging is the error you cannot get back.** A key that wrongly groups
two unrelated files is indistinguishable, downstream, from a key that
rightly groups two shots of one run — and the whole point of the axis is
to act on that grouping. A split costs you a grouping you wanted, and
you can see it: the run is in pieces on the screen, and you fix the rule.

So: **when in doubt, name the field in `include` and accept the split.**
Say field `A` is one you know identifies the recipe and field `B` is one
you are unsure about:

```
include [["A"]]              key = f(A)     files differing only in B merge   ← unrecoverable
include [["A"], ["B"]]       key = f(A, B)  files differing only in B split   ← visible, fixable
```

Leaving `B` out is the choice that cannot be undone, and it is the one
that reads like caution. Prefer `include` overall (it is the only rule
that reaches a per-file field like a compiled prompt graph), and reach
for `exclude` when the vocabulary is open and you cannot enumerate what
to keep.

## Why `include` is usually the answer

Measured on eleven images out of two VDSL runs: digesting every chunk
separates all eleven; dropping the run's `timestamp` separates all
eleven; dropping the generator's whole chunk separates all eleven. What
splits them is the `prompt` chunk — a compiled graph that differs per
image — and **no exclusion reaches it**. Selecting the recipe and nothing
else (`["vdsl","script"]`) recovers the two runs, at five and six.

## Writing one against a JPEG

A JPEG's metadata is its EXIF block, so the keywords are the addresses
above (`ifd0:0x010f`, `exif:0x829a`) and `decode` is `exif`. Which tags
to name is where this gets decided, and part of the answer is quotable
while the rest is not.

**Quotable.** Exif 3.0 Annex H (CIPA DC-008) classifies every standard
tag by what a tool may do to it: `Update 0` (rewritten on every edit),
`Update 1`, `Freeze 0` (shall not be deleted or modified under any
circumstance), `Freeze 1` (needs no update), `Freeze 2` (may be
corrected where wrong). Every image-structure tag is `Update 0`, and so
are `DateTime` (`ifd0:0x0132`) and `SubSecTime`. Excluding those is
following the specification: they say who exported the file, not how the
picture was made. <https://www.cipa.jp/e/std/std-sec.html> —
<https://archive.org/details/exif-specs-3.0-dc-008-translation-2023-e>
(Annex H, pp. 233–241).

**Not quotable.** Annex H asks *may a tool rewrite this*, not *does this
vary from one exposure to the next*, and treating `Freeze 1` / `Freeze 2`
as "steady across a run" is a reading rather than a citation. It holds
for the body, the lens and the firmware. It fails for exposure time,
aperture, ISO and focal length — all `Freeze 1`, all changing frame to
frame under auto-exposure. Naming them splits a run into its frames;
leaving them out merges runs shot on one camera. Both are defensible and
neither is the specification's doing, which is why the choice is yours
and not the server's. Nothing published classifies EXIF tags by
burst-stability: MWG reconciles containers, IPTC PMD defines properties,
XMP `xmpMM` versions documents, and C2PA's bindings are built *not* to
match related frames.

**Do not key on `ImageUniqueID` (`exif:0xa420`) alone**, however it
reads: it is the one `Freeze 0` tag, which sounds like the identifier
this axis wants. Two library managers found otherwise — one accepts it
only when the value is UUID-shaped, having found cameras writing their
model name into the field; the other declined it as missing, reused and
inconsistently written across vendors. Name something beside it.

Apple's `BurstUUID` — the one signal that really does name a burst —
lives inside the maker note, which this build stores as one opaque
value. No path reaches into it today.

## Registering one

```
GET    /asterism/series-strategies         list what is registered
POST   /asterism/series-strategies         register (body: the shape above)
PATCH  /asterism/series-strategies/{id}    partial update; omitted fields stay
DELETE /asterism/series-strategies/{id}    remove the rule and its keys
```

`POST` answers with the rule as stored, including the `id` its keys are
filed under. Editing `applies_to` / `decode` / `include` / `exclude`
throws away every key derived under the rule and re-derives them in the
background; editing `name` costs nothing. Re-derivation reads rows, never
files, so iterating on a rule is cheap by design — write one, look at
what it grouped, change it.

Rules seeded by a migration are marked `system: true`. That is
provenance, not permission: edit or delete them like any other.

## What is not here yet

Reading the groups back. This surface registers rules; no endpoint yet
answers "which materials did this rule put on which key".
"#;

#[tool_handler]
impl ServerHandler for AsterismMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        );
        info.server_info.name = "asterism".into();
        info.server_info.version = env!("CARGO_PKG_VERSION").into();
        info.instructions = Some(
            "Asterism is a local-first provenance ledger for AI-generated \
             and personal media. Start with `catalog_overview` to learn \
             the persona / modality / tag / group vocabulary, then \
             `asset_search` or `asset_list` to find assets, `asset_get` \
             for one asset's full record, and `asset_lineage` to trace \
             a derivation chain. `asset_add` ingests, and \
             `asset_comment_add` writes to an asset's comment thread as \
             yourself. The full HTTP API (100+ routes) lives under \
             /asterism/* on the same loopback port that serves /mcp. \
             Read `asterism://guides/onboarding` for the recommended \
             flow and stability notes."
                .into(),
        );
        info
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        // A Vec so that adding one is a push rather than a shape
        // change — which is what the second entry turned out to be.
        let resources = vec![
            Resource::new(GUIDE_ONBOARDING_URI, "onboarding")
                .with_title(GUIDE_ONBOARDING_TITLE)
                .with_description(
                    "Recommended tool flow and stability notes for the Asterism MCP surface.",
                )
                .with_mime_type("text/markdown"),
            Resource::new(SCHEMA_SERIES_STRATEGY_URI, "series-strategy")
                .with_title(SCHEMA_SERIES_STRATEGY_TITLE)
                .with_description(
                    "How to write and register a series Strategy: the closed decoder set, \
                     path grammar, and which of include / exclude to reach for.",
                )
                .with_mime_type("text/markdown"),
        ];
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        match request.uri.as_str() {
            GUIDE_ONBOARDING_URI => Ok(ReadResourceResult::new(vec![
                ResourceContents::text(GUIDE_ONBOARDING_BODY, GUIDE_ONBOARDING_URI)
                    .with_mime_type("text/markdown"),
            ])
            .into()),
            SCHEMA_SERIES_STRATEGY_URI => Ok(ReadResourceResult::new(vec![
                ResourceContents::text(SCHEMA_SERIES_STRATEGY_BODY, SCHEMA_SERIES_STRATEGY_URI)
                    .with_mime_type("text/markdown"),
            ])
            .into()),
            other => Err(McpError::resource_not_found(
                format!("unknown resource uri: {other}"),
                None,
            )),
        }
    }
}

/// Builds the streamable-http tower service the axum router nests at
/// `/mcp`.
///
/// `json_response = true` answers plain request/response tool calls
/// with `application/json` (the SSE stream is still used when a
/// handler emits notifications first — rmcp falls back automatically).
pub fn streamable_service(
    ctx: Arc<ServerCtx>,
) -> StreamableHttpService<AsterismMcp, LocalSessionManager> {
    let mut config = StreamableHttpServerConfig::default();
    config.json_response = true;
    StreamableHttpService::new(
        move || Ok(AsterismMcp::new(ctx.clone())),
        LocalSessionManager::default().into(),
        config,
    )
}
