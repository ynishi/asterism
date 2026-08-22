# asterism-ui::commands

Tauri command handlers — a thin translation layer. They pass DTOs
through to the application services in `asterism-core` and convert
`DomainError` into `UiError`. No business logic lives here.

Every mutation here names its attribution channel explicitly —
[`AttributionContext::owner_surface`]. This is the owner's own
operation surface (the desktop app's IPC), so the owner-ness is a
property of the surface rather than a guess about the caller, and the
commands carry no attribution fields for it to read. The argument is
required by the service signatures, so a new mutation cannot be added
here without choosing.

## Functions

- `accept_tag_suggestion` — Accepts one tag suggestion (#112): the ruling lands on the
- `active_profile` — Returns the active local data profile for persistent UI chrome.
- `add_asset` — Ingests an asset (entry point for the asset-add pipeline).
- `add_asset_batch` — Ingests a batch of assets (bulk form of `add_asset`).
- `add_asset_to_group` — Idempotent add of an asset to a Group.
- `append_thread_message` — Appends one Message. UI-side callers pass `author_kind = "human"`.
- `archive_persona` — Toggles a persona's archive flag.
- `archive_thread` — Toggles the archived flag.
- `asset_constellation` — Returns the fully-resolved hover-burst payload — each edge with
- `asset_declare_meta` — Records — or removes — one AlbumMeta statement on an asset: the
- `asset_declare_provenance` — Declares (or repairs) an asset's origin after the fact — the
- `asset_detail` — Detail view (asset + attached tags + constellation edges).
- `asset_edges` — Returns the top-`limit` edges (by weight) for hover-burst
- `asset_lineage` — Walks the whole `derived_from` chain around the asset, not just
- `asset_provenance` — Returns the 1-hop `derived_from` lineage around the asset —
- `asset_texts` — Resolves the full source text of each asset (session Reader
- `asset_video_preview` — Where a video's transcoded preview rendition stands — the command
- `attach_tag` — Attaches a tag to an asset by name (creates the tag row on first
- `attach_tag_batch` — Attaches one tag to many assets in one call (grid multi-select).
- `batch_group_membership` — Bulk attach / detach of asset↔group pairs. Returns
- `create_dir` — Creates a Dir under the given persona.
- `create_dispatch` — Kicks off one exporter run against a Selection. The apalis
- `create_group` — Creates a Group under the given persona.
- `create_material_layer` — Opens a band the person owns. Never the default — see
- `create_modality` — Registers a new modality master row.
- `create_query_group` — "Save as Group": mints a `kind='query'` Group from a `query_json`
- `create_snapshot` — Freezes a picked asset list into a Snapshot and stops there
- `create_thread` — Creates a Thread.
- `delete_asset_comment` — Deletes a comment. Idempotent.
- `delete_chapter_mark` — Removes one section. **Not** idempotent, unlike deleting a mark: a
- `delete_dir` — Deletes an **empty** Dir.
- `delete_material_layer` — Deletes a band the person owns, with everything in it. Refuses an
- `delete_material_mark` — Deletes a mark. Idempotent.
- `delete_modality` — Deletes a modality master row — only when no asset carries the slug
- `delete_persona_profile` — Removes the persona profile row entirely.
- `delete_persona_theme` — Removes the persona theme row entirely (reverts to defaults).
- `delete_session` — Deletes a Session — only when no `asset` row still references
- `delete_thread` — Deletes a Thread (cascades to messages).
- `delete_thread_message` — Deletes one Message (misfire correction).
- `detach_tag` — Removes a tag from an asset. Idempotent — a missing link is a
- `detach_tag_batch` — Detaches one tag from many assets in one call.
- `dispatch_run` — Live-source dispatch (`dispatch_run`): freezes a Group (query
- `edit_asset_comment` — Rewrites the body of an existing comment (stamps `edited_at`).
- `edit_chapter_mark` — Retitles a section and, unlike the mark face, may move it: the reason
- `edit_material_mark` — Rewrites the body of an existing mark (stamps `edited_at`). The
- `empty_trash` — Permanently deletes every asset in the trash. Irreversible, and
- `get_asset_thumb` — Returns the cached JPEG bytes of a thumbnail for `asset_id` at
- `get_asset_thumbs` — The same thing for a whole screenful: cached JPEG bytes for each of
- `get_dispatch` — Fetches a dispatch job by id — used by the poll loop that drives
- `get_persona_profile` — Fetches the persona's identity signal (avatar / bio / role).
- `get_persona_theme` — Fetches the persona's UI chrome (wallpaper reference). `None`
- `get_session` — Fetches one Session by surrogate id (`None` when absent). Used
- `get_snapshot` — Snapshot view metadata (`snapshot_get`): the freeze's id,
- `get_thread` — Fetches one Thread by id.
- `groups_of_asset` — Which Groups the asset already sits in — powers the "already
- `hydrate_cards` — Batch-hydrates cards by id. Companion to `list_asset_index` —
- `jobs_stats` — Snapshot of the apalis `Jobs` table used by the UI progress
- `link_group` — Connects a Group into another Group (cycle- / persona-guarded).
- `list_asset_comments` — Lists every comment on `asset_id` in chronological order.
- `list_asset_index` — Index-only grid listing for 6-figure result sets. Returns
- `list_assets` — Lists assets for the grid (returns `AssetCard` projections).
- `list_chapter_marks` — Lists the sections in one band, in the reading order the band states
- `list_color_asset_counts` — Sidebar COLOR facet counts — `(bucket, count)` per palette swatch
- `list_dirs` — Sidebar Dir tree (flat `parent_id` list; the UI assembles it).
- `list_dispatch` — Lists dispatch jobs with the same predicate surface as the HTTP
- `list_duplicate_conflicts` — The duplicate questions still waiting on a person, newest first,
- `list_duplicate_groups` — Duplicate report — sets of live assets sharing a fingerprint on
- `list_events` — Newest-first telemetry listing (kind / time-window filters). Feeds
- `list_exporters` — Registered exporter slugs — the action bar renders one row per
- `list_format_asset_counts` — Sidebar FORMAT facet counts (asset-model v4) — `(format, count)`
- `list_group_links` — Every Group-in-Group connection in scope — the UI builds the
- `list_groups` — Sidebar Groups section.
- `list_material_layers` — Lists every band over `asset_id`'s material, each with the chapters
- `list_material_marks` — Lists every mark in `asset_id`'s material, in the material's own
- `list_modalities` — Modality master listing — one row per registered modality (hidden
- `list_modality_asset_counts` — Sidebar Modality counts — `(modality_slug, asset_count)`, optionally
- `list_persona_asset_counts` — Sidebar Persona counts — `(persona_id, asset_count)` per persona
- `list_personas` — Lists every persona (used to render the sidebar).
- `list_sessions` — Sessions view — one row per `session_id` in the query scope.
- `list_settings` — Every known application setting, resolved through code default →
- `list_snapshots_containing` — Reverse lookup — every Snapshot whose asset_ids list contains this
- `list_tag_counts` — Sidebar Tags section — every tag paired with the number of
- `list_tag_suggestions` — Lists what the bound visual model proposed for one asset (#112),
- `list_thread_messages` — Lists the Messages of a Thread.
- `list_threads` — Lists Threads under the given anchor, freshest first. Archived
- `merge_assets` — The manual merge verb: a person's ruling that a set of rows is one
- `merge_groups` — Merges one manual group into another and deletes the source
- `move_dir` — Re-parents a Dir (`None` = to the root); cycle-guarded.
- `move_group_to_dir` — Files a Group under a Dir (`None` = back to the root).
- `paste_image_import` — Writes a clipboard-pasted image blob to
- `patch_session_metadata` — Partially updates a Session's metadata (`title` / `note` /
- `post_asset_comment` — Posts a new comment. See [`PostAssetCommentCommand`] for the
- `post_chapter_mark` — Adds a section to a band the person owns.
- `post_material_mark` — Places a new mark. See [`PostMaterialMarkCommand`] for the anchor and
- `promote_snapshot_to_group` — Promotes a Snapshot into a hand-owned Group (mirror of
- `promote_tag_to_group` — Snapshots every asset carrying a tag into a newly-created Group.
- `promote_volatile_selection` — Fuses freeze + promote for the grid's volatile pick (W5-d):
- `purge_asset` — Permanently deletes an already-trashed asset. Conflicts when the
- `purge_group` — Permanently deletes an already-trashed Group (cascades the m:n
- `purge_persona` — Permanently deletes an already-trashed persona and everything it
- `random_assets` — A random handful out of the current filter — the sidebar's
- `rebuild_edges` — Enqueues an incremental constellation-edge rebuild for the asset.
- `rebuild_sessions` — Enqueues a `SessionRebuild` job. The precomputed rkyv snapshot
- `record_diag` — Appends one webview-origin diagnostic to `diag_log` — the capture
- `record_event` — Appends one telemetry event to the local `event_log`. Fire-and-
- `redispatch` — Re-runs a finished dispatch with the same frozen input (P2).
- `register_persona` — Registers a new persona.
- `rehome_dropped_path` — Rehomes a dropped path into `~/Pictures/Asterism/dropped/`
- `reject_tag_suggestion` — Rejects one tag suggestion (#112); this model never proposes the
- `remove_asset_from_group` — Idempotent remove of an asset from a Group.
- `rename_dir` — Renames a Dir.
- `rename_group` — Renames a Group.
- `rename_session` — Renames a Session (title-only write). Passing `title: null`
- `reorder_group_assets` — Rewrites the front-to-back order of a Group's assets after a drag.
- `reorder_group_children` — Rewrites the order of a Group's child groups.
- `reorder_personas` — Rewrites `display_order` across a persona slice.
- `reset_setting` — Clears one setting override and returns the value that now applies.
- `resolve_duplicate_conflict` — Answers one duplicate question — `folded` (queues the fold onto
- `restore_asset` — Returns a trashed asset to the live set.
- `restore_group` — Returns a trashed Group to the sidebar.
- `restore_persona` — Returns a trashed persona and the assets that went with it.
- `search_asset_ids` — The same retrieval as `search_assets`, reduced to the rank order.
- `search_assets` — Full-text / fuzzy search.
- `set_default_material_layer` — Chooses the band the panel shows, and the one a new mark lands in.
- `set_persona_profile` — Upserts the persona's identity signal.
- `set_persona_theme` — Sets (or clears) the wallpaper for a persona.
- `set_setting` — Stores one setting override and returns the value that now applies.
- `snapshot_members` — Snapshot view members (`snapshot_members`): renderable cards
- `trash_asset` — Moves an asset to the trash (reversible).
- `trash_group` — Moves a Group to the trash (reversible; membership and drag order
- `trash_persona` — Moves a persona and every asset it holds to the trash (reversible).
- `unlink_group` — Removes a Group-in-Group connection.
- `update_asset_meta` — Partially updates an asset's metadata.
- `update_asset_meta_batch` — Partially updates metadata for multiple assets in one call.
- `update_modality` — Partially updates a modality master row (each omitted field is left
- `update_query_group_query` — "Update query": validates + persists a replacement rule (rejecting
- `visual_model_status` — Which visual model this process bound, if any (#112).

