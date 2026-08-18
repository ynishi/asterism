//! Conversion between domain types and contract DTOs.
//!
//! `asterism-contract` is a leaf crate and does not know the domain types;
//! every conversion goes through this module. Wire-representation rules
//! (id = UUID hyphenated string, timestamps = unix epoch milliseconds, and
//! so on) live in `asterism-contract`'s crate docs.

use asterism_contract::dto::{
    AssetCardDto, AssetCommentDto, AssetDetailDto, AssetDto, AssetPageDto, ChapterMarkDto, DirDto,
    DispatchDto, EdgeDto, GroupDto, GroupLinkDto, GroupSummaryDto, MaterialLayerDto,
    MaterialMarkDto, MessageDto, MessageRefDto, ModalityDefDto, PersonaDto, PersonaProfileDto,
    PersonaThemeDto, SeriesStrategyDto, SessionDto, SessionPageDto, SettingDto, SettingLayerDto,
    SnapshotDto, TagCountDto, TagDto, ThreadAnchorDto, ThreadDto,
};
use asterism_contract::query::ListAssetsQuery;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::app_setting::EffectiveSetting;
use crate::domain::asset::{Asset, AssetCard, AssetQuery, TrashFilter, UNCLASSIFIED_MODALITY};
use crate::domain::asset_comment::AssetComment;
use crate::domain::chapter_mark::ChapterMark;
use crate::domain::color::ColorBucket;
use crate::domain::dir::Dir;
use crate::domain::dispatch::{DispatchJob, DispatchState};
use crate::domain::edge::ConstellationEdge;
use crate::domain::group::{Group, GroupLink, GroupSummary};
use crate::domain::material_layer::{LayerRole, MaterialLayer};
use crate::domain::material_mark::{MaterialAnchor, MaterialMark};
use crate::domain::modality::ModalityView;
use crate::domain::persona::Persona;
use crate::domain::persona_profile::PersonaProfile;
use crate::domain::persona_theme::PersonaTheme;
use crate::domain::render::render_policy;
use crate::domain::repository::RegisteredStrategy;
use crate::domain::series::Path as SeriesPath;
use crate::domain::session::Session;
use crate::domain::snapshot::Snapshot;
use crate::domain::tag::{Tag, TagCount};
use crate::domain::thread::{EntityRef, Message, Thread, ThreadAnchor};
use crate::domain::value::{
    AssetCommentId, AssetId, ChapterMarkId, DirId, DispatchId, GroupId, Label, MaterialLayerId,
    MaterialMarkId, MessageId, MimeType, Modality, Page, PersonaId, SnapshotId, TagId, ThreadId,
    Viewer, Visibility,
};
use crate::error::DomainError;

/// Parses a UUID from the wire representation (returns a validation error
/// on malformed input).
pub fn parse_uuid(value: &str, field: &str) -> Result<Uuid, DomainError> {
    Uuid::parse_str(value)
        .map_err(|_| DomainError::Validation(format!("invalid uuid in {field}: {value:?}")))
}

/// Parses a unix-epoch-milliseconds timestamp from the wire (returns a
/// validation error if the value cannot be represented as `DateTime<Utc>`).
pub fn parse_ms(ms: i64, field: &str) -> Result<DateTime<Utc>, DomainError> {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .ok_or_else(|| DomainError::Validation(format!("timestamp out of range in {field}: {ms}")))
}

/// Parses the wire trash selector. `None` (omitted) is the live side,
/// so a caller that predates the trash cannot surface trashed rows.
/// An unrecognised value is rejected rather than defaulted: silently
/// treating a typo as "live" would hide the trash view, and treating it
/// as "any" would leak it.
pub(crate) fn parse_trash_filter(raw: Option<&str>) -> Result<TrashFilter, DomainError> {
    match raw {
        None | Some("live") => Ok(TrashFilter::LiveOnly),
        Some("trashed") => Ok(TrashFilter::TrashedOnly),
        Some("any") => Ok(TrashFilter::Any),
        Some(other) => Err(DomainError::Validation(format!(
            "unknown trash filter: {other:?} (expected \"live\", \"trashed\", or \"any\")"
        ))),
    }
}

/// Highest star rating the domain accepts. The write side clamps to it
/// (`AssetService::update_meta`); the read side rejects past it, because
/// a clamp on a filter bound would answer a different question from the
/// one asked.
const MAX_RATING: u8 = 5;

/// Parses one end of the star-rating band.
///
/// Out-of-range is an error rather than a clamp: `rating_min=7` clamped
/// to 5 would return the five-star assets and look like a working
/// filter, which is worse than being told the band does not exist.
fn parse_rating_bound(value: Option<u8>, field: &str) -> Result<Option<u8>, DomainError> {
    match value {
        Some(v) if v > MAX_RATING => Err(DomainError::Validation(format!(
            "{field} out of range: {v} (expected 0..={MAX_RATING})"
        ))),
        other => Ok(other),
    }
}

/// Rejects an inverted numeric band.
///
/// Shared by the three `min`/`max` axes because the reasoning is one
/// piece: an inverted band matches nothing by construction, so the empty
/// page it would produce reads as a claim about the library ("nothing is
/// that long") when the fault is in the request. The rating axis pairs
/// this with a range check; the two metric axes have no ceiling to check
/// against, so this is the whole of their validation.
fn reject_inverted_band<T: PartialOrd + std::fmt::Display>(
    min: Option<T>,
    max: Option<T>,
    min_field: &str,
    max_field: &str,
) -> Result<(), DomainError> {
    if let (Some(min), Some(max)) = (min, max)
        && min > max
    {
        return Err(DomainError::Validation(format!(
            "{min_field} {min} is above {max_field} {max}"
        )));
    }
    Ok(())
}

