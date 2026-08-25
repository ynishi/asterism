# asterism-ui::commands

Tauri command handlers — a thin translation layer. They pass DTOs
through to the application services in `asterism-core` and convert
`DomainError` into `UiError`. No business logic lives here.

Every mutation that writes to *this machine* names its attribution
channel explicitly — [`AttributionContext::owner_surface`]. This is
the owner's own operation surface (the desktop app's IPC), so the
owner-ness is a property of the surface rather than a guess about the
caller, and the commands carry no attribution fields for it to read.
The argument is required by the service signatures, so a new
mutation cannot be added here without choosing.

The qualifier is #153's and covers exactly one block, at the end of
this file: the verbs that write to a **team**. There the author is
the authenticated member and the team's server stamps it, so a
context stated here would be a second answer to a settled question.
Two of those verbs — connecting and publishing — write nothing
through a service that takes a context, and `publish_line_to_team`
additionally writes local relation rows, which carry no actor at
all. The block says all of this where it sits.

# This surface and the HTTP one are mirrors

Not "mostly overlapping" — a verb on one belongs on the other, in the
same change. `asterism-server`'s `http` module doc states the rule
and the two differences that are by design: attribution, and where
the id comes from.

MCP is not part of that obligation. It is curated on purpose, which
its own module doc explains.

## Functions

