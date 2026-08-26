# asterism-core::application::mapping

Conversion between domain types and contract DTOs.

`asterism-contract` is a leaf crate and does not know the domain types;
every conversion goes through this module. Wire-representation rules
(id = UUID hyphenated string, timestamps = unix epoch milliseconds, and
so on) live in `asterism-contract`'s crate docs.

## Functions

- `asset_comment_to_dto` — Converts an `AssetComment` domain entity to its wire DTO.
- `asset_to_dto` — Converts an `Asset` entity to an `AssetDto` (every field, used for the detail view).
- `card_to_dto` — Converts an `AssetCard` projection to an `AssetCardDto`. Search
- `card_to_dto_with_hit` — Same shape as [`card_to_dto`] but populates the search-only
- `chapter_mark_to_dto` — Converts a `ChapterMark` domain entity to its wire DTO.
- `detail_to_dto` — Bundles an asset with its tags and edges as an `AssetDetailDto`.
- `dir_to_dto` — Converts a `Dir` to its wire DTO.
- `dispatch_to_dto` — Converts a `DispatchJob` domain entity to its wire DTO.
- `edge_to_dto` — Converts a `ConstellationEdge` to an `EdgeDto`.
- `effective_setting_to_dto` — Converts a resolved setting to its wire DTO, projecting the whole
- `entity_ref_to_dto` — Converts a domain [`EntityRef`] to its wire DTO.
- `forge_anchored` — Reads which thing a conversation is about.
- `forge_body` — Reads something somebody said, refusing an empty one.
- `forge_collisions_to_dto` — Converts what work still collides with to what a screen reads
- `forge_discarded_to_dto` — Converts what a drop released.
- `forge_history_to_dto` — Converts a line and its whole chain, in the chain's order.
- `forge_line_id` — Reads a line id off the wire.
- `forge_line_to_dto` — Converts a `Line` to the summary a caller reads.
- `forge_message_id` — Reads a message id off the wire.
- `forge_message_to_dto` — Converts one message, with what it says now beside what it said
- `forge_name` — Reads a name off the wire, refusing a blank one.
- `forge_op` — Reads one operation off the wire.
- `forge_outcome` — Reads how a caller says work ended.
- `forge_pursuit_id` — Reads a pursuit id off the wire.
- `forge_pursuit_to_dto` — Converts a piece of work to what a caller reads: how it opened,
- `forge_revision_to_dto` — Converts one correction.
- `forge_round_to_dto` — Converts one round to what a caller reads.
- `forge_states_to_dto` — Converts the fold of a line's chain to what is on it.
- `forge_strategy_id` — Reads a strategy id off the wire.
- `forge_strategy_to_dto` — Converts a rule's id and description to what a chooser reads.
- `forge_thread_id` — Reads a thread id off the wire.
- `forge_thread_to_dto` — Converts a conversation to what a caller reads: every message and
- `group_link_to_dto` — Converts a `GroupLink` to its wire DTO.
- `group_summary_to_dto` — Converts a `GroupSummary` to its wire DTO.
- `group_to_dto` — Converts a `Group` to its wire DTO.
- `head_status_to_dto` — Assembles the model panel's head status (#130) out of the three
- `index_page_to_dto` — Converts a `Page<AssetIndex>` to an `AssetIndexPageDto`.
- `index_to_dto` — Converts an `AssetIndex` projection to its wire form.
- `material_layer_to_dto` — Converts a `MaterialLayer` domain entity to its wire DTO.
- `material_mark_to_dto` — Converts a `MaterialMark` domain entity to its wire DTO.
- `message_to_dto` — Converts a domain [`Message`] to its wire DTO.
- `modality_view_to_dto` — Converts a `ModalityView` (master row + live asset count) to its
- `page_to_dto` — Converts a `Page<AssetCard>` to an `AssetPageDto`.
- `parse_asset_comment_id` — Parses the wire representation of an asset-comment id.
- `parse_asset_id` — Parses the wire representation of an asset id.
- `parse_chapter_mark_id` — Parses the wire representation of a chapter-mark id.
- `parse_dir_id` — Parses the wire representation of a dir id.
- `parse_dispatch_id` — Parses the wire representation of a dispatch id.
- `parse_group_id` — Parses the wire representation of a group id.
- `parse_layer_role` — Parses the wire spelling of a layer role.
- `parse_material_layer_id` — Parses the wire representation of a material-layer id.
- `parse_material_mark_id` — Parses the wire representation of a material-mark id.
- `parse_message_id` — Parses the wire representation of a message id.
- `parse_message_ref` — Parses one wire `MessageRefDto` chip into an [`EntityRef`].
- `parse_ms` — Parses a unix-epoch-milliseconds timestamp from the wire (returns a
- `parse_persona_id` — Parses the wire representation of a persona id.
- `parse_snapshot_id` — Parses the wire representation of a snapshot id.
- `parse_tag_id` — Parses the wire representation of a tag id.
- `parse_thread_anchor` — Parses a `(anchor_kind, anchor_id)` wire pair into a
- `parse_thread_id` — Parses the wire representation of a thread id.
- `parse_timeline_span` — Lifts a wire `(start_ms, end_ms)` pair onto the domain's timeline.
- `parse_uuid` — Parses a UUID from the wire representation (returns a validation error
- `persona_profile_to_dto` — Converts a `PersonaProfile` domain object to a `PersonaProfileDto`.
- `persona_theme_to_dto` — Converts a `PersonaTheme` domain object to a `PersonaThemeDto`.
- `persona_to_dto` — Converts a `Persona` domain object to a `PersonaDto`.
- `registered_strategy_to_dto` — Converts a registered series Strategy to its wire DTO.
- `session_page_to_dto` — Converts a `Page<Session>` (the shape now returned by
- `session_to_dto` — Converts a `Session` entity to its wire DTO. Every field maps
- `snapshot_to_dto` — Converts a `Snapshot` domain entity to its wire DTO (contract
- `tag_count_to_dto` — Converts a `TagCount` to a `TagCountDto`.
- `tag_to_dto` — Converts a `Tag` to a `TagDto`.
- `thread_anchor_to_dto` — Converts a domain [`ThreadAnchor`] to its wire DTO.
- `thread_to_dto` — Converts a domain [`Thread`] to its wire DTO.
- `to_asset_query` — Converts the wire `ListAssetsQuery` to a domain `AssetQuery`. When