/// Converts the wire `ListAssetsQuery` to a domain `AssetQuery`. When
/// `viewer_subject` is `None`, the viewer defaults to `Owner`.
///
/// The numeric bands (rating, playback length, stored size) are validated
/// here rather than deeper down, so every transport that reaches a
/// listing — HTTP `GET`, Tauri IPC, a stored Query Group rule — gets the
/// same `400`-shaped answer for the same bad band.
pub fn to_asset_query(query: &ListAssetsQuery) -> Result<AssetQuery, DomainError> {
    let rating_min = parse_rating_bound(query.rating_min, "rating_min")?;
    let rating_max = parse_rating_bound(query.rating_max, "rating_max")?;
    reject_inverted_band(rating_min, rating_max, "rating_min", "rating_max")?;
    // The two metric bands carry no range check to go with the inversion
    // one: rating is bounded by the star widget, while length and size
    // have no ceiling the domain could name, so `u64` is the whole
    // definition set and a bound above the largest asset is a legitimate
    // (if empty) question rather than a malformed one.
    reject_inverted_band(
        query.duration_min_ms,
        query.duration_max_ms,
        "duration_min_ms",
        "duration_max_ms",
    )?;
    reject_inverted_band(
        query.size_min_bytes,
        query.size_max_bytes,
        "size_min_bytes",
        "size_max_bytes",
    )?;
    // The resolution band is a third of the same shape, and carries no
    // range check for the same reason: the count has no ceiling the
    // domain could name, so a bound above the largest picture in the
    // library is a legitimate (if empty) question. Note this bounds the
    // *product* — there is deliberately no width or height band, because
    // the columns hold coded dimensions and a band over either side
    // answers backwards for material stored rotated
    // (`ListAssetsQuery::pixels_min`).
    reject_inverted_band(
        query.pixels_min,
        query.pixels_max,
        "pixels_min",
        "pixels_max",
    )?;
    // `rating_max = 0` also matches nothing by construction: the write
    // side never stores a zero (`update_meta` treats rating 0 as "clear
    // the rating"), so stored ratings are `1..=5`. Same reasoning as the
    // inverted band — an always-empty band is a fact about the request,
    // not about the corpus. `rating_min = 0` stays legal: as a lower
    // bound it means "every rated asset".
    if rating_max == Some(0) {
        return Err(DomainError::Validation(
            "rating_max 0 matches no rated asset (stored ratings are 1..=5; rating 0 clears the rating)"
                .into(),
        ));
    }
    Ok(AssetQuery {
        viewer: match &query.viewer_subject {
            None => Viewer::Owner,
            Some(subject) => Viewer::Subject(subject.clone()),
        },
        persona_id: query
            .persona_id
            .as_deref()
            .map(|s| parse_uuid(s, "persona_id").map(PersonaId::from_uuid))
            .transpose()?,
        // The Unclassified bucket travels on the same wire field as a
        // real slug, using a sentinel the `Modality` newtype cannot
        // produce (`[a-z0-9_-]` rejects the leading `!`). Keeping it on
        // one field is what lets the sidebar treat "unclassified" as
        // just another row the user clicks.
        modality: match query.modality.as_deref() {
            Some(UNCLASSIFIED_MODALITY) | None => None,
            Some(slug) => Some(Modality::new(slug)?),
        },
        modality_unset: query.modality.as_deref() == Some(UNCLASSIFIED_MODALITY),
        occurred_from: query
            .occurred_from_ms
            .map(|ms| parse_ms(ms, "occurred_from_ms"))
            .transpose()?,
        occurred_until: query
            .occurred_until_ms
            .map(|ms| parse_ms(ms, "occurred_until_ms"))
            .transpose()?,
        // Ingest / modification windows. Each end is validated the same
        // way as the occurrence window — an unrepresentable epoch is a
        // `400` naming the field, not a bound quietly dropped — but an
        // inverted pair is deliberately *not* rejected; see
        // `ListAssetsQuery::created_from_ms`.
        created_from: query
            .created_from_ms
            .map(|ms| parse_ms(ms, "created_from_ms"))
            .transpose()?,
        created_until: query
            .created_until_ms
            .map(|ms| parse_ms(ms, "created_until_ms"))
            .transpose()?,
        updated_from: query
            .updated_from_ms
            .map(|ms| parse_ms(ms, "updated_from_ms"))
            .transpose()?,
        updated_until: query
            .updated_until_ms
            .map(|ms| parse_ms(ms, "updated_until_ms"))
            .transpose()?,
        tag_ids: query
            .tag_ids
            .iter()
            .map(|s| parse_uuid(s, "tag_ids").map(TagId::from_uuid))
            .collect::<Result<Vec<_>, _>>()?,
        // A closed enum with no parse step: serde already refused
        // anything outside the two variants at the wire boundary.
        tag_match: query.tag_match,
        group_ids: query
            .group_ids
            .iter()
            .map(|s| parse_uuid(s, "group_ids").map(GroupId::from_uuid))
            .collect::<Result<Vec<_>, _>>()?,
        // session-model v2: the wire drill key (kept named `session_id`
        // for wire back-compat) is the composite Asset id; members are
        // drilled via `container_id`. Migrated members carry both keys,
        // freshly-ingested members carry only `container_id`, so
        // `container_id` is the axis that lists both.
        container_id: query
            .session_id
            .as_deref()
            .map(parse_asset_id)
            .transpose()?,
        label: query.label.clone().map(Label::new).transpose()?,
        // Trimmed here so every consumer downstream can treat
        // `Some` as "there is a filter" without re-checking for a
        // blank the user left in the box.
        text_match: query
            .text_match
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        format: query.format.clone(),
        color: query.color.as_deref().map(ColorBucket::parse).transpose()?,
        rating_min,
        rating_max,
        // Checked against the write side's own shape. A key that no
        // statement could carry (uppercase, a dot) and a value that none
        // could hold (blank) are refused rather than answered with an
        // empty page, on the rule the rating band above states: an
        // always-empty answer is a fact about the request.
        album_meta_key: query
            .album_meta_key
            .as_deref()
            .map(crate::domain::album_meta::parse_key)
            .transpose()?,
        album_meta_value: query
            .album_meta_value
            .as_deref()
            .map(crate::domain::album_meta::parse_value)
            .transpose()?,
        // Carried across unchanged: the unit is the column's own on both
        // sides, and the only rule either axis has (inversion) was
        // applied above. Nothing here is type-checked into place —
        // `AssetQuery` is never destructured exhaustively, so a bound
        // dropped at this line compiles clean and answers every length /
        // size question with the unfiltered set. `metric_bands_reach_the_domain_query`
        // is what actually holds these four lines down.
        duration_min_ms: query.duration_min_ms,
        duration_max_ms: query.duration_max_ms,
        size_min_bytes: query.size_min_bytes,
        size_max_bytes: query.size_max_bytes,
        pixels_min: query.pixels_min,
        pixels_max: query.pixels_max,
        trash: parse_trash_filter(query.trash.as_deref())?,
        offset: query.offset,
        limit: query.limit,
    })
}

/// Converts a `Persona` domain object to a `PersonaDto`.
pub fn persona_to_dto(persona: &Persona) -> PersonaDto {
    PersonaDto {
        id: persona.id.to_string(),
        pack_id: persona.pack_id.as_ref().map(|p| p.as_str().to_string()),
        name: persona.name.clone(),
        accent_color: persona.accent_color.clone(),
        display_order: persona.display_order,
        archived: persona.archived,
        created_at_ms: persona.created_at.timestamp_millis(),
        updated_at_ms: persona.updated_at.timestamp_millis(),
    }
}

/// Converts a `PersonaProfile` domain object to a `PersonaProfileDto`.
pub fn persona_profile_to_dto(profile: &PersonaProfile) -> PersonaProfileDto {
    PersonaProfileDto {
        persona_id: profile.persona_id.to_string(),
        avatar_asset_id: profile.avatar_asset_id.as_ref().map(|a| a.to_string()),
        bio_short: profile.bio_short.clone(),
        role_tag: profile.role_tag.clone(),
        updated_at_ms: profile.updated_at.timestamp_millis(),
    }
}

/// Converts a `PersonaTheme` domain object to a `PersonaThemeDto`.
pub fn persona_theme_to_dto(theme: &PersonaTheme) -> PersonaThemeDto {
    PersonaThemeDto {
        persona_id: theme.persona_id.to_string(),
        wallpaper_asset_id: theme.wallpaper_asset_id.as_ref().map(|a| a.to_string()),
        updated_at_ms: theme.updated_at.timestamp_millis(),
    }
}

/// Converts an `AssetCard` projection to an `AssetCardDto`. Search
/// hit augmentation (`score` / `snippet`) is layered on separately
/// by [`card_to_dto_with_hit`] on the search read path.
pub fn card_to_dto(card: &AssetCard) -> AssetCardDto {
    AssetCardDto {
        id: card.id.to_string(),
        persona_id: card.persona_id.to_string(),
        modality: card.modality.as_ref().map(|m| m.as_str().to_string()),
        occurred_at_ms: card.occurred_at.timestamp_millis(),
        cover: card.cover.as_ref().map(|c| c.as_str().to_string()),
        labels: card.labels.iter().map(|l| l.as_str().to_string()).collect(),
        file_size_bytes: card.file_size_bytes,
        duration_ms: card.duration_ms,
        pixel_count: card.pixel_count,
        mime: card.mime.clone(),
        // Answered here rather than left for the UI to derive from
        // `mime`. It goes through `render_policy` — not
        // `MimeType::media` — because the role is part of the answer:
        // a container owns no bytes and gets no player, whatever mime
        // happens to be attached to it.
        media: render_policy(
            card.mime.as_deref().map(MimeType::parse).as_ref(),
            card.role,
            false,
        )
        .media
        .as_str()
        .to_string(),
        // The display rendering, not the storage form. Every consumer of
        // this field renders it — a basename label on the burst and in
        // the duplicates panel, a tooltip, a clipboard copy — and none
        // round-trips it, so what crosses is the spelling a person
        // recognises rather than the one the column happens to hold.
        // That is what lets the storage encoding change without the wire
        // type changing with it.
        source_locator: card.source_locator.to_display(),
        group_ids: card.group_ids.iter().map(|g| g.to_string()).collect(),
        primary_group_position: card.primary_group_position,
        created_at_ms: card.created_at.timestamp_millis(),
        // The sync cursor. Same millisecond resolution the window field
        // reads, so a caller can hand this straight back as
        // `updated_from_ms` without a rounding step that would either
        // skip a row or replay a page.
        updated_at_ms: card.updated_at.timestamp_millis(),
        rating: card.rating,
        palette: card.palette.clone(),
        has_note: card.has_note,
        has_thread: card.has_thread,
        role: card.role.as_str().to_string(),
        title: card.title.clone(),
        member_count: card.member_count,
        score: None,
        snippet: None,
        // Same `(kind, subject)` split `asset_to_dto` writes — the card
        // and the detail must not disagree about who a row is by.
        author_kind: card.author.as_ref().map(|a| a.kind_slug().to_string()),
        author_subject: card
            .author
            .as_ref()
            .and_then(|a| a.subject())
            .map(str::to_string),
        operator_ai: card.operator_ai.as_ref().map(|o| o.as_str().to_string()),
    }
}