- `accept_tag_suggestion` — Accepts one tag suggestion (#112): the ruling lands on the
- `active_profile` — Returns the active local data profile for persistent UI chrome.
- `add_asset` — Ingests an asset (entry point for the asset-add pipeline).
- `add_asset_batch` — Ingests a batch of assets (bulk form of `add_asset`).
- `add_asset_to_group` — Idempotent add of an asset to a Group.
- `amend_forge_message` — Corrects something said.
- `append_thread_message` — Appends one Message. UI-side callers pass `author_kind = "human"`.
- `archive_forge_line` — Finished with. Takes no landing until it is reopened, and is the
- `archive_persona` — Toggles a persona's archive flag.
- `archive_thread` — Toggles the archived flag.
- `asset_constellation` — Returns the fully-resolved hover-burst payload — each edge with
- `asset_declare_meta` — Records — or removes — one AlbumMeta statement on an asset: the
- `asset_declare_provenance` — Declares (or repairs) an asset's origin after the fact — the
- `asset_declare_source_type` — Asserts — or retracts, via an absent `source_type` — the asset's
- `asset_detail` — Detail view (asset + attached tags + constellation edges).
- `asset_edges` — Returns the top-`limit` edges (by weight) for hover-burst
- `asset_lineage` — Walks the whole `derived_from` chain around the asset, not just
- `asset_provenance` — Returns the 1-hop `derived_from` lineage around the asset —
- `asset_source_type` — What the asset's source type currently rests on — the read twin of
- `asset_texts` — Resolves the full source text of each asset (session Reader
- `asset_video_preview` — Where a video's transcoded preview rendition stands — the command
- `attach_tag` — Attaches a tag to an asset by name (creates the tag row on first
- `attach_tag_batch` — Attaches one tag to many assets in one call (grid multi-select).
- `batch_group_membership` — Bulk attach / detach of asset↔group pairs. Returns
- `clone_shared_entry` — Takes a copy of one entry of a shared line (#148 decision 10).
- `close_forge_pursuit` — Ends the work, and puts what it says on the line if it says
- `connect_team_server` — Logs this window in to a team server and holds the session.
- `create_dir` — Creates a Dir under the given persona.
- `create_dispatch` — Kicks off one exporter run against a Selection. The apalis
- `create_group` — Creates a Group under the given persona.
- `create_material_layer` — Opens a band the person owns. Never the default — see
- `create_modality` — Registers a new modality master row.
- `create_query_group` — "Save as Group": mints a `kind='query'` Group from a `query_json`
- `create_series_strategy` — Registers a series rule and asks for the keys it implies — the
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
- `delete_series_strategy` — Removes a series rule and, by the schema's cascade, every key
- `delete_session` — Deletes a Session — only when no `asset` row still references
- `delete_tag` — Drops a tag channel and every link to it — the command twin of
- `delete_thread` — Deletes a Thread (cascades to messages).
- `delete_thread_message` — Deletes one Message (misfire correction).
- `detach_tag` — Removes a tag from an asset. Idempotent — a missing link is a
- `detach_tag_batch` — Detaches one tag from many assets in one call.
- `discard_forge_line` — Takes the line, its history and every piece of work against it.
- `disconnect_team_server` — Drops the session. The panel goes empty rather than stale.
- `dispatch_run` — Live-source dispatch (`dispatch_run`): freezes a Group (query
- `edit_asset_comment` — Rewrites the body of an existing comment (stamps `edited_at`).
- `edit_chapter_mark` — Retitles a section and, unlike the mark face, may move it: the reason
- `edit_material_mark` — Rewrites the body of an existing mark (stamps `edited_at`). The
- `empty_trash` — Permanently deletes every asset in the trash. Irreversible, and
- `get_asset_thumb` — Returns the cached JPEG bytes of a thumbnail for `asset_id` at
- `get_asset_thumbs` — The same thing for a whole screenful: cached JPEG bytes for each of
- `get_dispatch` — Fetches a dispatch job by id — used by the poll loop that drives
- `get_forge_line` — The line and its whole history.
- `get_forge_line_states` — What is on the line, folded from the chain.
- `get_forge_pursuit` — The work, whole — one read rather than the line's two.
- `get_forge_pursuit_behind` — The landings this work has not seen, oldest first.
- `get_forge_pursuit_collisions` — What this work still asks for that the line has moved since.
- `get_forge_thread` — The conversation, whole — every message and every correction to
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
- `list_forge_lines` — Every line, without its history.
- `list_forge_pursuit_children` — Work opened from this work.
- `list_forge_pursuits_of_line` — Every piece of work against a line, open and ended alike.
- `list_forge_strategies` — Every rule a line can be pointed at, built from the rules this
- `list_forge_threads_about` — Conversations about one thing in the forge — the work as a whole,
- `list_format_asset_counts` — Sidebar FORMAT facet counts (asset-model v4) — `(format, count)`
- `list_group_links` — Every Group-in-Group connection in scope — the UI builds the
- `list_groups` — Sidebar Groups section.
- `list_material_layers` — Lists every band over `asset_id`'s material, each with the chapters
- `list_material_marks` — Lists every mark in `asset_id`'s material, in the material's own
- `list_modalities` — Modality master listing — one row per registered modality (hidden
- `list_modality_asset_counts` — Sidebar Modality counts — `(modality_slug, asset_count)`, optionally
- `list_observations` — Every observation stream on one timeline, newest first — the
- `list_persona_asset_counts` — Sidebar Persona counts — `(persona_id, asset_count)` per persona
- `list_personas` — Lists every persona (used to render the sidebar).
- `list_series_strategies` — Every registered series rule, oldest first, seeded and user-written
- `list_sessions` — Sessions view — one row per `session_id` in the query scope.
- `list_settings` — Every known application setting, resolved through code default →
- `list_shared_lines` — Every line a team hosts, without its history.
- `list_snapshots_containing` — Reverse lookup — every Snapshot whose asset_ids list contains this
- `list_streams` — The stream names [`list_observations`]'s `stream` filter accepts —
- `list_tag_counts` — Sidebar Tags section — every tag paired with the number of
- `list_tag_suggestions` — Lists what the bound visual model proposed for one asset (#112),
- `list_thread_messages` — Lists the Messages of a Thread.
- `list_threads` — Lists Threads under the given anchor, freshest first. Archived
- `merge_assets` — The manual merge verb: a person's ruling that a set of rows is one
- `merge_groups` — Merges one manual group into another and deletes the source
- `merge_tags` — Folds one tag channel into another and deletes the source — the
- `move_dir` — Re-parents a Dir (`None` = to the root); cycle-guarded.
- `move_group_to_dir` — Files a Group under a Dir (`None` = back to the root).
- `open_forge_line` — Opens a line.
- `open_forge_pursuit` — Opens work against a line.
- `open_forge_thread` — Opens a conversation about something in the forge.
- `organize_by_location` — Backfill: auto-organises existing assets under a Dir tree derived
- `paste_image_import` — Writes a clipboard-pasted image blob to
- `patch_session_metadata` — Partially updates a Session's metadata (`title` / `note` /
- `post_asset_comment` — Posts a new comment. See [`PostAssetCommentCommand`] for the
- `post_chapter_mark` — Adds a section to a band the person owns.
- `post_material_mark` — Places a new mark. See [`PostMaterialMarkCommand`] for the anchor and
- `promote_snapshot_to_group` — Promotes a Snapshot into a hand-owned Group (mirror of
- `promote_tag_to_group` — Snapshots every asset carrying a tag into a newly-created Group.
- `promote_volatile_selection` — Fuses freeze + promote for the grid's volatile pick (W5-d):
- `publish_line_to_team` — Seeds a team's line from a local one (#148 decision 11).
- `pull_tag_head` — Enqueues a `HeadPull` install of a fetched head artifact — the
- `purge_asset` — Permanently deletes an already-trashed asset. Conflicts when the
- `purge_group` — Permanently deletes an already-trashed Group (cascades the m:n
- `purge_persona` — Permanently deletes an already-trashed persona and everything it
- `push_forge_round` — Writes a round.
- `random_assets` — A random handful out of the current filter — the sidebar's
- `rebuild_edges` — Enqueues an incremental constellation-edge rebuild for the asset.
- `rebuild_index` — Enqueues a batch `IndexRebuild` job and returns its task id — the
- `rebuild_sessions` — Enqueues a `SessionRebuild` job. The precomputed rkyv snapshot
- `record_diag` — Appends one webview-origin diagnostic to `diag_log` — the capture
- `record_event` — Appends one telemetry event to the local `event_log`. Fire-and-
- `redispatch` — Re-runs a finished dispatch with the same frozen input (P2).
- `register_persona` — Registers a new persona.
- `rehome_dropped_path` — Rehomes a dropped path into `$HOME/asterism/dropped/`
- `reject_tag_suggestion` — Rejects one tag suggestion (#112); this model never proposes the
- `remeasure_dims` — Re-reads artefacts and rewrites `width_px` / `height_px` — the
- `remove_asset_from_group` — Idempotent remove of an asset from a Group.
- `rename_dir` — Renames a Dir.
- `rename_forge_line` — Moves the line's own description. Not a landing: nothing goes on the
- `rename_forge_thread` — Names the conversation, or takes its name off.
- `rename_group` — Renames a Group.
- `rename_session` — Renames a Session (title-only write). Passing `title: null`
- `rename_tag` — Renames a tag channel in place — the command twin of
- `reopen_forge_line` — Takes it back out.
- `reorder_group_assets` — Rewrites the front-to-back order of a Group's assets after a drag.
- `reorder_group_children` — Rewrites the order of a Group's child groups.
- `reorder_personas` — Rewrites `display_order` across a persona slice.
- `rescan_duplicates` — Re-derives duplicate conflicts from fingerprints already on the
- `reset_setting` — Clears one setting override and returns the value that now applies.
- `resolve_duplicate_conflict` — Answers one duplicate question — `folded` (queues the fold onto
- `resolve_forge_pursuit` — Lets the line's rule answer whatever this work collides with.
- `restore_asset` — Returns a trashed asset to the live set.
- `restore_group` — Returns a trashed Group to the sidebar.
- `restore_persona` — Returns a trashed persona and the assets that went with it.
- `say_in_forge_thread` — Says something.
- `search_asset_ids` — The same retrieval as `search_assets`, reduced to the rank order.
- `search_assets` — Full-text / fuzzy search.
- `set_default_material_layer` — Chooses the band the panel shows, and the one a new mark lands in.
- `set_forge_line_strategy` — Points the line at a different rule, from here on.
- `set_persona_profile` — Upserts the persona's identity signal.
- `set_persona_theme` — Sets (or clears) the wallpaper for a persona.
- `set_setting` — Stores one setting override and returns the value that now applies.
- `shared_line_history` — A shared line and its whole history.
- `shared_line_states` — What is on a shared line, folded from its chain by the server.
- `snapshot_members` — Snapshot view members (`snapshot_members`): renderable cards
- `team_server_session` — Whether this window is talking to a team server.
- `train_tag_head` — Enqueues a `HeadTrain` run over the rulings under the bound
- `trash_asset` — Moves an asset to the trash (reversible).
- `trash_group` — Moves a Group to the trash (reversible; membership and drag order
- `trash_persona` — Moves a persona and every asset it holds to the trash (reversible).
- `unlink_group` — Removes a Group-in-Group connection.
- `update_asset_meta` — Partially updates an asset's metadata.
- `update_asset_meta_batch` — Partially updates metadata for multiple assets in one call.
- `update_modality` — Partially updates a modality master row (each omitted field is left
- `update_query_group_query` — "Update query": validates + persists a replacement rule (rejecting
- `update_series_strategy` — Partially updates a series rule (each omitted field is left
- `visual_model_status` — Which visual model this process bound, if any (#112).