/// Same shape as [`card_to_dto`] but populates the search-only
/// `score` + `snippet` fields from a retrieval `Candidate`.
pub fn card_to_dto_with_hit(card: &AssetCard, score: f32, snippet: Option<String>) -> AssetCardDto {
    AssetCardDto {
        score: Some(score),
        snippet,
        ..card_to_dto(card)
    }
}

/// Converts a `Page<AssetCard>` to an `AssetPageDto`.
pub fn page_to_dto(page: &Page<AssetCard>) -> AssetPageDto {
    AssetPageDto {
        items: page.items.iter().map(card_to_dto).collect(),
        offset: page.offset,
        limit: page.limit,
        total: page.total,
    }
}

/// Converts an `AssetIndex` projection to its wire form.
pub fn index_to_dto(
    idx: &crate::domain::asset::AssetIndex,
) -> asterism_contract::dto::AssetIndexEntryDto {
    asterism_contract::dto::AssetIndexEntryDto {
        id: idx.id.to_string(),
        persona_id: idx.persona_id.to_string(),
        modality: idx.modality.as_ref().map(|m| m.as_str().to_string()),
        occurred_at_ms: idx.occurred_at.timestamp_millis(),
        labels: idx.labels.iter().map(|l| l.as_str().to_string()).collect(),
        group_ids: idx.group_ids.iter().map(|g| g.to_string()).collect(),
        primary_group_position: idx.primary_group_position,
        created_at_ms: idx.created_at.timestamp_millis(),
        // Same cursor value the card path carries — the two projections
        // answer the same sync question, and a client that pages on
        // index rows must not have to hydrate to find its cursor.
        updated_at_ms: idx.updated_at.timestamp_millis(),
        // The three metric axes' keys, carried verbatim. `None` travels as
        // `None`: a stand-in `0` would put a still image at the head of
        // longest-first, which is the failure the absent state exists to
        // avoid (`sort_eval::absent_last_desc`).
        duration_ms: idx.duration_ms,
        file_size_bytes: idx.file_size_bytes,
        pixel_count: idx.pixel_count,
        role: idx.role.as_str().to_string(),
    }
}

/// Converts a `Page<AssetIndex>` to an `AssetIndexPageDto`.
pub fn index_page_to_dto(
    page: &Page<crate::domain::asset::AssetIndex>,
) -> asterism_contract::dto::AssetIndexPageDto {
    asterism_contract::dto::AssetIndexPageDto {
        items: page.items.iter().map(index_to_dto).collect(),
        offset: page.offset,
        limit: page.limit,
        total: page.total,
    }
}

/// Converts a `Session` entity to its wire DTO. Every field maps
/// through unchanged (Session is already stored in unix-ms form on
/// the entity — see `domain::session::Session`).
pub fn session_to_dto(s: &Session) -> SessionDto {
    SessionDto {
        id: s.id.as_str().to_string(),
        persona_id: s.persona_id.to_string(),
        external_key: s.external_key.as_str().to_string(),
        title: s.metadata.title.clone(),
        note: s.metadata.note.clone(),
        cover_hint: s.metadata.cover_hint.clone(),
        started_at_ms: s.started_at_ms,
        ended_at_ms: s.ended_at_ms,
        message_count: s.message_count,
        created_at_ms: s.created_at_ms,
        updated_at_ms: s.updated_at_ms,
    }
}

/// Converts a `Page<Session>` (the shape now returned by
/// [`AssetRepository::list_sessions`](crate::domain::repository::AssetRepository::list_sessions))
/// to its wire DTO.
pub fn session_page_to_dto(page: &Page<Session>) -> SessionPageDto {
    SessionPageDto {
        items: page.items.iter().map(session_to_dto).collect(),
        offset: page.offset,
        limit: page.limit,
        total: page.total,
    }
}

/// Converts an `Asset` entity to an `AssetDto` (every field, used for the detail view).
pub fn asset_to_dto(asset: &Asset) -> AssetDto {
    let (visibility_restricted, visibility_sharing) = match &asset.visibility {
        Visibility::Open => (false, Vec::new()),
        Visibility::Restricted { sharing } => (true, sharing.clone()),
    };
    AssetDto {
        id: asset.id.to_string(),
        persona_id: asset.persona_id.to_string(),
        source_kind: asset.source.kind.as_str().to_string(),
        // Display rendering, for the same reason `card_to_dto` sends
        // one: the detail pane shows this to a person and nothing reads
        // it back, so it must not be coupled to whatever the column
        // holds.
        locator: asset.source.locator.to_display(),
        file_size_bytes: asset.source.file_size_bytes,
        platform: asset.source.platform.clone(),
        // Primary material's format fact (`None` when the entity was
        // not hydrated — batch paths — or the fact is unknown).
        // The wire form is the stored token, the rule `role` and
        // `author_kind` already follow: a closed set crosses the
        // boundary as the string it is persisted as.
        mime: asset
            .materials
            .first()
            .and_then(|m| m.mime.as_ref())
            .map(|m| m.as_str().to_string()),
        // Same projection as the card's, from the parsed form the
        // entity already holds.
        media: render_policy(
            asset.materials.first().and_then(|m| m.mime.as_ref()),
            asset.role,
            false,
        )
        .media
        .as_str()
        .to_string(),
        // Primary material's fingerprint, stored value and all: a
        // marker (`unhashable:no-bytes`) is a different answer from the
        // absence the same field shows for "not hashed yet" / "not
        // hydrated", so neither is translated into the other here.
        content_hash: asset.materials.first().and_then(|m| m.content_hash.clone()),
        modality: asset.modality.as_ref().map(|m| m.as_str().to_string()),
        labels: asset
            .labels
            .iter()
            .map(|l| l.as_str().to_string())
            .collect(),
        occurred_at_ms: asset.occurred_at.timestamp_millis(),
        // session-model v2: composition membership + composite title
        // replace the old `session_id` field on the wire.
        container_id: asset.container_id.as_ref().map(|c| c.to_string()),
        title: asset.title.clone(),
        bundle_id: asset.bundle_id.as_ref().map(|b| b.as_str().to_string()),
        role: asset.role.as_str().to_string(),
        cover: asset.cover.as_ref().map(|c| c.as_str().to_string()),
        keywords: asset
            .keywords
            .iter()
            .map(|k| k.as_str().to_string())
            .collect(),
        register_note: asset.register_note.as_ref().map(|r| r.as_str().to_string()),
        visibility_restricted,
        visibility_sharing,
        duration_ms: asset.duration_ms,
        // Projected straight through, halves included. The pair-or-nothing
        // rule is a write-side assertion (`AssetService::add`), so a row
        // that arrived half-filled through some other writer reads back as
        // it stands — repairing it here would hide the write that did it,
        // and refusing to project it would make an existing row
        // unreadable.
        width_px: asset.width_px,
        height_px: asset.height_px,
        rating: asset.rating,
        palette: asset.palette.clone(),
        extra_json: match &asset.extra {
            serde_json::Value::Null => None,
            other => Some(other.to_string()),
        },
        created_at_ms: asset.created_at.timestamp_millis(),
        updated_at_ms: asset.updated_at.timestamp_millis(),
        // Attribution projects as the same `(kind, subject)` pair the
        // column carries; absent means unrecorded, never the owner.
        author_kind: asset.author().map(|a| a.kind_slug().to_string()),
        author_subject: asset.author().and_then(|a| a.subject()).map(str::to_string),
        operator_ai: asset.operator_ai().map(|o| o.as_str().to_string()),
        // The channel projects outward only: a reader may see how an
        // attribution arrived, and no command lets one be sent back in.
        attributed_via: asset.attributed_via().map(|c| c.slug().to_string()),
        // The declaration, not a resolved value: absent stays absent
        // rather than projecting the default the detector would apply,
        // which is the same rule the attribution pair above follows.
        on_duplicate: asset.on_duplicate.map(|d| d.as_str().to_string()),
        // The fold axis, both halves. A read by id is allowed to reach
        // a headstone — that is what makes an old reference resolvable —
        // so the record it hands back has to be able to say so.
        // `fold_policy` projects as its slug and is never absent: every
        // row starts at `auto`, and "nobody has ruled" is an answer.
        folded_into: asset.folded_into.map(|k| k.to_string()),
        fold_policy: asset.fold_policy.as_str().to_string(),
    }
}

/// Converts a `ModalityView` (master row + live asset count) to its
/// wire DTO. `kind` / `cover_template` project through their slug
/// forms.
pub fn modality_view_to_dto(view: &ModalityView) -> ModalityDefDto {
    ModalityDefDto {
        slug: view.def.slug.as_str().to_string(),
        label: view.def.label.clone(),
        terminal: view.def.terminal,
        sort_order: view.def.sort_order,
        hidden: view.def.hidden,
        cover_template: view.def.cover_template.map(|t| t.as_str().to_string()),
        asset_count: view.asset_count,
    }
}

/// Converts a registered series Strategy to its wire DTO.
///
/// `applies_to` and `decode` project as the tokens they are stored and
/// registered as — one spelling for both directions the value travels,
/// which is what keeps a rule from being registered as one decoder and
/// applied as another. The path lists keep their nesting for the same
/// class of reason: flattened, `["vdsl","script"]` reads as one path
/// naming a keyword nothing carries.
pub fn registered_strategy_to_dto(registered: &RegisteredStrategy) -> SeriesStrategyDto {
    let strategy = &registered.strategy;
    SeriesStrategyDto {
        id: strategy.id.to_string(),
        name: strategy.name.clone(),
        applies_to: strategy.applies_to.as_str().to_string(),
        decode: strategy.decode.as_str().to_string(),
        include: paths_to_wire(&strategy.include),
        exclude: paths_to_wire(&strategy.exclude),
        system: registered.system,
        created_at_ms: registered.created_at.timestamp_millis(),
        updated_at_ms: registered.updated_at.timestamp_millis(),
    }
}

/// The wire form of a path list — segments out, nesting kept.
fn paths_to_wire(paths: &[SeriesPath]) -> Vec<Vec<String>> {
    paths.iter().map(|path| path.segments().to_vec()).collect()
}

/// Converts a resolved setting to its wire DTO, projecting the whole
/// layer chain and folding in the registry metadata (`kind` / `min` /
/// `max` / `env_var` / `summary`) a settings UI needs to render the
/// control without a second lookup.
pub fn effective_setting_to_dto(setting: &EffectiveSetting) -> SettingDto {
    let def = setting.key.def();
    SettingDto {
        key: def.key.to_string(),
        kind: def.kind.as_str().to_string(),
        value_json: setting.value_json.clone(),
        source: setting.source.as_str().to_string(),
        layers: setting
            .layers
            .iter()
            .map(|layer| SettingLayerDto {
                source: layer.source.as_str().to_string(),
                value_json: layer.value_json.clone(),
                origin: layer.origin.map(|o| o.to_string()),
                rejected: layer.rejected.clone(),
            })
            .collect(),
        env_var: def.env_var.map(|name| name.to_string()),
        min: def.range.map(|(min, _)| min),
        max: def.range.map(|(_, max)| max),
        summary: def.summary.to_string(),
    }
}

/// Converts a `Tag` to a `TagDto`.
pub fn tag_to_dto(tag: &Tag) -> TagDto {
    TagDto {
        id: tag.id.to_string(),
        name: tag.name.clone(),
        axis: tag.axis.map(|a| a.as_str().to_string()),
    }
}

/// Converts a `TagCount` to a `TagCountDto`.
pub fn tag_count_to_dto(tc: &TagCount) -> TagCountDto {
    TagCountDto {
        tag: tag_to_dto(&tc.tag),
        asset_count: tc.asset_count,
    }
}

/// Converts a `Group` to its wire DTO.
pub fn group_to_dto(g: &Group) -> GroupDto {
    GroupDto {
        id: g.id.to_string(),
        persona_id: g.persona_id.to_string(),
        name: g.name.clone(),
        description: g.description.clone(),
        dir_id: g.dir_id.map(|d| d.to_string()),
        kind: g.kind.as_str().to_string(),
        query_json: g.query_json.clone(),
        origin_snapshot_id: g.origin_snapshot_id.map(|s| s.to_string()),
        last_refresh_at_ms: g.last_refresh_at.map(|t| t.timestamp_millis()),
        last_refresh_status: g.last_refresh_status.clone(),
        last_refresh_error: g.last_refresh_error.clone(),
        created_at_ms: g.created_at.timestamp_millis(),
        updated_at_ms: g.updated_at.timestamp_millis(),
    }
}

/// Converts a `Dir` to its wire DTO.
pub fn dir_to_dto(d: &Dir) -> DirDto {
    DirDto {
        id: d.id.to_string(),
        persona_id: d.persona_id.to_string(),
        parent_id: d.parent_id.map(|p| p.to_string()),
        name: d.name.clone(),
        position: d.position,
        created_at_ms: d.created_at.timestamp_millis(),
        updated_at_ms: d.updated_at.timestamp_millis(),
    }
}

/// Converts a `GroupLink` to its wire DTO.
pub fn group_link_to_dto(l: &GroupLink) -> GroupLinkDto {
    GroupLinkDto {
        parent_group_id: l.parent_id.to_string(),
        child_group_id: l.child_id.to_string(),
        position: l.position,
    }
}

/// Converts a `GroupSummary` to its wire DTO.
pub fn group_summary_to_dto(gs: &GroupSummary) -> GroupSummaryDto {
    GroupSummaryDto {
        group: group_to_dto(&gs.group),
        asset_count: gs.asset_count,
    }
}

/// Converts a `ConstellationEdge` to an `EdgeDto`.
pub fn edge_to_dto(edge: &ConstellationEdge) -> EdgeDto {
    EdgeDto {
        id: edge.id.to_string(),
        from_asset_id: edge.from.to_string(),
        to_asset_id: edge.to.to_string(),
        kind: edge.kind.as_str().to_string(),
        label: edge.label.clone(),
        weight: edge.weight.map(|w| w as f64),
    }
}

/// Bundles an asset with its tags and edges as an `AssetDetailDto`.
pub fn detail_to_dto(asset: &Asset, tags: &[Tag], edges: &[ConstellationEdge]) -> AssetDetailDto {
    AssetDetailDto {
        asset: asset_to_dto(asset),
        tags: tags.iter().map(tag_to_dto).collect(),
        edges: edges.iter().map(edge_to_dto).collect(),
    }
}

/// Parses the wire representation of an asset id.
pub fn parse_asset_id(value: &str) -> Result<AssetId, DomainError> {
    Ok(AssetId::from_uuid(parse_uuid(value, "asset_id")?))
}

/// Parses the wire representation of a persona id.
pub fn parse_persona_id(value: &str) -> Result<PersonaId, DomainError> {
    Ok(PersonaId::from_uuid(parse_uuid(value, "persona_id")?))
}

/// Parses the wire representation of a group id.
pub fn parse_group_id(value: &str) -> Result<GroupId, DomainError> {
    Ok(GroupId::from_uuid(parse_uuid(value, "group_id")?))
}

/// Parses the wire representation of a dir id.
pub fn parse_dir_id(value: &str) -> Result<DirId, DomainError> {
    Ok(DirId::from_uuid(parse_uuid(value, "dir_id")?))
}

/// Parses the wire representation of a tag id.
pub fn parse_tag_id(value: &str) -> Result<TagId, DomainError> {
    Ok(TagId::from_uuid(parse_uuid(value, "tag_id")?))
}

/// Parses the wire representation of a snapshot id.
pub fn parse_snapshot_id(value: &str) -> Result<SnapshotId, DomainError> {
    Ok(SnapshotId::from_uuid(parse_uuid(value, "snapshot_id")?))
}

/// Parses the wire representation of a dispatch id.
pub fn parse_dispatch_id(value: &str) -> Result<DispatchId, DomainError> {
    Ok(DispatchId::from_uuid(parse_uuid(value, "dispatch_id")?))
}

/// Parses the wire representation of a pursuit id.
///
/// One of the two forge parsers still living in the catalogue's mapping
/// module; they belong beside the forge's own wire types (#81).
pub fn parse_pursuit_id(
    value: &str,
) -> Result<crate::domain::forge::value::PursuitId, DomainError> {
    Ok(crate::domain::forge::value::PursuitId::from_uuid(
        parse_uuid(value, "pursuit_id")?,
    ))
}

/// Parses the wire representation of a project id.
pub fn parse_project_id(
    value: &str,
) -> Result<crate::domain::forge::value::ProjectId, DomainError> {
    Ok(crate::domain::forge::value::ProjectId::from_uuid(
        parse_uuid(value, "project_id")?,
    ))
}

/// Parses the wire representation of an asset-comment id.
pub fn parse_asset_comment_id(value: &str) -> Result<AssetCommentId, DomainError> {
    Ok(AssetCommentId::from_uuid(parse_uuid(value, "comment_id")?))
}

/// Parses the wire representation of a material-mark id.
pub fn parse_material_mark_id(value: &str) -> Result<MaterialMarkId, DomainError> {
    Ok(MaterialMarkId::from_uuid(parse_uuid(value, "mark_id")?))
}

/// Parses the wire representation of a material-layer id.
pub fn parse_material_layer_id(value: &str) -> Result<MaterialLayerId, DomainError> {
    Ok(MaterialLayerId::from_uuid(parse_uuid(value, "layer_id")?))
}

/// Parses the wire representation of a chapter-mark id.
pub fn parse_chapter_mark_id(value: &str) -> Result<ChapterMarkId, DomainError> {
    Ok(ChapterMarkId::from_uuid(parse_uuid(value, "chapter_id")?))
}

/// Parses the wire spelling of a layer role.
///
/// A slug this build has no variant for is a **caller** error here, where
/// the same slug read out of a row is an infrastructure one — which is
/// why [`LayerRole::from_slug`] returns `Option` and each of its two
/// callers says so in its own words rather than one restating the other.
pub fn parse_layer_role(slug: &str) -> Result<LayerRole, DomainError> {
    LayerRole::from_slug(slug).ok_or_else(|| {
        DomainError::Validation(format!(
            "unknown layer role: {slug:?} (expected \"structure\" or \"annotation\")"
        ))
    })
}

/// Lifts a wire `(start_ms, end_ms)` pair onto the domain's timeline.
///
/// The wire carries `i64` because that is what the storage columns and
/// every other timestamp on the contract are; the axis starts at the
/// presentation origin and does not run backwards from it, so a negative
/// value is a caller error rather than a position before the start.
///
/// Emptiness, inversion and the storable range are
/// [`TimelineSpan::new`](crate::domain::material_mark::TimelineSpan::new)'s
/// to refuse and are not restated here. Written once for both callers
/// that place something on that axis — a mark's temporal anchor and a
/// chapter's section — so the two cannot disagree about what a wire
/// millisecond means.
pub fn parse_timeline_span(
    start_ms: i64,
    end_ms: Option<i64>,
) -> Result<crate::domain::material_mark::TimelineSpan, DomainError> {
    let start = parse_timeline_ms(start_ms, "start_ms")?;
    let end = end_ms
        .map(|value| parse_timeline_ms(value, "end_ms"))
        .transpose()?;
    crate::domain::material_mark::TimelineSpan::new(start, end)
}

/// One end of a span, lifted onto the domain's unsigned axis.
fn parse_timeline_ms(value: i64, field: &str) -> Result<u64, DomainError> {
    u64::try_from(value).map_err(|_| {
        DomainError::Validation(format!(
            "{field} = {value} is before the start of the timeline"
        ))
    })
}

/// Parses the wire representation of a thread id.
pub fn parse_thread_id(value: &str) -> Result<ThreadId, DomainError> {
    Ok(ThreadId::from_uuid(parse_uuid(value, "thread_id")?))
}

/// Parses the wire representation of a message id.
pub fn parse_message_id(value: &str) -> Result<MessageId, DomainError> {
    Ok(MessageId::from_uuid(parse_uuid(value, "message_id")?))
}

/// Parses a `(anchor_kind, anchor_id)` wire pair into a
/// [`ThreadAnchor`]. `anchor_id` is required for every kind except
/// `"app_global"`.
pub fn parse_thread_anchor(kind: &str, id: Option<&str>) -> Result<ThreadAnchor, DomainError> {
    match kind {
        "app_global" => {
            if id.is_some() {
                return Err(DomainError::Validation(
                    "anchor_id must be None for app_global".into(),
                ));
            }
            Ok(ThreadAnchor::AppGlobal)
        }
        "snapshot" => {
            let id = id.ok_or_else(|| {
                DomainError::Validation("anchor_id required for kind = snapshot".into())
            })?;
            Ok(ThreadAnchor::Snapshot(parse_snapshot_id(id)?))
        }
        "query_group" => {
            let id = id.ok_or_else(|| {
                DomainError::Validation("anchor_id required for kind = query_group".into())
            })?;
            Ok(ThreadAnchor::QueryGroup(parse_group_id(id)?))
        }
        "card" => {
            let id = id.ok_or_else(|| {
                DomainError::Validation("anchor_id required for kind = card".into())
            })?;
            Ok(ThreadAnchor::Card(parse_asset_id(id)?))
        }
        other => Err(DomainError::Validation(format!(
            "unknown anchor kind: {other:?}"
        ))),
    }
}

/// Parses one wire `MessageRefDto` chip into an [`EntityRef`].
pub fn parse_message_ref(chip: &MessageRefDto) -> Result<EntityRef, DomainError> {
    match chip.kind.as_str() {
        "card" => Ok(EntityRef::Card(parse_asset_id(&chip.id)?)),
        "snapshot" => Ok(EntityRef::Snapshot(parse_snapshot_id(&chip.id)?)),
        "query_group" => Ok(EntityRef::QueryGroup(parse_group_id(&chip.id)?)),
        other => Err(DomainError::Validation(format!(
            "unknown ref kind: {other:?}"
        ))),
    }
}

/// Converts a domain [`ThreadAnchor`] to its wire DTO.
pub fn thread_anchor_to_dto(anchor: &ThreadAnchor) -> ThreadAnchorDto {
    ThreadAnchorDto {
        kind: anchor.kind_slug().to_string(),
        id: anchor.anchor_id(),
    }
}

/// Converts a domain [`EntityRef`] to its wire DTO.
pub fn entity_ref_to_dto(reference: &EntityRef) -> MessageRefDto {
    MessageRefDto {
        kind: reference.kind_slug().to_string(),
        id: reference.ref_id(),
    }
}

/// Converts a domain [`Thread`] to its wire DTO.
pub fn thread_to_dto(thread: &Thread) -> ThreadDto {
    ThreadDto {
        id: thread.id.to_string(),
        title: thread.title.clone(),
        anchor: thread_anchor_to_dto(&thread.anchor),
        created_at_ms: thread.created_at.timestamp_millis(),
        updated_at_ms: thread.updated_at.timestamp_millis(),
        last_message_at_ms: thread.last_message_at.map(|t| t.timestamp_millis()),
        message_count: thread.message_count,
        archived: thread.archived,
    }
}

/// Converts a domain [`Message`] to its wire DTO.
pub fn message_to_dto(message: &Message) -> MessageDto {
    MessageDto {
        id: message.id.to_string(),
        thread_id: message.thread_id.to_string(),
        author_kind: message.author.kind_slug().to_string(),
        author_name: message.author.agent_name().map(str::to_string),
        author_persona_id: message.author.persona_id().map(|p| p.to_string()),
        role: message.role.slug().to_string(),
        body: message.body.clone(),
        refs: message.refs.iter().map(entity_ref_to_dto).collect(),
        created_at_ms: message.created_at.timestamp_millis(),
    }
}

/// Converts an `AssetComment` domain entity to its wire DTO.
pub fn asset_comment_to_dto(comment: &AssetComment) -> AssetCommentDto {
    AssetCommentDto {
        id: comment.id.to_string(),
        asset_id: comment.asset_id.to_string(),
        author_kind: comment.author.kind_slug().to_string(),
        author_persona_id: comment.author.persona_id().map(|p| p.to_string()),
        body: comment.body.clone(),
        created_at_ms: comment.created_at.timestamp_millis(),
        edited_at_ms: comment.edited_at.as_ref().map(|t| t.timestamp_millis()),
        gesture: comment.gesture.map(|g| g.slug().to_string()),
    }
}

/// Narrows a domain millisecond value to the signed representation the
/// wire carries.
///
/// [`TimelineSpan::new`](crate::domain::material_mark::TimelineSpan::new)
/// refuses anything past `i64::MAX`, so the clamp is unreachable from a
/// constructed anchor. It is written as a clamp rather than `as` because
/// the two disagree on what to do with a value that arrived some other
/// way: `as` wraps it to a negative instant somewhere before the origin,
/// which every consumer would read as a position; this pins it to the
/// far end of the axis, where it is at least on the right side of zero.
fn wire_ms(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Converts a `MaterialMark` domain entity to its wire DTO.
///
/// The anchor flattens into `anchor_kind` plus the column group that
/// kind uses. The slug comes from the domain
/// ([`MaterialAnchor::kind_slug`]) rather than from this match, so the
/// wire and the `anchor_kind` column cannot end up with two spellings of
/// one kind; the match is here for the coordinates alone, and a second
/// variant makes it a compile error rather than a DTO with the wrong
/// columns filled in.
pub fn material_mark_to_dto(mark: &MaterialMark) -> MaterialMarkDto {
    let (start_ms, end_ms) = match &mark.anchor {
        MaterialAnchor::Temporal(span) => {
            (Some(wire_ms(span.start_ms())), span.end_ms().map(wire_ms))
        }
    };
    MaterialMarkDto {
        id: mark.id.to_string(),
        asset_id: mark.asset_id.to_string(),
        anchor_kind: mark.anchor.kind_slug().to_string(),
        start_ms,
        end_ms,
        author_kind: mark.author.kind_slug().to_string(),
        author_persona_id: mark.author.persona_id().map(|p| p.to_string()),
        body: mark.body.clone(),
        created_at_ms: mark.created_at.timestamp_millis(),
        edited_at_ms: mark.edited_at.as_ref().map(|t| t.timestamp_millis()),
    }
}

/// Converts a `MaterialLayer` domain entity to its wire DTO.
///
/// `origin` and `role` take their spelling from the domain
/// ([`LayerOrigin::slug`](crate::domain::material_layer::LayerOrigin::slug)
/// / [`LayerRole::slug`]) rather than from a match here, for the reason
/// [`material_mark_to_dto`] gives about anchor kinds: the wire, the
/// column and the schema's `CHECK` are then unable to end up with two
/// spellings of one value.
pub fn material_layer_to_dto(layer: &MaterialLayer) -> MaterialLayerDto {
    MaterialLayerDto {
        id: layer.id.to_string(),
        asset_id: layer.asset_id.to_string(),
        material_ord: layer.material_ord,
        origin: layer.origin.slug().to_string(),
        role: layer.role.slug().to_string(),
        is_default: layer.is_default,
        ord: layer.ord,
    }
}

/// Converts a `ChapterMark` domain entity to its wire DTO.
///
/// The span is not flattened behind an `anchor_kind` the way a mark's
/// anchor is: a chapter carries a [`TimelineSpan`](crate::domain::material_mark::TimelineSpan)
/// by type rather than one variant of a coordinate-space enum, so
/// `start_ms` is always there and there is no kind to name.
pub fn chapter_mark_to_dto(chapter: &ChapterMark) -> ChapterMarkDto {
    ChapterMarkDto {
        id: chapter.id.to_string(),
        layer_id: chapter.layer_id.to_string(),
        start_ms: wire_ms(chapter.span.start_ms()),
        end_ms: chapter.span.end_ms().map(wire_ms),
        label: chapter.label.clone(),
        ord: chapter.ord,
    }
}

/// Converts a `Snapshot` domain entity to its wire DTO (contract
/// cleanup wave: the legacy `SelectionDto` shape is gone; every
/// surface speaks `SnapshotDto`).
pub fn snapshot_to_dto(s: &Snapshot) -> SnapshotDto {
    SnapshotDto {
        id: s.id.to_string(),
        persona_id: s.persona_id.to_string(),
        content_hash: s.content_hash.clone(),
        asset_ids: s.asset_ids.iter().map(|a| a.to_string()).collect(),
        created_at_ms: s.created_at.timestamp_millis(),
    }
}

/// Converts a `DispatchJob` domain entity to its wire DTO.
///
/// The `state_message` and `progress_*` fields expand out of the
/// enum-carried payload so wire consumers do not have to pattern-match
/// on a tagged variant string.
pub fn dispatch_to_dto(job: &DispatchJob) -> DispatchDto {
    let (state_message, progress_current, progress_total) = match &job.state {
        DispatchState::Pending | DispatchState::Done => (None, None, None),
        DispatchState::Running {
            current,
            total,
            message,
        } => (message.clone(), *current, *total),
        DispatchState::Failed { message } => (Some(message.clone()), None, None),
        DispatchState::Cancelled { reason } => (reason.clone(), None, None),
    };
    DispatchDto {
        id: job.id.to_string(),
        snapshot_id: job.snapshot_id.to_string(),
        persona_id: job.persona_id.to_string(),
        pursuit_id: job.pursuit_id.map(|p| p.to_string()),
        exporter_slug: job.exporter_slug.clone(),
        action: job.action.clone(),
        params_json: match &job.params {
            serde_json::Value::Null => "{}".into(),
            other => other.to_string(),
        },
        // A pending job has no handle, and a JSON `null` payload is a
        // handle that says nothing — both are "the backend has not
        // answered yet", and neither deserves to arrive as the string
        // `"null"` for a reader to special-case.
        handle_json: match &job.handle {
            None | Some(serde_json::Value::Null) => None,
            Some(handle) => Some(handle.to_string()),
        },
        // Same reading of absence as the handle above, for the same
        // reason: a record that says nothing is not a record.
        attempt_json: match &job.attempt {
            None | Some(serde_json::Value::Null) => None,
            Some(attempt) => Some(attempt.to_string()),
        },
        state: job.state.slug().to_string(),
        state_message,
        progress_current,
        progress_total,
        output_asset_ids: job.output_asset_ids.iter().map(|a| a.to_string()).collect(),
        created_at_ms: job.created_at.timestamp_millis(),
        updated_at_ms: job.updated_at.timestamp_millis(),
        completed_at_ms: job.completed_at.map(|t| t.timestamp_millis()),
        source_group_id: job.source_group_id.map(|g| g.to_string()),
        source_query_json: job.source_query_json.clone(),
        operator_ai: job.operator_ai().map(|o| o.as_str().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::attribution::AttributionContext;
    use crate::domain::content_hash::{UNHASHABLE, of_bytes};
    use crate::domain::material::Material;
    use crate::domain::value::{SourceKind, SourceRef};
    use asterism_contract::query::ListAssetsQuery;

    /// An item asset holding one primary material whose hash column is
    /// in the given state.
    fn asset_with_stored_hash(stored: Option<&str>) -> Asset {
        let source =
            SourceRef::new(SourceKind::new("fs").expect("kind"), "/pics/a.png").expect("source");
        let locator = source.locator.clone();
        let mut asset = Asset::new(
            PersonaId::new(),
            source,
            None,
            Utc::now(),
            &AttributionContext::unrecorded(),
        );
        let mut material = Material::primary(locator, None, Utc::now());
        material.content_hash = stored.map(str::to_string);
        asset.attach_material(material).expect("an item takes one");
        asset
    }

    /// The detail payload carries the primary material's fingerprint,
    /// and carries each of the column's three states as its own answer.
    ///
    /// The marker row is the one with teeth. `unhashable:no-bytes` says
    /// "there are no bytes to read" — a permanent fact about a record
    /// inside a container or a locator off this disk. Translating it to
    /// absence on the way out would tell every consumer "nobody has
    /// looked yet" instead, and leave them waiting on a hash that is
    /// never coming.
    #[test]
    fn detail_payload_carries_each_hash_state_as_its_own_answer() {
        let digest = of_bytes(b"the same photograph, byte for byte\n");
        for stored in [Some(digest.as_str()), Some(UNHASHABLE), None] {
            let dto = asset_to_dto(&asset_with_stored_hash(stored));
            assert_eq!(
                dto.content_hash.as_deref(),
                stored,
                "stored {stored:?} must reach the wire as itself"
            );
        }

        // The fourth shape is not a material state at all: a payload
        // built from an entity that carries no material (a collection,
        // or an un-hydrated row) has nothing to report, which is the
        // second meaning of the absent case.
        let source = SourceRef::new(SourceKind::new("fs").expect("kind"), "/pics/a.png").unwrap();
        let bare = Asset::new(
            PersonaId::new(),
            source,
            None,
            Utc::now(),
            &AttributionContext::unrecorded(),
        );
        assert_eq!(asset_to_dto(&bare).content_hash, None);
    }

    /// The default wire query must land on the live side. This is the
    /// last line of defence for "a client that never heard of the trash
    /// cannot show trashed assets".
    #[test]
    fn wire_query_without_a_trash_selector_reads_live_rows() {
        let domain = to_asset_query(&ListAssetsQuery::default()).unwrap();
        assert_eq!(domain.trash, TrashFilter::LiveOnly);
    }

    #[test]
    fn trash_selector_maps_each_side_and_rejects_typos() {
        for (wire, expected) in [
            ("live", TrashFilter::LiveOnly),
            ("trashed", TrashFilter::TrashedOnly),
            ("any", TrashFilter::Any),
        ] {
            let query = ListAssetsQuery {
                trash: Some(wire.into()),
                ..ListAssetsQuery::default()
            };
            assert_eq!(to_asset_query(&query).unwrap().trash, expected, "{wire}");
        }

        // A typo must not silently pick a side: defaulting to live would
        // hide the trash view, defaulting to any would leak it.
        let typo = ListAssetsQuery {
            trash: Some("trash".into()),
            ..ListAssetsQuery::default()
        };
        let err = to_asset_query(&typo).unwrap_err();
        assert!(
            matches!(err, DomainError::Validation(ref m) if m.contains("trash")),
            "expected a validation error naming the field, got {err:?}"
        );
    }

    /// Both ends of the accepted range, and both bounds, reach the domain
    /// query untouched. `0` is inside the range even though the write
    /// side stores no zero (it clears the rating instead): a caller
    /// asking for `0..=5` is asking for every rated asset, which is a
    /// meaningful band.
    #[test]
    fn rating_band_passes_through_within_range() {
        for (min, max) in [
            (Some(0), None),
            (Some(0), Some(5)),
            (Some(3), None),
            (None, Some(2)),
        ] {
            let query = ListAssetsQuery {
                rating_min: min,
                rating_max: max,
                ..ListAssetsQuery::default()
            };
            let domain = to_asset_query(&query).expect("in-range band");
            assert_eq!(domain.rating_min, min);
            assert_eq!(domain.rating_max, max);
        }
    }

    /// Above the five-star ceiling is rejected rather than clamped: a
    /// clamp would answer `rating_min=7` with the five-star assets and
    /// look like the filter worked.
    #[test]
    fn rating_bound_above_five_is_rejected() {
        for (min, max, field) in [(Some(6), None, "rating_min"), (None, Some(9), "rating_max")] {
            let query = ListAssetsQuery {
                rating_min: min,
                rating_max: max,
                ..ListAssetsQuery::default()
            };
            let err = to_asset_query(&query).unwrap_err();
            assert!(
                matches!(err, DomainError::Validation(ref m) if m.contains(field)),
                "expected a validation error naming {field}, got {err:?}"
            );
        }
    }

    /// An inverted band is a fact about the request, not about the
    /// corpus — so it is an error rather than an empty page.
    #[test]
    fn inverted_rating_band_is_rejected() {
        let query = ListAssetsQuery {
            rating_min: Some(4),
            rating_max: Some(2),
            ..ListAssetsQuery::default()
        };
        let err = to_asset_query(&query).unwrap_err();
        assert!(
            matches!(err, DomainError::Validation(ref m) if m.contains("rating_min")),
            "expected a validation error naming the inverted band, got {err:?}"
        );

        // Equal bounds are the single-star band, not an inversion.
        let exact = ListAssetsQuery {
            rating_min: Some(3),
            rating_max: Some(3),
            ..ListAssetsQuery::default()
        };
        let domain = to_asset_query(&exact).expect("min == max is a one-value band");
        assert_eq!((domain.rating_min, domain.rating_max), (Some(3), Some(3)));
    }

    /// Each of the four new bounds has to land on its own domain field.
    /// The fixture gives every one a distinct instant, so a cross-wired
    /// pair (`created_until` fed from `updated_until_ms`, the shape a
    /// copy-paste produces) fails instead of type-checking.
    #[test]
    fn ingest_and_modification_windows_map_to_their_own_fields() {
        let query = ListAssetsQuery {
            created_from_ms: Some(1_000),
            created_until_ms: Some(2_000),
            updated_from_ms: Some(3_000),
            updated_until_ms: Some(4_000),
            ..ListAssetsQuery::default()
        };
        let domain = to_asset_query(&query).expect("in-range instants");
        assert_eq!(domain.created_from, Some(parse_ms(1_000, "t").unwrap()));
        assert_eq!(domain.created_until, Some(parse_ms(2_000, "t").unwrap()));
        assert_eq!(domain.updated_from, Some(parse_ms(3_000, "t").unwrap()));
        assert_eq!(domain.updated_until, Some(parse_ms(4_000, "t").unwrap()));

        // Absent stays absent — the no-window state, and the only one in
        // which a caller sees the whole library.
        let none = to_asset_query(&ListAssetsQuery::default()).unwrap();
        assert_eq!(none.created_from, None);
        assert_eq!(none.created_until, None);
        assert_eq!(none.updated_from, None);
        assert_eq!(none.updated_until, None);
    }

    /// An epoch outside the representable range is a `400` naming the
    /// field. Dropping the bound instead would answer a narrow window
    /// with the whole library and look like a working filter.
    #[test]
    fn out_of_range_window_bound_is_rejected_by_field() {
        for field in [
            "created_from_ms",
            "created_until_ms",
            "updated_from_ms",
            "updated_until_ms",
        ] {
            let mut query = ListAssetsQuery::default();
            match field {
                "created_from_ms" => query.created_from_ms = Some(i64::MAX),
                "created_until_ms" => query.created_until_ms = Some(i64::MAX),
                "updated_from_ms" => query.updated_from_ms = Some(i64::MAX),
                _ => query.updated_until_ms = Some(i64::MAX),
            }
            let err = to_asset_query(&query).unwrap_err();
            assert!(
                matches!(err, DomainError::Validation(ref m) if m.contains(field)),
                "expected a validation error naming {field}, got {err:?}"
            );
        }
    }

    /// An inverted window is *not* rejected — it reaches the repository
    /// and returns an empty page. Pinned because the neighbouring rating
    /// band does the opposite, and the divergence is a decision (symmetry
    /// with the never-validated `occurred_*` pair) rather than an
    /// oversight for a later reader to "fix".
    #[test]
    fn inverted_time_window_is_accepted_and_left_to_the_query() {
        let query = ListAssetsQuery {
            created_from_ms: Some(9_000),
            created_until_ms: Some(1_000),
            updated_from_ms: Some(9_000),
            updated_until_ms: Some(1_000),
            ..ListAssetsQuery::default()
        };
        let domain = to_asset_query(&query).expect("inverted window is not a validation error");
        assert!(domain.created_from > domain.created_until);
        assert!(domain.updated_from > domain.updated_until);
    }

    /// `rating_max = 0` can never match: stored ratings are `1..=5`
    /// (a zero on the write side clears the rating). Like the inverted
    /// band, an always-empty band is answered with an error, not an
    /// empty page. The lower bound keeps `0` (= every rated asset).
    #[test]
    fn rating_max_zero_is_rejected() {
        let query = ListAssetsQuery {
            rating_max: Some(0),
            ..ListAssetsQuery::default()
        };
        let err = to_asset_query(&query).unwrap_err();
        assert!(
            matches!(err, DomainError::Validation(ref m) if m.contains("rating_max 0")),
            "expected a validation error naming rating_max 0, got {err:?}"
        );
    }

    /// Each of the six metric bounds has to land on its own domain
    /// field.
    ///
    /// This is the only thing holding those six lines of
    /// [`to_asset_query`] in place: nothing destructures `AssetQuery`
    /// exhaustively, so a bound dropped in the mapper — or crossed with
    /// its neighbour — compiles clean and answers every length / size /
    /// resolution question with the unfiltered set. The six values are
    /// distinct and ordered so that any crossing (min↔max within an axis,
    /// or across the three axes) lands on a value this test names.
    #[test]
    fn metric_bands_reach_the_domain_query() {
        let query = ListAssetsQuery {
            duration_min_ms: Some(1_000),
            duration_max_ms: Some(2_000),
            size_min_bytes: Some(3_000),
            size_max_bytes: Some(4_000),
            pixels_min: Some(5_000),
            pixels_max: Some(6_000),
            ..ListAssetsQuery::default()
        };
        let domain = to_asset_query(&query).expect("an ascending band is valid");
        assert_eq!(domain.duration_min_ms, Some(1_000));
        assert_eq!(domain.duration_max_ms, Some(2_000));
        assert_eq!(domain.size_min_bytes, Some(3_000));
        assert_eq!(domain.size_max_bytes, Some(4_000));
        assert_eq!(domain.pixels_min, Some(5_000));
        assert_eq!(domain.pixels_max, Some(6_000));

        // Absent stays absent — the no-band state, and the only one in
        // which stills, unprobed containers and rows nobody measured stay
        // in the result set.
        let none = to_asset_query(&ListAssetsQuery::default()).unwrap();
        assert_eq!(none.duration_min_ms, None);
        assert_eq!(none.duration_max_ms, None);
        assert_eq!(none.size_min_bytes, None);
        assert_eq!(none.size_max_bytes, None);
        assert_eq!(none.pixels_min, None);
        assert_eq!(none.pixels_max, None);

        // One end alone is a band too: "at least a minute" needs no
        // ceiling, and the NULL exclusion fires on either end.
        let open_ended = ListAssetsQuery {
            duration_min_ms: Some(60_000),
            size_max_bytes: Some(1_048_576),
            pixels_min: Some(2_000_000),
            ..ListAssetsQuery::default()
        };
        let domain = to_asset_query(&open_ended).expect("a half-open band is valid");
        assert_eq!(
            (
                domain.duration_min_ms,
                domain.duration_max_ms,
                domain.size_min_bytes,
                domain.size_max_bytes,
                domain.pixels_min,
                domain.pixels_max
            ),
            (
                Some(60_000),
                None,
                None,
                Some(1_048_576),
                Some(2_000_000),
                None
            )
        );
    }

    /// Inverted metric bands are rejected, matching the rating band
    /// rather than the time windows: an empty page for `duration_min_ms
    /// > duration_max_ms` reads as "nothing in the library is that long".
    ///
    /// All three axes are exercised because they are three separate calls
    /// in the mapper — validating one and forgetting the next is the
    /// shape a copy-paste produces.
    #[test]
    fn inverted_metric_band_is_rejected() {
        for (query, field) in [
            (
                ListAssetsQuery {
                    duration_min_ms: Some(120_000),
                    duration_max_ms: Some(1_000),
                    ..ListAssetsQuery::default()
                },
                "duration_min_ms",
            ),
            (
                ListAssetsQuery {
                    size_min_bytes: Some(1_048_576),
                    size_max_bytes: Some(1_024),
                    ..ListAssetsQuery::default()
                },
                "size_min_bytes",
            ),
            (
                ListAssetsQuery {
                    pixels_min: Some(12_000_000),
                    pixels_max: Some(2_000_000),
                    ..ListAssetsQuery::default()
                },
                "pixels_min",
            ),
        ] {
            let err = to_asset_query(&query).unwrap_err();
            assert!(
                matches!(err, DomainError::Validation(ref m) if m.contains(field)),
                "expected a validation error naming {field}, got {err:?}"
            );
        }

        // Equal ends are a one-value band, not an inversion — the exact
        // shape a "files of exactly this size" question takes.
        let exact = ListAssetsQuery {
            duration_min_ms: Some(5_000),
            duration_max_ms: Some(5_000),
            size_min_bytes: Some(2_048),
            size_max_bytes: Some(2_048),
            pixels_min: Some(8_294_400),
            pixels_max: Some(8_294_400),
            ..ListAssetsQuery::default()
        };
        let domain = to_asset_query(&exact).expect("min == max is a one-value band");
        assert_eq!(domain.duration_min_ms, domain.duration_max_ms);
        assert_eq!(domain.size_min_bytes, domain.size_max_bytes);
        assert_eq!(domain.pixels_min, domain.pixels_max);
    }

    /// No ceiling check accompanies the inversion one: unlike the star
    /// axis, length, size and resolution have no domain maximum, so a
    /// bound past anything in the library is a legitimate question that
    /// returns an empty page rather than a `400`.
    #[test]
    fn metric_bounds_have_no_upper_limit() {
        let query = ListAssetsQuery {
            duration_min_ms: Some(u64::MAX),
            size_min_bytes: Some(u64::MAX),
            pixels_min: Some(u64::MAX),
            ..ListAssetsQuery::default()
        };
        let domain = to_asset_query(&query).expect("the axes have no ceiling to exceed");
        assert_eq!(domain.duration_min_ms, Some(u64::MAX));
        assert_eq!(domain.size_min_bytes, Some(u64::MAX));
        assert_eq!(domain.pixels_min, Some(u64::MAX));
    }

    /// A dispatch with a handle on it.
    fn dispatch_with_handle(handle: Option<serde_json::Value>) -> DispatchJob {
        let mut job = DispatchJob::new(
            SnapshotId::new(),
            PersonaId::new(),
            "http",
            "render",
            serde_json::json!({ "endpoint": "http://backend.test" }),
            Utc::now(),
            &AttributionContext::unrecorded(),
        )
        .expect("a slug and an action is all the constructor asks for");
        job.handle = handle;
        job
    }

    /// The recorded exchange arrives on the wire shape, so a reader
    /// asking what a dispatch sent and what came back of it does not
    /// have to open the database to find out.
    #[test]
    fn dispatch_dto_carries_the_handle_payload() {
        let payload = serde_json::json!({
            "handle": "job-1",
            "exchange": {
                "request": { "method": "POST", "body": { "prompt": "a plate" } },
                "response": { "job_id": "job-1" }
            }
        });
        let dto = dispatch_to_dto(&dispatch_with_handle(Some(payload.clone())));
        let carried: serde_json::Value =
            serde_json::from_str(&dto.handle_json.expect("an issued handle reaches the wire"))
                .expect("what the column holds is JSON, and stays JSON");
        assert_eq!(carried, payload);
    }

    /// Two ways of having no handle — no payload at all, and a payload
    /// that is the JSON value `null` — and they answer alike, because
    /// both say the backend has not been heard from. Neither arrives as
    /// the string `"null"` for every reader to special-case.
    #[test]
    fn a_dispatch_without_a_handle_carries_none() {
        assert_eq!(
            dispatch_to_dto(&dispatch_with_handle(None)).handle_json,
            None
        );
        assert_eq!(
            dispatch_to_dto(&dispatch_with_handle(Some(serde_json::Value::Null))).handle_json,
            None
        );
    }
}
